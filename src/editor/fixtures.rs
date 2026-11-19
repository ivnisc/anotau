//! Servidor local para tests: la `SQLite` y las reglas reales de `anotau-server`, sin HTTP.

use anotau_protocol::{self as api, Command, EntryKind, Id, Outcome, Resolution};
use anotau_server::{database, store};
use ratatui::crossterm::event::KeyCode;
use rusqlite::{Connection, params};

use super::Editor;
use crate::{
    network::{Failure, Job, Origin},
    workspace::Tree,
};

pub(crate) const WELCOME: &str = include_str!("../../assets/templates/bienvenida.md");

pub(crate) struct Server {
    _directory: tempfile::TempDir,
    connection: Connection,
    pub(crate) owner: Id,
    operations: u64,
}

impl Server {
    /// Base vacía con una cuenta registrada: Personal y Bienvenida.
    pub(crate) fn new() -> Self {
        let directory = tempfile::tempdir().expect("directorio temporal");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
                .expect("permisos privados");
        }
        let mut connection =
            database::open(&directory.path().join("anotau.sqlite3")).expect("SQLite migrada");
        let owner = store::register(&mut connection, "yo", "hash").expect("cuenta");
        Self {
            _directory: directory,
            connection,
            owner,
            operations: 0,
        }
    }

    pub(crate) fn account(&mut self, name: &str) -> Id {
        store::register(&mut self.connection, name, "hash").expect("cuenta")
    }

    pub(crate) fn run_as(&mut self, owner: Id, command: Command) -> Outcome {
        self.operations += 1;
        let request = api::Mutation {
            operation_id: format!("{:064x}", self.operations),
            command,
        };
        store::mutate(&mut self.connection, owner, &request)
            .expect("operación aceptada")
            .outcome
    }

    pub(crate) fn state_of(&mut self, owner: Id) -> api::Snapshot {
        store::snapshot(&mut self.connection, owner).expect("estado")
    }

    pub(crate) fn state(&mut self) -> api::Snapshot {
        self.state_of(self.owner)
    }

    /// Sugerencias guardadas: cuenta (ninguna si es anónima) y texto.
    pub(crate) fn suggestions(&self) -> Vec<(Option<Id>, String)> {
        self.connection
            .prepare("SELECT account_id, body FROM suggestions ORDER BY id")
            .and_then(|mut statement| {
                statement
                    .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                    .collect()
            })
            .expect("sugerencias")
    }

    pub(crate) fn markdown(&self, id: Id) -> String {
        store::note(&self.connection, self.owner, id)
            .expect("nota")
            .markdown
    }

    pub(crate) fn entry(&mut self, title: &str) -> api::Entry {
        self.state()
            .entries
            .into_iter()
            .find(|entry| entry.title == title)
            .expect("entrada")
    }

    /// Crea una entrada en un espacio de `owner`; las notas reciben `markdown`.
    pub(crate) fn create(
        &mut self,
        owner: Id,
        workspace: Id,
        parent: Option<Id>,
        title: &str,
        markdown: Option<&str>,
    ) -> Id {
        let kind = if markdown.is_some() {
            EntryKind::Note
        } else {
            EntryKind::Folder
        };
        let Outcome::Changed {
            entry: Some(id), ..
        } = self.run_as(
            owner,
            Command::CreateEntry {
                workspace,
                parent,
                title: title.into(),
                kind,
            },
        )
        else {
            panic!("entrada creada")
        };
        if let Some(markdown) = markdown.filter(|markdown| !markdown.is_empty()) {
            self.run_as(
                owner,
                Command::Save {
                    id,
                    revision: 1,
                    markdown: markdown.into(),
                },
            );
        }
        id
    }

    /// Presencia real: una sesión con la última interacción hace `idle` segundos.
    pub(crate) fn session(&self, account: Id, idle: i64) {
        self.connection
            .execute(
                "INSERT INTO sessions(token_hash, account_id, created_at, last_activity_at)
                 VALUES (randomblob(32), ?1, unixepoch() - ?2, unixepoch() - ?2)",
                params![account, idle],
            )
            .expect("sesión");
    }

    /// Contacto aceptado entre la cuenta propia y `other`.
    pub(crate) fn befriend(&mut self, other: Id) {
        self.run_as(self.owner, Command::Contact { recipient: other });
        let id = self
            .state_of(other)
            .directory
            .people
            .into_iter()
            .find_map(|person| match person.contact {
                api::Contact::Incoming(id) if person.id == self.owner => Some(id),
                _ => None,
            })
            .expect("solicitud recibida");
        self.run_as(
            other,
            Command::ResolveContact {
                id,
                resolution: Resolution::Accept,
            },
        );
    }

    /// Editor autenticado con el estado actual, con la nota inicial ya cargada.
    pub(crate) fn editor(&mut self) -> Editor {
        let mut editor = Editor::from_remote(self.state()).expect("estado válido");
        self.sync(&mut editor);
        editor
    }

    /// Atiende las peticiones del editor contra la base, como el hilo de red.
    pub(crate) fn sync(&mut self, editor: &mut Editor) {
        while let Some(job) = editor.next_job() {
            match job {
                Job::Mutate(request) => {
                    match store::mutate(&mut self.connection, self.owner, &request) {
                        Ok(change) => editor.changed(change),
                        Err(error) => editor.failed(Origin::Mutate, failure(&error)),
                    }
                }
                Job::Note(id) => match store::note(&self.connection, self.owner, id) {
                    Ok(note) => editor.received_note(note),
                    Err(error) => editor.failed(Origin::Note(id), failure(&error)),
                },
                Job::Health
                | Job::Recover(_)
                | Job::Authenticate { .. }
                | Job::Sync { .. }
                | Job::Logout
                | Job::DeleteAccount { .. }
                | Job::ChangePassword { .. }
                | Job::Export(_) => {
                    panic!("el editor solo escribe y lee notas")
                }
            }
        }
    }

    /// Árbol del espacio Personal tal como lo construye el cliente.
    pub(crate) fn tree(&mut self) -> Tree {
        let mut state = self.state();
        let space = state.workspaces.remove(0);
        let entries = state
            .entries
            .into_iter()
            .filter(|entry| entry.workspace == space.id)
            .collect();
        Tree::from_remote(space, entries).expect("árbol válido")
    }
}

/// Texto de la pantalla de prueba, celda por celda.
pub(crate) fn screen(terminal: &ratatui::Terminal<ratatui::backend::TestBackend>) -> String {
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect()
}

fn failure(error: &anotau_server::api::Failure) -> Failure {
    Failure::from_status(error.status(), error.message().into())
}

/// Cuenta con dos espacios, carpetas anidadas, directorio y una copia recibida.
///
/// Personal contiene Bienvenida, la carpeta Universidad (Apuntes y la subcarpeta Proyecto
/// con Ideas para Anotau) y Lecturas (Por leer). Matemática contiene Álgebra con Vectores.
/// Universidad y Álgebra quedan desplegadas.
pub(crate) fn demo() -> (Server, Editor) {
    let mut server = Server::new();
    let owner = server.owner;
    let personal = server.state().workspaces[0].id;
    let university = server.create(owner, personal, None, "Universidad", None);
    let notes = "# Apuntes\n\n## Esta semana\n\n- [ ] Revisar el proyecto";
    let apuntes = server.create(owner, personal, Some(university), "Apuntes", Some(notes));
    let project = server.create(owner, personal, Some(university), "Proyecto", None);
    let ideas = "# Ideas para Anotau\n\n- Probar la navegación entre notas.";
    server.create(
        owner,
        personal,
        Some(project),
        "Ideas para Anotau",
        Some(ideas),
    );
    let reading = server.create(owner, personal, None, "Lecturas", None);
    let pending = "# Por leer\n\n- [ ] Guía de Ratatui";
    server.create(owner, personal, Some(reading), "Por leer", Some(pending));
    let Outcome::Changed {
        workspace: Some(math),
        ..
    } = server.run_as(
        owner,
        Command::CreateWorkspace {
            title: "Matemática".into(),
        },
    )
    else {
        panic!("espacio creado")
    };
    let algebra = server.create(owner, math, None, "Álgebra", None);
    let dot_product = "# Vectores\n\n- Repasar el producto escalar.";
    let vectors = server.create(owner, math, Some(algebra), "Vectores", Some(dot_product));

    let mut people = Vec::new();
    for name in [
        "Zoe", "Carla", "Ana", "Diego", "Bruno", "Elena", "Javier", "María",
    ] {
        people.push(server.account(name));
    }
    let [_, carla, ana, diego, ..] = people[..] else {
        unreachable!("directorio de ejemplo")
    };
    server.session(carla, 0);
    server.session(ana, 0);
    server.session(diego, 400);
    server.befriend(carla);
    server.run_as(owner, Command::Contact { recipient: diego });
    let shared = server.state_of(carla).workspaces[0].id;
    let offered = server.create(
        carla,
        shared,
        None,
        "Apuntes compartidos.md",
        Some("Compartido"),
    );
    let revision = server
        .state_of(carla)
        .entries
        .into_iter()
        .find(|entry| entry.id == offered)
        .expect("nota ofrecida")
        .revision;
    server.run_as(
        carla,
        Command::Offer {
            id: offered,
            revision,
            recipient: owner,
        },
    );

    let mut editor = server.editor();
    let space = &mut editor.workspaces[0].tree;
    space.reveal_server(apuntes);
    space.navigate(KeyCode::Home);
    space.navigate(KeyCode::Down);
    let space = &mut editor.workspaces[1].tree;
    space.reveal_server(vectors);
    space.navigate(KeyCode::Home);
    (server, editor)
}
