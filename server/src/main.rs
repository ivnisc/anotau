mod config;

use std::{
    net::{SocketAddr, TcpListener},
    sync::Arc,
    time::Duration,
};

use anotau_protocol::{API_VERSION, Health, ServerStatus};
use anotau_server::{api, backup, database, files, limits::ConnectionLimits};
use anyhow::Context;
use axum::{Json, Router, http::StatusCode, routing::get};
use axum_server::{
    Handle,
    tls_rustls::{RustlsAcceptor, RustlsConfig},
};
use config::{Config, TlsFiles};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};

fn load_tls(files: TlsFiles) -> anyhow::Result<RustlsConfig> {
    files::require_private_directory(
        files
            .key
            .parent()
            .context("La clave TLS necesita un directorio padre")?,
    )?;
    files::require_private_file(&files.key)?;
    let chain = CertificateDer::pem_file_iter(files.certificate)
        .context("No se pudo abrir el certificado TLS")?
        .collect::<Result<Vec<_>, _>>()
        .context("Certificado TLS inválido")?;
    let key = PrivateKeyDer::from_pem_file(files.key).context("Clave TLS inválida")?;
    let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .with_no_client_auth()
    .with_single_cert(chain, key)?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(RustlsConfig::from_config(Arc::new(config)))
}

fn routes(name: String, status: ServerStatus) -> Router {
    let health = Health {
        service: "anotau".into(),
        api_version: API_VERSION,
        schema_version: database::SCHEMA_VERSION,
        name,
        status,
    };
    let router = Router::new().route(
        "/v1/health",
        get(move || {
            let health = health.clone();
            async move { Json(health) }
        }),
    );
    if status == ServerStatus::Maintenance {
        router.fallback(|| async {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(anotau_protocol::Error {
                    message: "El servidor está en mantención. Vuelve a intentarlo más tarde."
                        .into(),
                }),
            )
        })
    } else {
        router
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let args = std::env::args_os()
        .skip(1)
        .map(|arg| arg.into_string().ok())
        .collect::<Option<Vec<_>>>()
        .unwrap_or_default();
    let task = match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
        [] => Task::Serve,
        ["--help"] => {
            println!(
                "Uso: anotau-server [--backup | tickets | reset-password <usuario>]\n\
                 ANOTAU_DATABASE: ruta absoluta de SQLite\n\
                 ANOTAU_LISTEN: dirección de escucha (127.0.0.1:8443 por defecto)\n\
                 ANOTAU_TLS_CERT y ANOTAU_TLS_KEY: certificado y clave PEM; obligatorios fuera \
                 de loopback.\nANOTAU_BACKUP_DIR: directorio privado de respaldos diarios; \
                 ANOTAU_BACKUP_KEEP: cuántos conservar (7 por defecto).\n\
                 --backup: respalda ahora y termina.\n\
                 tickets: lista las solicitudes de recuperación pendientes.\n\
                 reset-password <usuario>: asigna una contraseña temporal, cierra sus sesiones \
                 y resuelve sus solicitudes."
            );
            return Ok(());
        }
        ["--backup"] => Task::Backup,
        ["tickets"] => Task::Tickets,
        ["reset-password", username] => Task::Reset(username.to_owned()),
        _ => anyhow::bail!(
            "Uso: anotau-server [--help | --backup | tickets | reset-password <usuario>]"
        ),
    };
    let config = Config::from_env()?;
    match task {
        Task::Serve => {}
        Task::Tickets => return tickets(&config.database),
        Task::Reset(username) => return reset_password(&config.database, &username),
        Task::Backup => {
            let (directory, keep) = config
                .backup
                .context("Configura ANOTAU_BACKUP_DIR para respaldar")?;
            let path = backup::run(&config.database, &directory, keep)?;
            println!("Respaldo creado en {}", path.display());
            return Ok(());
        }
    }
    let tls = config.tls.map(load_tls).transpose()?;
    // en mantención SQLite queda libre para trabajar sobre la base, también de los respaldos
    if let (Some(backups), false) = (config.backup.clone(), config.maintenance) {
        tokio::spawn(backup_daily(config.database.clone(), backups));
    }
    let app = if config.maintenance {
        eprintln!("Anotau en mantención: solo responde /v1/health");
        routes(config.name, ServerStatus::Maintenance)
    } else {
        let database = database::open(&config.database)?;
        routes(config.name, ServerStatus::Operational).merge(api::routes(database)?)
    };
    let handle = Handle::new();
    let listener =
        TcpListener::bind(config.listen).context("No se pudo abrir el puerto configurado")?;
    let address = listener.local_addr()?;
    let mut server = axum_server::from_tcp(listener)?
        .handle(handle.clone())
        .http1_only()
        .first_request_timeout(Duration::from_secs(5));
    server
        .http_builder()
        .http1()
        .header_read_timeout(Duration::from_secs(5))
        .max_headers(32)
        .max_buf_size(16 * 1024)
        .keep_alive(false);
    tokio::spawn(shutdown(handle));
    // la vida máxima cubre handshake, cabeceras y petición (5 + 5 + 15 s) con margen
    let limits = ConnectionLimits::new(256, 16, Duration::from_secs(30));
    if let Some(tls) = tls {
        eprintln!("Anotau escucha mediante HTTPS en {address}");
        server
            .acceptor(
                RustlsAcceptor::new(tls)
                    .handshake_timeout(Duration::from_secs(5))
                    .acceptor(limits),
            )
            .serve(app.into_make_service_with_connect_info::<SocketAddr>())
            .await?;
    } else {
        eprintln!("Anotau escucha mediante HTTP local en {address}");
        server
            .acceptor(limits)
            .serve(app.into_make_service_with_connect_info::<SocketAddr>())
            .await?;
    }
    Ok(())
}

/// Qué hace el proceso: servir la API o una tarea de administración que termina.
enum Task {
    Serve,
    Backup,
    Tickets,
    Reset(String),
}

fn tickets(database: &std::path::Path) -> anyhow::Result<()> {
    let connection = database::open(database)?;
    let pending = anotau_server::recovery::pending(&connection)
        .map_err(|error| anyhow::anyhow!(error.message()))?;
    if pending.is_empty() {
        println!("No hay solicitudes de recuperación pendientes.");
    }
    for ticket in pending {
        println!(
            "#{}  {} UTC  {}  contacto: {}",
            ticket.id, ticket.created_at, ticket.username, ticket.contact
        );
    }
    Ok(())
}

/// Contraseña temporal para una cuenta, tras verificar la identidad por fuera de Anotau.
fn reset_password(database: &std::path::Path, username: &str) -> anyhow::Result<()> {
    let mut connection = database::open(database)?;
    let password = anotau_server::recovery::temporary_password()
        .and_then(|password| Ok((api::password_hash(&password)?, password)))
        .map_err(|error| anyhow::anyhow!(error.message()));
    let (hash, password) = password?;
    let expires =
        anotau_server::recovery::reset(&mut connection, username, &hash).map_err(|error| {
            if error.status() == 404 {
                anyhow::anyhow!("No existe la cuenta «{username}».")
            } else {
                anyhow::anyhow!(error.message())
            }
        })?;
    println!(
        "Contraseña temporal de {username}: {password}\n\
         Vence el {expires} UTC si no se cambia antes en F3 → Cambiar contraseña.\n\
         Se cerraron sus sesiones y se resolvieron sus solicitudes. Entrégala por el contacto \
         verificado junto con ese plazo."
    );
    Ok(())
}

/// Respaldo al arrancar y luego cada 24 horas; un fallo se informa y se reintenta al día siguiente.
async fn backup_daily(
    database: std::path::PathBuf,
    (directory, keep): (std::path::PathBuf, usize),
) {
    let mut daily = tokio::time::interval(Duration::from_secs(24 * 60 * 60));
    loop {
        daily.tick().await;
        let (database, directory) = (database.clone(), directory.clone());
        match tokio::task::spawn_blocking(move || backup::run(&database, &directory, keep)).await {
            Ok(Ok(path)) => eprintln!("Respaldo creado en {}", path.display()),
            Ok(Err(error)) => eprintln!("Falló el respaldo: {error:#}"),
            Err(error) => eprintln!("Falló la tarea de respaldo: {error}"),
        }
    }
}

async fn shutdown(handle: Handle<SocketAddr>) {
    #[cfg(unix)]
    {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut terminate) => {
                tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
            }
            Err(error) => eprintln!("No se pudo registrar SIGTERM: {error}"),
        }
    }
    #[cfg(not(unix))]
    let _ = tokio::signal::ctrl_c().await;
    handle.graceful_shutdown(Some(Duration::from_secs(5)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;

    #[tokio::test]
    async fn health_reports_protocol_without_exposing_paths_or_creating_account_routes() {
        let app = routes("anotau-vm".into(), ServerStatus::Operational);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        assert_eq!(
            std::str::from_utf8(&body).unwrap(),
            format!(
                "{{\"service\":\"anotau\",\"api_version\":{API_VERSION},\"schema_version\":{},\
                 \"name\":\"anotau-vm\",\"status\":\"operational\"}}",
                database::SCHEMA_VERSION
            )
        );
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/accounts")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn maintenance_answers_health_and_refuses_everything_else() {
        let app = routes("anotau-vm".into(), ServerStatus::Maintenance);
        let request = |method: &str, uri: &str| {
            Request::builder()
                .method(method)
                .uri(uri)
                .body(Body::empty())
                .unwrap()
        };
        let response = app
            .clone()
            .oneshot(request("GET", "/v1/health"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 4096).await.unwrap();
        assert!(
            std::str::from_utf8(&body)
                .unwrap()
                .ends_with("\"status\":\"maintenance\"}")
        );
        for (method, uri) in [("POST", "/v1/sessions"), ("GET", "/v1/state")] {
            let response = app.clone().oneshot(request(method, uri)).await.unwrap();
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        }
    }
}
