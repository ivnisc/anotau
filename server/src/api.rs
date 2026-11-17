use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anotau_protocol::{Activity, Credentials, Id, Mutation, Session};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use axum::{
    Json, Router,
    extract::{ConnectInfo, DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;

use crate::store;

pub type Result<T> = std::result::Result<T, Failure>;

#[derive(Debug)]
pub struct Failure {
    status: StatusCode,
    message: &'static str,
    source: Option<anyhow::Error>,
}

impl Failure {
    /// Código HTTP con el que la API responde este fallo.
    #[must_use]
    pub fn status(&self) -> u16 {
        self.status.as_u16()
    }

    /// Mensaje visible para el usuario, sin detalles internos.
    #[must_use]
    pub fn message(&self) -> &'static str {
        self.message
    }

    pub(crate) fn invalid(message: &'static str) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message,
            source: None,
        }
    }
    pub(crate) fn conflict(message: &'static str) -> Self {
        Self {
            status: StatusCode::CONFLICT,
            message,
            source: None,
        }
    }
    pub(crate) fn missing() -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: "El elemento ya no existe o no pertenece a tu cuenta.",
            source: None,
        }
    }
    pub(crate) fn unauthorized() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: "La sesión venció o las credenciales no son válidas.",
            source: None,
        }
    }
    fn busy() -> Self {
        Self {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: "El servidor está ocupado. Reintenta más tarde.",
            source: None,
        }
    }
    pub(crate) fn internal(error: impl Into<anyhow::Error>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: "No se pudo completar la operación.",
            source: Some(error.into()),
        }
    }
}

impl From<rusqlite::Error> for Failure {
    fn from(error: rusqlite::Error) -> Self {
        if matches!(
            error,
            rusqlite::Error::SqliteFailure(ref cause, _)
                if cause.code == rusqlite::ErrorCode::ConstraintViolation
        ) {
            // las reglas esperables se validan antes; esto queda como respaldo y se registra
            Self {
                source: Some(error.into()),
                ..Self::conflict(
                    "Otro cambio impidió completar la operación. Espera a que se actualice la \
                     vista y vuelve a intentarlo.",
                )
            }
        } else {
            Self::internal(error)
        }
    }
}
impl From<serde_json::Error> for Failure {
    fn from(error: serde_json::Error) -> Self {
        Self::internal(error)
    }
}
impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        if let Some(source) = self.source {
            eprintln!("Error de operación: {source:#}");
        }
        (
            self.status,
            Json(anotau_protocol::Error {
                message: self.message.into(),
            }),
        )
            .into_response()
    }
}

#[derive(Clone)]
struct Backend {
    database: Arc<Mutex<Connection>>,
    capacity: Arc<Semaphore>,
    authentication: Arc<Semaphore>,
    attempts: Arc<Mutex<HashMap<IpAddr, (Instant, u8)>>>,
    dummy_hash: Arc<str>,
}

impl Backend {
    async fn database<T: Send + 'static>(
        &self,
        work: impl FnOnce(&mut Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let permit = Arc::clone(&self.capacity)
            .try_acquire_owned()
            .map_err(|_| Failure::busy())?;
        let database = Arc::clone(&self.database);
        let deadline = Instant::now() + Duration::from_secs(8);
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let mut connection = database
                .lock()
                .map_err(|_| Failure::internal(anyhow::anyhow!("Bloqueo de SQLite dañado")))?;
            if Instant::now() >= deadline {
                return Err(Failure::busy());
            }
            connection.progress_handler(1000, Some(move || Instant::now() >= deadline))?;
            let result = work(&mut connection);
            connection.progress_handler(0, None::<fn() -> bool>)?;
            if !connection.is_autocommit() {
                connection.execute_batch("ROLLBACK")?;
            }
            result
        })
        .await
        .map_err(Failure::internal)?
    }

    fn authentication_allowed(&self, ip: IpAddr) -> Result<()> {
        let mut attempts = self.attempts.lock().map_err(|_| Failure::busy())?;
        let now = Instant::now();
        if attempts.len() >= 2048 {
            attempts
                .retain(|_, (started, _)| now.duration_since(*started) < Duration::from_secs(60));
        }
        if attempts.len() >= 2048 && !attempts.contains_key(&ip) {
            return Err(Failure::busy());
        }
        let (started, count) = attempts.entry(ip).or_insert((now, 0));
        if now.duration_since(*started) >= Duration::from_secs(60) {
            *started = now;
            *count = 0;
        }
        if *count >= 10 {
            return Err(Failure {
                status: StatusCode::TOO_MANY_REQUESTS,
                message: "Demasiados intentos de acceso. Espera un minuto.",
                source: None,
            });
        }
        *count += 1;
        Ok(())
    }
}

fn random() -> Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| {
        Failure::internal(anyhow::anyhow!("No se pudo obtener aleatoriedad: {error}"))
    })?;
    Ok(hex::encode(bytes))
}

/// Verificador Argon2 con sal aleatoria; lo usan el registro, el cambio y el restablecimiento.
pub fn password_hash(password: &str) -> Result<String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|error| Failure::internal(anyhow::anyhow!("No se pudo crear salt: {error}")))?;
    let salt = SaltString::encode_b64(&bytes)
        .map_err(|error| Failure::internal(anyhow::anyhow!("Salt inválido: {error}")))?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|error| Failure::internal(anyhow::anyhow!("No se pudo calcular Argon2: {error}")))
}

fn token(headers: &HeaderMap) -> Result<String> {
    let token = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|header| header.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or_else(Failure::unauthorized)?;
    if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(Failure::unauthorized());
    }
    Ok(token.into())
}

fn authorize(connection: &Connection, token: &str) -> Result<Id> {
    connection
        .query_row(
            "SELECT account_id FROM sessions
             WHERE token_hash = ?1 AND revoked_at IS NULL AND last_activity_at > unixepoch() - 900",
            [Sha256::digest(token.as_bytes()).as_slice()],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(Failure::unauthorized)
}

async fn authenticate(
    backend: Backend,
    ip: IpAddr,
    credentials: Credentials,
    register: bool,
) -> Result<Json<Session>> {
    backend.authentication_allowed(ip)?;
    if credentials.username.trim().is_empty()
        || credentials.username.len() > 256
        || credentials.username.chars().any(char::is_control)
        || credentials.password.is_empty()
        || credentials.password.len() > 1024
    {
        return Err(Failure::invalid(
            "Usuario de 1 a 256 bytes sin controles; contraseña de 1 a 1024 bytes.",
        ));
    }
    if register
        && let Some(problem) = anotau_protocol::username_problem(&credentials.username)
            .or_else(|| anotau_protocol::password_problem(&credentials.password))
    {
        return Err(Failure::invalid(problem));
    }
    let permit = Arc::clone(&backend.authentication)
        .try_acquire_owned()
        .map_err(|_| Failure::busy())?;
    let username = credentials.username;
    let verifier = if register {
        None
    } else {
        let username = username.clone();
        backend
            .database(move |connection| {
                Ok(connection
                    .query_row(
                        "SELECT password_hash FROM accounts WHERE username=?1",
                        [username],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()?)
            })
            .await?
    };
    let dummy = Arc::clone(&backend.dummy_hash);
    let hash = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        if register {
            password_hash(&credentials.password)
        } else {
            let hash = verifier.as_deref().unwrap_or(&dummy);
            let parsed = PasswordHash::new(hash)
                .map_err(|_| Failure::internal(anyhow::anyhow!("Verificador Argon2 inválido")))?;
            if Argon2::default()
                .verify_password(credentials.password.as_bytes(), &parsed)
                .is_err()
                || verifier.is_none()
            {
                return Err(Failure::unauthorized());
            }
            Ok(hash.to_owned())
        }
    })
    .await
    .map_err(Failure::internal)??;
    backend
        .database(move |connection| {
            let account = if register {
                store::register(connection, &username, &hash)?
            } else {
                crate::recovery::signed_in_account(connection, &username, &hash)?
            };
            let token = random()?;
            connection.execute(
                "INSERT INTO sessions(token_hash,account_id) VALUES (?1,?2)",
                params![Sha256::digest(token.as_bytes()).as_slice(), account],
            )?;
            Ok(Json(Session {
                token,
                account_id: account,
                username,
                state: store::snapshot(connection, account)?,
            }))
        })
        .await
}

async fn register(
    State(backend): State<Backend>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(credentials): Json<Credentials>,
) -> Result<Json<Session>> {
    authenticate(backend, peer.ip(), credentials, true).await
}
async fn login(
    State(backend): State<Backend>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(credentials): Json<Credentials>,
) -> Result<Json<Session>> {
    authenticate(backend, peer.ip(), credentials, false).await
}

async fn state(
    State(backend): State<Backend>,
    headers: HeaderMap,
    Query(cursor): Query<anotau_protocol::Cursor>,
) -> Result<Json<anotau_protocol::Snapshot>> {
    let token = token(&headers)?;
    backend
        .database(move |connection| {
            let owner = authorize(connection, &token)?;
            Ok(Json(store::snapshot_at(connection, owner, cursor)?))
        })
        .await
}

async fn note(
    State(backend): State<Backend>,
    headers: HeaderMap,
    Path(id): Path<Id>,
) -> Result<Json<anotau_protocol::Note>> {
    let token = token(&headers)?;
    backend
        .database(move |connection| {
            let owner = authorize(connection, &token)?;
            Ok(Json(store::note(connection, owner, id)?))
        })
        .await
}

async fn mutate(
    State(backend): State<Backend>,
    headers: HeaderMap,
    Json(request): Json<Mutation>,
) -> Result<Json<anotau_protocol::Change>> {
    let token = token(&headers)?;
    backend
        .database(move |connection| {
            let owner = authorize(connection, &token)?;
            Ok(Json(store::mutate(connection, owner, &request)?))
        })
        .await
}

async fn activity(
    State(backend): State<Backend>,
    headers: HeaderMap,
    Json(activity): Json<Activity>,
) -> Result<StatusCode> {
    let token = token(&headers)?;
    backend
        .database(move |connection| {
            authorize(connection, &token)?;
            if activity.idle_seconds >= 900 {
                return Err(Failure::unauthorized());
            }
            let idle = i64::try_from(activity.idle_seconds).map_err(Failure::internal)?;
            connection.execute(
                "UPDATE sessions
                 SET last_activity_at = max(last_activity_at, unixepoch() - ?1),
                     last_seen_at = unixepoch()
                 WHERE token_hash = ?2",
                params![idle, Sha256::digest(token.as_bytes()).as_slice()],
            )?;
            Ok(StatusCode::NO_CONTENT)
        })
        .await
}

/// Solicitud de recuperación sin sesión. Responde 202 exista o no la cuenta y cuenta como
/// intento de acceso, para que no sirva para enumerar nombres ni llenar la tabla.
async fn recovery(
    State(backend): State<Backend>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(request): Json<anotau_protocol::RecoveryRequest>,
) -> Result<StatusCode> {
    backend.authentication_allowed(peer.ip())?;
    if request.username.trim().is_empty()
        || request.username.len() > 256
        || request.username.chars().any(char::is_control)
    {
        return Err(Failure::invalid("Ingresa tu nombre de usuario."));
    }
    if let Some(problem) = anotau_protocol::contact_problem(&request.contact) {
        return Err(Failure::invalid(problem));
    }
    backend
        .database(move |connection| {
            crate::recovery::request(connection, &request.username, &request.contact)?;
            Ok(StatusCode::ACCEPTED)
        })
        .await
}

/// Comprueba la contraseña del titular de una sesión fuera del bucle async.
///
/// Cuenta como intento de acceso. 403 y no 401 si no coincide: la sesión sigue válida y el
/// cliente no debe cerrarla.
async fn verify_owner(backend: &Backend, ip: IpAddr, token: &str, password: String) -> Result<Id> {
    backend.authentication_allowed(ip)?;
    if password.is_empty() || password.len() > 1024 {
        return Err(Failure::invalid("Ingresa tu contraseña para confirmar."));
    }
    let permit = Arc::clone(&backend.authentication)
        .try_acquire_owned()
        .map_err(|_| Failure::busy())?;
    let session = token.to_owned();
    let (owner, hash) = backend
        .database(move |connection| {
            let owner = authorize(connection, &session)?;
            let hash: String = connection.query_row(
                "SELECT password_hash FROM accounts WHERE id = ?1",
                [owner],
                |row| row.get(0),
            )?;
            Ok((owner, hash))
        })
        .await?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let parsed = PasswordHash::new(&hash)
            .map_err(|_| Failure::internal(anyhow::anyhow!("Verificador Argon2 inválido")))?;
        Argon2::default()
            .verify_password(password.as_bytes(), &parsed)
            .map_err(|_| Failure {
                status: StatusCode::FORBIDDEN,
                message: "La contraseña actual no es correcta.",
                source: None,
            })
    })
    .await
    .map_err(Failure::internal)??;
    Ok(owner)
}

/// Cambia la contraseña y cierra todas las sesiones de la cuenta, también la actual.
async fn change_password(
    State(backend): State<Backend>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(change): Json<anotau_protocol::PasswordChange>,
) -> Result<StatusCode> {
    let token = token(&headers)?;
    if let Some(problem) = anotau_protocol::password_problem(&change.new) {
        return Err(Failure::invalid(problem));
    }
    let owner = verify_owner(&backend, peer.ip(), &token, change.current).await?;
    let hash = tokio::task::spawn_blocking(move || password_hash(&change.new))
        .await
        .map_err(Failure::internal)??;
    backend
        .database(move |connection| {
            // la sesión pudo revocarse mientras se verificaba la contraseña
            if authorize(connection, &token)? != owner {
                return Err(Failure::unauthorized());
            }
            let transaction = connection.transaction()?;
            transaction.execute(
                // la contraseña elegida por la persona es definitiva: deja de vencer
                "UPDATE accounts SET password_hash = ?1, password_expires_at = NULL WHERE id = ?2",
                params![hash, owner],
            )?;
            transaction.execute(
                "UPDATE sessions SET revoked_at = unixepoch()
                 WHERE account_id = ?1 AND revoked_at IS NULL",
                [owner],
            )?;
            transaction.commit()?;
            Ok(StatusCode::NO_CONTENT)
        })
        .await
}

/// Borra la cuenta de la sesión con todos sus datos, si la contraseña es la del titular.
///
/// Una sesión abierta no basta: quien encuentre el equipo desbloqueado no puede borrarla.
async fn delete_account(
    State(backend): State<Backend>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(confirmation): Json<anotau_protocol::AccountDeletion>,
) -> Result<StatusCode> {
    let token = token(&headers)?;
    let owner = verify_owner(&backend, peer.ip(), &token, confirmation.password).await?;
    backend
        .database(move |connection| {
            // la sesión pudo revocarse mientras se verificaba la contraseña
            if authorize(connection, &token)? != owner {
                return Err(Failure::unauthorized());
            }
            store::delete_account(connection, owner)?;
            Ok(StatusCode::NO_CONTENT)
        })
        .await
}

async fn logout(State(backend): State<Backend>, headers: HeaderMap) -> Result<StatusCode> {
    let token = token(&headers)?;
    backend
        .database(move |connection| {
            connection.execute(
                "UPDATE sessions SET revoked_at = unixepoch()
                 WHERE token_hash = ?1 AND revoked_at IS NULL",
                [Sha256::digest(token.as_bytes()).as_slice()],
            )?;
            Ok(StatusCode::NO_CONTENT)
        })
        .await
}

pub fn routes(database: Connection) -> anyhow::Result<Router> {
    let backend = Backend {
        database: Arc::new(Mutex::new(database)),
        capacity: Arc::new(Semaphore::new(8)),
        authentication: Arc::new(Semaphore::new(2)),
        attempts: Arc::default(),
        dummy_hash: password_hash("unverifiable-dummy-password")
            .map_err(|_| anyhow::anyhow!("No se pudo inicializar Argon2"))?
            .into(),
    };
    Ok(Router::new()
        .route("/v1/accounts", post(register).delete(delete_account))
        .route("/v1/sessions", post(login).delete(logout))
        .route("/v1/password", post(change_password))
        .route("/v1/recovery", post(recovery))
        .route("/v1/state", get(state))
        .route("/v1/notes/{id}", get(note))
        .route("/v1/actions", post(mutate))
        .route("/v1/activity", post(activity))
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
        .layer(axum::middleware::from_fn(
            |request: axum::extract::Request, next: axum::middleware::Next| async move {
                tokio::time::timeout(Duration::from_secs(15), next.run(request))
                    .await
                    .unwrap_or_else(|_| {
                        (
                            StatusCode::REQUEST_TIMEOUT,
                            "Se agotó el plazo de la petición.",
                        )
                            .into_response()
                    })
            },
        ))
        .with_state(backend))
}
