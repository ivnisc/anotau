use std::{
    env, fs,
    io::Read,
    sync::{
        Arc,
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};

use anotau_protocol::{
    API_VERSION, Activity, Change, Credentials, Health, Id, Mutation, Note, Session, Snapshot,
};
use anyhow::{Context, ensure};
use reqwest::{
    Url,
    blocking::{Client, RequestBuilder},
    redirect::Policy,
};
use serde::de::DeserializeOwned;

/// CA grabada al compilar con `ANOTAU_DEFAULT_CA_CERT` (ver `build.rs`).
const DEFAULT_CA: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/default_ca.pem"));

pub(crate) struct Config {
    pub(crate) server: Url,
    certificate: Option<Vec<u8>>,
}

impl Config {
    pub(crate) fn from_env() -> anyhow::Result<Self> {
        // los binarios de una instancia graban su URL al compilar; la variable la reemplaza
        let value = env::var("ANOTAU_SERVER_URL")
            .ok()
            .or_else(|| option_env!("ANOTAU_DEFAULT_SERVER_URL").map(String::from))
            .context("Configura ANOTAU_SERVER_URL con la URL HTTPS del servidor")?;
        let mut server = Url::parse(&value).context("ANOTAU_SERVER_URL no es una URL válida")?;
        let local = server.host_str().is_some_and(|host| {
            host.parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
        });
        ensure!(
            server.scheme() == "https" || (server.scheme() == "http" && local),
            "El servidor requiere HTTPS; HTTP solo se admite con una IP de loopback"
        );
        ensure!(
            server.host_str().is_some()
                && server.username().is_empty()
                && server.password().is_none()
                && server.query().is_none()
                && server.fragment().is_none(),
            "ANOTAU_SERVER_URL no debe contener credenciales, query ni fragmentos"
        );
        if !server.path().ends_with('/') {
            server.set_path(&format!("{}/", server.path()));
        }
        // la CA del entorno manda; si no hay, la incrustada al compilar (vacía si no se incrustó)
        let certificate = env::var_os("ANOTAU_CA_CERT")
            .map(fs::read)
            .transpose()
            .context("No se pudo leer ANOTAU_CA_CERT")?
            .or_else(|| {
                Some(DEFAULT_CA)
                    .filter(|ca| !ca.is_empty())
                    .map(<[u8]>::to_vec)
            });
        Ok(Self {
            server,
            certificate,
        })
    }
}

impl Config {
    /// Servidor y puerto configurados, para mostrarlos antes de conocer su nombre.
    pub(crate) fn address(&self) -> String {
        let host = self.server.host_str().unwrap_or_default();
        self.server
            .port()
            .map_or_else(|| host.to_owned(), |port| format!("{host}:{port}"))
    }
}

/// Cómo se autentica la persona: con una cuenta existente o registrando una nueva.
#[derive(Clone, Copy)]
pub(crate) enum Account {
    Login,
    Register,
}

impl Account {
    fn path(self) -> &'static str {
        match self {
            Self::Login => "v1/sessions",
            Self::Register => "v1/accounts",
        }
    }
}

pub(crate) enum Job {
    /// Estado del servidor para la bienvenida; no requiere sesión.
    Health,
    /// Solicitud de recuperación de contraseña; no requiere sesión.
    Recover(anotau_protocol::RecoveryRequest),
    Authenticate {
        credentials: Credentials,
        account: Account,
    },
    Mutate(Arc<Mutation>),
    Note(Id),
    Sync {
        last_activity: Instant,
    },
    Logout,
    /// Borra la cuenta de la sesión con todos sus datos; la contraseña confirma al titular.
    DeleteAccount {
        password: String,
    },
    /// Escribe notas como archivos `.md` en una carpeta local.
    Export(ExportRequest),
    /// Cambia la contraseña; la sesión deja de valer al confirmarse.
    ChangePassword {
        current: String,
        new: String,
    },
}

/// Exportación: las notas abiertas llevan su texto; las demás se piden al servidor.
pub(crate) struct ExportRequest {
    pub(crate) directory: std::path::PathBuf,
    /// Título de la carpeta exportada, que se crea dentro de `directory`.
    pub(crate) folder: Option<String>,
    pub(crate) notes: Vec<(Vec<String>, String, ExportContent)>,
}

pub(crate) enum ExportContent {
    Text(String),
    Remote(Id),
}

pub(crate) enum Reply {
    Health(Health),
    RecoveryRequested,
    Authenticated(Session),
    Changed(Change),
    Note(Note),
    State(Snapshot),
    LoggedOut,
    AccountDeleted,
    PasswordChanged,
    Exported {
        path: std::path::PathBuf,
        notes: usize,
    },
}

/// Qué debe hacer el cliente ante un fallo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reason {
    /// La sesión ya no es válida; la operación se conserva hasta volver a autenticar.
    Unauthorized,
    /// No se sabe si el servidor aplicó la operación; se reintenta con la misma identidad.
    Uncertain,
    /// Otra revisión o resolución ganó; se conserva el borrador y no se reintenta solo.
    Conflict,
    /// El servidor rechazó la petición sin aplicarla.
    Rejected,
}

pub(crate) struct Failure {
    pub(crate) message: String,
    pub(crate) reason: Reason,
}

impl Failure {
    pub(crate) fn new(reason: Reason, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            reason,
        }
    }

    /// Clasifica una respuesta HTTP de error del servidor.
    pub(crate) fn from_status(status: u16, message: String) -> Self {
        let reason = match status {
            401 => Reason::Unauthorized,
            409 => Reason::Conflict,
            408 | 500..=599 => Reason::Uncertain,
            _ => Reason::Rejected,
        };
        Self::new(reason, message)
    }
}

pub(crate) struct Response {
    pub(crate) epoch: u64,
    pub(crate) origin: Origin,
    pub(crate) result: Result<Reply, Failure>,
}

/// Petición que produjo una respuesta; una lectura de nota recuerda su nota.
#[derive(Clone, Copy)]
pub(crate) enum Origin {
    Health,
    Recover,
    Authenticate,
    Mutate,
    Note(Id),
    Sync,
    Logout,
    DeleteAccount,
    ChangePassword,
    Export,
}

pub(crate) struct Network {
    sender: SyncSender<(u64, Job)>,
    pub(crate) responses: Receiver<Response>,
}

impl Network {
    pub(crate) fn start(config: Config) -> anyhow::Result<Self> {
        let mut builder = Client::builder()
            .redirect(Policy::none())
            .retry(reqwest::retry::never())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(12))
            .https_only(config.server.scheme() == "https");
        if let Some(certificate) = config.certificate {
            builder = builder.add_root_certificate(
                reqwest::Certificate::from_pem(&certificate)
                    .context("Certificado de la CA inválido (ANOTAU_CA_CERT o el incrustado)")?,
            );
        }
        let client = builder
            .build()
            .context("No se pudo crear el cliente HTTPS")?;
        let (sender, jobs) = mpsc::sync_channel(8);
        let (responses, receiver) = mpsc::sync_channel(8);
        thread::Builder::new()
            .name("anotau-network".into())
            .spawn(move || {
                let mut token = None;
                for (epoch, job) in jobs {
                    let origin = match &job {
                        Job::Health => Origin::Health,
                        Job::Recover(_) => Origin::Recover,
                        Job::Authenticate { .. } => Origin::Authenticate,
                        Job::Mutate(_) => Origin::Mutate,
                        Job::Note(id) => Origin::Note(*id),
                        Job::Sync { .. } => Origin::Sync,
                        Job::Logout => Origin::Logout,
                        Job::DeleteAccount { .. } => Origin::DeleteAccount,
                        Job::ChangePassword { .. } => Origin::ChangePassword,
                        Job::Export(_) => Origin::Export,
                    };
                    let result = execute(&client, &config.server, &mut token, job);
                    if responses
                        .send(Response {
                            epoch,
                            origin,
                            result,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .context("No se pudo iniciar la tarea de red")?;
        Ok(Self {
            sender,
            responses: receiver,
        })
    }

    pub(crate) fn submit(&self, epoch: u64, job: Job) -> Result<(), Job> {
        self.sender
            .try_send((epoch, job))
            .map_err(|error| match error {
                mpsc::TrySendError::Full((_, job)) | mpsc::TrySendError::Disconnected((_, job)) => {
                    job
                }
            })
    }
}

pub(crate) fn operation(command: anotau_protocol::Command) -> anyhow::Result<Arc<Mutation>> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|error| anyhow::anyhow!("No se pudo identificar la operación: {error}"))?;
    Ok(Arc::new(Mutation {
        operation_id: hex::encode(bytes),
        command,
    }))
}

fn send(request: RequestBuilder) -> Result<reqwest::blocking::Response, Failure> {
    let response = request.send().map_err(|error| {
        let message = if error.is_connect() {
            "No se pudo conectar con el servidor. \
             Comprueba la URL, la red o VPN y la confianza del certificado."
                .into()
        } else {
            format!("Falló la comunicación con el servidor: {error}")
        };
        Failure::new(Reason::Uncertain, message)
    })?;
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let mut text = String::new();
    let _ = response.take(4096).read_to_string(&mut text);
    let message = serde_json::from_str::<anotau_protocol::Error>(&text).map_or_else(
        |_| format!("El servidor respondió HTTP {status}"),
        |error| error.message,
    );
    Err(Failure::from_status(status.as_u16(), message))
}

fn json<T: DeserializeOwned>(request: RequestBuilder) -> Result<T, Failure> {
    let mut bytes = Vec::new();
    send(request)?
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            Failure::new(Reason::Uncertain, format!("Respuesta incompleta: {error}"))
        })?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err(Failure::new(
            Reason::Uncertain,
            "La respuesta supera el límite del cliente.",
        ));
    }
    serde_json::from_slice(&bytes).map_err(|_| {
        Failure::new(
            Reason::Uncertain,
            "La respuesta no corresponde al protocolo de Anotau.",
        )
    })
}

fn execute(
    client: &Client,
    base: &Url,
    token: &mut Option<String>,
    job: Job,
) -> Result<Reply, Failure> {
    let url = |path: &str| {
        base.join(path)
            .map_err(|_| Failure::new(Reason::Rejected, "Ruta del servidor inválida."))
    };
    if let Job::Health = job {
        return json(client.get(url("v1/health")?)).map(Reply::Health);
    }
    if let Job::Recover(request) = job {
        send(client.post(url("v1/recovery")?).json(&request))?;
        return Ok(Reply::RecoveryRequested);
    }
    if let Job::Authenticate {
        credentials,
        account,
    } = job
    {
        let health: Health = json(client.get(url("v1/health")?))?;
        if health.api_version != API_VERSION {
            return Err(Failure::new(
                Reason::Rejected,
                format!(
                    "El servidor usa la versión {} de la API y este cliente la {API_VERSION}. \
                     Actualiza Anotau.",
                    health.api_version
                ),
            ));
        }
        let mut session: Session = json(client.post(url(account.path())?).json(&credentials))?;
        *token = Some(std::mem::take(&mut session.token));
        session.state = complete_state(
            client,
            base,
            token.as_deref().unwrap_or_default(),
            session.state,
        )?;
        return Ok(Reply::Authenticated(session));
    }
    let bearer = token
        .as_deref()
        .ok_or_else(|| Failure::new(Reason::Unauthorized, "Inicia sesión para continuar."))?;
    match job {
        Job::Mutate(request) => {
            let mut change: Change = json(
                client
                    .post(url("v1/actions")?)
                    .bearer_auth(bearer)
                    .json(&*request),
            )?;
            if let Some(state) = change.state.take() {
                // la escritura ya se confirmó: completar el estado se reintenta como incierto
                let state = complete_state(client, base, bearer, state).map_err(|error| {
                    if error.reason == Reason::Unauthorized {
                        error
                    } else {
                        Failure::new(Reason::Uncertain, error.message)
                    }
                })?;
                change.state = Some(state);
            }
            Ok(Reply::Changed(change))
        }
        Job::Note(id) => json(
            client
                .get(url(&format!("v1/notes/{id}"))?)
                .bearer_auth(bearer),
        )
        .map(Reply::Note),
        Job::Sync { last_activity } => {
            send(
                client
                    .post(url("v1/activity")?)
                    .bearer_auth(bearer)
                    .json(&Activity {
                        idle_seconds: last_activity.elapsed().as_secs(),
                    }),
            )?;
            let state = json(client.get(url("v1/state")?).bearer_auth(bearer))?;
            complete_state(client, base, bearer, state).map(Reply::State)
        }
        Job::Logout => {
            let response = send(client.delete(url("v1/sessions")?).bearer_auth(bearer));
            *token = None;
            response.map(|_| Reply::LoggedOut)
        }
        Job::DeleteAccount { password } => {
            let request = client
                .delete(url("v1/accounts")?)
                .json(&anotau_protocol::AccountDeletion { password });
            ending_session(request, token, Reply::AccountDeleted)
        }
        Job::ChangePassword { current, new } => {
            let request = client
                .post(url("v1/password")?)
                .json(&anotau_protocol::PasswordChange { current, new });
            ending_session(request, token, Reply::PasswordChanged)
        }
        Job::Export(request) => export(client, &url, bearer, request),
        Job::Health | Job::Recover(_) | Job::Authenticate { .. } => {
            unreachable!("health y autenticación se procesan antes de consultar token")
        }
    }
}

/// Petición de cuenta que, si se acepta, deja sin valor la sesión: borrar la cuenta o cambiar
/// la contraseña, que revoca todas sus sesiones.
fn ending_session(
    request: RequestBuilder,
    token: &mut Option<String>,
    reply: Reply,
) -> Result<Reply, Failure> {
    let bearer = token
        .as_deref()
        .ok_or_else(|| Failure::new(Reason::Unauthorized, "Inicia sesión para continuar."))?;
    send(request.bearer_auth(bearer))?;
    *token = None;
    Ok(reply)
}

/// Lee del servidor las notas que no estaban abiertas y escribe los archivos.
fn export(
    client: &Client,
    url: &dyn Fn(&str) -> Result<Url, Failure>,
    bearer: &str,
    request: ExportRequest,
) -> Result<Reply, Failure> {
    let mut files = Vec::with_capacity(request.notes.len());
    for (folders, title, content) in request.notes {
        let markdown = match content {
            ExportContent::Text(text) => text,
            ExportContent::Remote(id) => {
                json::<Note>(
                    client
                        .get(url(&format!("v1/notes/{id}"))?)
                        .bearer_auth(bearer),
                )?
                .markdown
            }
        };
        files.push(crate::export::File {
            folders,
            title,
            markdown,
        });
    }
    let path = crate::export::write(&request.directory, request.folder.as_deref(), &files)
        .map_err(|error| Failure::new(Reason::Rejected, format!("No se pudo exportar: {error}")))?;
    Ok(Reply::Exported {
        path,
        notes: files.len(),
    })
}

fn complete_state(
    client: &Client,
    base: &Url,
    token: &str,
    mut state: Snapshot,
) -> Result<Snapshot, Failure> {
    let deadline = Instant::now() + Duration::from_secs(15);
    while let Some(cursor) = state.next.take() {
        if Instant::now() >= deadline {
            return Err(Failure::new(
                Reason::Rejected,
                "La lectura paginada excedió su plazo. Reintenta.",
            ));
        }
        let url = base
            .join("v1/state")
            .map_err(|_| Failure::new(Reason::Rejected, "Ruta inválida."))?;
        let mut next: Snapshot = json(
            client
                .get(url)
                .bearer_auth(token)
                .query(&cursor)
                .timeout(deadline.saturating_duration_since(Instant::now())),
        )?;
        if next.workspaces.is_empty()
            && next.entries.is_empty()
            && next.directory.people.is_empty()
            && next.directory.incoming.is_empty()
        {
            return Err(Failure::new(
                Reason::Rejected,
                "El servidor devolvió una página sin avance.",
            ));
        }
        state.workspaces.append(&mut next.workspaces);
        state.entries.append(&mut next.entries);
        state.directory.people.append(&mut next.directory.people);
        state
            .directory
            .incoming
            .append(&mut next.directory.incoming);
        state.next = next.next;
        if state.entries.len()
            + state.workspaces.len()
            + state.directory.people.len()
            + state.directory.incoming.len()
            > 100_000
        {
            return Err(Failure::new(
                Reason::Rejected,
                "El estado supera 100000 elementos en este cliente.",
            ));
        }
    }
    Ok(state)
}
