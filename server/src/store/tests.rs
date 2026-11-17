use anotau_protocol::{Command, Destination, Draft, EntryKind, Id, Mutation, Outcome, Snapshot};
use rusqlite::Connection;

use super::{delete_account, mutate, note, register, snapshot};

struct Account {
    _directory: tempfile::TempDir,
    connection: Connection,
    owner: Id,
    operations: u64,
}

impl Account {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
                .unwrap();
        }
        let mut connection = crate::database::open(&directory.path().join("test.sqlite3")).unwrap();
        let owner = register(&mut connection, "yo", "hash").unwrap();
        Self {
            _directory: directory,
            connection,
            owner,
            operations: 0,
        }
    }

    fn run_as(&mut self, owner: Id, command: Command) -> super::Result<Outcome> {
        self.operations += 1;
        let request = Mutation {
            operation_id: format!("{:064x}", self.operations),
            command,
        };
        mutate(&mut self.connection, owner, &request).map(|change| change.outcome)
    }

    fn run(&mut self, command: Command) -> super::Result<Outcome> {
        self.run_as(self.owner, command)
    }

    fn state(&mut self) -> Snapshot {
        snapshot(&mut self.connection, self.owner).unwrap()
    }

    fn personal(&mut self) -> Id {
        self.state().workspaces[0].id
    }

    fn create(&mut self, parent: Option<Id>, title: &str, kind: EntryKind) -> Id {
        let workspace = self.personal();
        let Outcome::Changed {
            entry: Some(id), ..
        } = self
            .run(Command::CreateEntry {
                workspace,
                parent,
                title: title.into(),
                kind,
            })
            .unwrap()
        else {
            panic!("se esperaba una entrada creada")
        };
        id
    }

    fn title(&mut self, id: Id) -> String {
        let state = self.state();
        state
            .entries
            .into_iter()
            .find(|entry| entry.id == id)
            .unwrap()
            .title
    }

    fn revision(&mut self, id: Id) -> i64 {
        let state = self.state();
        state
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .unwrap()
            .revision
    }

    fn children(&mut self, parent: Id) -> Vec<(Id, String)> {
        self.state()
            .entries
            .into_iter()
            .filter(|entry| entry.parent == Some(parent))
            .map(|entry| (entry.id, entry.title))
            .collect()
    }
}

#[test]
fn registration_creates_personal_space_with_the_welcome_template() {
    let mut account = Account::new();
    let state = account.state();
    assert_eq!(state.workspaces.len(), 1);
    assert_eq!(state.workspaces[0].title, "Personal");
    assert_eq!(state.entries.len(), 1);
    assert_eq!(state.entries[0].title, "Bienvenida");
    let welcome = note(&account.connection, account.owner, state.entries[0].id).unwrap();
    assert_eq!(
        welcome.markdown,
        include_str!("../../../assets/templates/bienvenida.md")
    );
}

#[test]
fn rename_and_move_reject_occupied_names_and_cycles_without_changes() {
    let mut account = Account::new();
    let course = account.create(None, "Curso", EntryKind::Folder);
    let archive = account.create(None, "Archivo", EntryKind::Folder);
    let nested = account.create(Some(course), "Clases", EntryKind::Folder);
    let revision = account.revision(course);
    let rename = account.run(Command::RenameEntry {
        id: course,
        revision,
        title: "Archivo".into(),
    });
    let rename = rename.unwrap_err();
    assert_eq!(rename.status(), 409);
    assert_eq!(
        rename.message(),
        "Ya hay una nota o carpeta con ese nombre en esta ubicación."
    );
    for parent in [course, nested] {
        let moved = account.run(Command::MoveEntry {
            id: course,
            revision,
            parent: Some(parent),
        });
        assert!(
            moved.unwrap_err().message().contains("dentro de sí misma"),
            "un movimiento no puede formar ciclos"
        );
    }
    account.create(Some(archive), "Curso", EntryKind::Note);
    let moved = account.run(Command::MoveEntry {
        id: course,
        revision,
        parent: Some(archive),
    });
    let moved = moved.unwrap_err();
    assert_eq!(moved.status(), 409);
    assert!(moved.message().starts_with("El destino ya tiene"));
    assert_eq!(account.title(course), "Curso");
    assert_eq!(account.revision(course), revision);
}

#[test]
fn copy_creates_independent_subtrees_with_drafts_and_keeps_identity_on_rename() {
    let mut account = Account::new();
    let course = account.create(None, "Curso", EntryKind::Folder);
    let nested = account.create(Some(course), "Clases", EntryKind::Folder);
    let lesson = account.create(Some(nested), "Apunte", EntryKind::Note);
    let revision = account.revision(lesson);
    account
        .run(Command::Save {
            id: lesson,
            revision,
            markdown: "original".into(),
        })
        .unwrap();
    let draft = Draft {
        id: lesson,
        revision: account.revision(lesson),
        markdown: "borrador e\u{301} 👩‍💻".into(),
    };
    let revision = account.revision(course);
    let inside = account.run(Command::CopyEntry {
        id: course,
        revision,
        parent: Some(nested),
        drafts: vec![draft.clone()],
    });
    assert_eq!(inside.unwrap_err().status(), 400);
    let Outcome::Changed {
        entry: Some(copy), ..
    } = account
        .run(Command::CopyEntry {
            id: course,
            revision,
            parent: None,
            drafts: vec![draft],
        })
        .unwrap()
    else {
        panic!("copia creada")
    };
    assert_ne!(copy, course);
    assert_eq!(account.title(copy), "Curso (2)");
    let (copied_nested, _) = account.children(copy)[0].clone();
    let (copied_note, _) = account.children(copied_nested)[0].clone();
    let markdown = |account: &Account, id| note(&account.connection, account.owner, id).unwrap();
    assert_eq!(
        markdown(&account, copied_note).markdown,
        "borrador e\u{301} 👩‍💻"
    );
    assert_eq!(markdown(&account, lesson).markdown, "original");

    let revision = account.revision(course);
    account
        .run(Command::RenameEntry {
            id: course,
            revision,
            title: "Matemática".into(),
        })
        .unwrap();
    assert_eq!(account.title(course), "Matemática");
    assert_eq!(account.children(course)[0].0, nested);
}

#[test]
fn stale_revisions_and_reused_operation_ids_are_conflicts() {
    let mut account = Account::new();
    let id = account.state().entries[0].id;
    let revision = account.revision(id);
    account
        .run(Command::Save {
            id,
            revision,
            markdown: "uno".into(),
        })
        .unwrap();
    let stale = account.run(Command::Save {
        id,
        revision,
        markdown: "dos".into(),
    });
    assert_eq!(stale.unwrap_err().status(), 409);

    let request = Mutation {
        operation_id: "f".repeat(64),
        command: Command::Save {
            id,
            revision: revision + 1,
            markdown: "tres".into(),
        },
    };
    let first = mutate(&mut account.connection, account.owner, &request).unwrap();
    let again = mutate(&mut account.connection, account.owner, &request).unwrap();
    assert!(matches!(
        (first.outcome, again.outcome),
        (Outcome::Saved { revision: a, .. }, Outcome::Saved { revision: b, .. }) if a == b
    ));
    let changed = Mutation {
        operation_id: request.operation_id,
        command: Command::Save {
            id,
            revision: revision + 2,
            markdown: "otro".into(),
        },
    };
    let Err(reused) = mutate(&mut account.connection, account.owner, &changed) else {
        panic!("una identidad reutilizada con otros argumentos es un conflicto")
    };
    assert_eq!(reused.status(), 409);
    assert_eq!(
        note(&account.connection, account.owner, id)
            .unwrap()
            .markdown,
        "tres"
    );
}

#[test]
fn transfers_need_consent_and_accepting_twice_returns_the_same_copy() {
    let mut account = Account::new();
    let other = register(&mut account.connection, "carla", "hash").unwrap();
    let id = account.state().entries[0].id;
    let revision = account.revision(id);
    let offer = Command::Offer {
        id,
        revision,
        recipient: other,
    };
    let refused = account.run(offer.clone()).unwrap_err();
    assert!(
        refused
            .message()
            .contains("aceptar tu solicitud de contacto")
    );
    account.run(Command::Contact { recipient: other }).unwrap();
    let again = account
        .run(Command::Contact { recipient: other })
        .unwrap_err();
    assert_eq!(
        again.message(),
        "Ya hay una solicitud de contacto pendiente con este usuario."
    );
    let contact = snapshot(&mut account.connection, other)
        .unwrap()
        .directory
        .people
        .into_iter()
        .find_map(|person| match person.contact {
            anotau_protocol::Contact::Incoming(id) => Some(id),
            _ => None,
        })
        .unwrap();
    account
        .run_as(
            other,
            Command::ResolveContact {
                id: contact,
                resolution: anotau_protocol::Resolution::Accept,
            },
        )
        .unwrap();
    account.run(offer).unwrap();
    let inbox = snapshot(&mut account.connection, other).unwrap();
    let transfer = inbox.directory.incoming[0].id;
    let destination = Destination {
        workspace: inbox.workspaces[0].id,
        parent: None,
    };
    let accept = Command::ResolveTransfer {
        id: transfer,
        destination: Some(destination),
    };
    let Outcome::Delivered { note: first, .. } = account.run_as(other, accept.clone()).unwrap()
    else {
        panic!("entrega")
    };
    let Outcome::Delivered { note: second, .. } = account.run_as(other, accept).unwrap() else {
        panic!("entrega repetida")
    };
    assert_eq!(first, second);
    let copy = note(&account.connection, other, first).unwrap();
    assert_eq!(
        copy.markdown,
        include_str!("../../../assets/templates/bienvenida.md")
    );
    assert_ne!(first, id);
    let reject = account.run_as(
        other,
        Command::ResolveTransfer {
            id: transfer,
            destination: None,
        },
    );
    assert_eq!(reject.unwrap_err().status(), 409);
    // el remitente no ve la copia ajena
    assert_eq!(
        note(&account.connection, account.owner, first)
            .unwrap_err()
            .status(),
        404
    );
}

#[test]
fn sql_in_user_values_is_stored_literally_and_invalid_input_is_rejected() {
    let mut account = Account::new();
    let tables = |account: &Account| -> i64 {
        account
            .connection
            .query_row("SELECT count(*) FROM sqlite_schema", [], |row| row.get(0))
            .unwrap()
    };
    let before = tables(&account);
    let username = "x'); DROP TABLE accounts; --";
    let intruder = register(&mut account.connection, username, "hash").unwrap();
    let title = "'; DELETE FROM entries; --";
    let note = account.create(None, title, EntryKind::Note);
    let markdown = "\"; UPDATE accounts SET password_hash = 'x'; -- ' OR 1=1";
    let revision = account.revision(note);
    account
        .run(Command::Save {
            id: note,
            revision,
            markdown: markdown.into(),
        })
        .unwrap();
    assert_eq!(account.title(note), title);
    assert_eq!(
        super::note(&account.connection, account.owner, note)
            .unwrap()
            .markdown,
        markdown
    );
    assert_eq!(tables(&account), before);
    let stored: String = account
        .connection
        .query_row(
            "SELECT username FROM accounts WHERE id = ?1",
            [intruder],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored, username);
    // el contenido ajeno sigue aislado aunque el nombre parezca SQL
    assert_eq!(
        super::note(&account.connection, intruder, note)
            .unwrap_err()
            .status(),
        404
    );

    let revision = account.revision(note);
    for title in ["", "   ", "con\u{7}control", &"a".repeat(257)] {
        let refused = account.run(Command::RenameEntry {
            id: note,
            revision,
            title: title.into(),
        });
        assert_eq!(refused.unwrap_err().status(), 400, "{title:?}");
    }
    let refused = account.run(Command::Save {
        id: note,
        revision,
        markdown: "control C1 \u{85}".into(),
    });
    assert_eq!(refused.unwrap_err().status(), 400);
    assert_eq!(account.title(note), title);
}

#[test]
fn suggestions_keep_the_author_unless_anonymous_and_anonymous_ones_leave_no_receipt() {
    let mut account = Account::new();
    let receipts = |account: &Account| -> i64 {
        account
            .connection
            .query_row("SELECT count(*) FROM operation_receipts", [], |row| {
                row.get(0)
            })
            .unwrap()
    };
    for (text, anonymous) in [("  Modo oscuro por defecto ", false), ("Más temas", true)] {
        account
            .run(Command::Suggest {
                text: text.into(),
                anonymous,
            })
            .unwrap();
    }
    let stored: Vec<(Option<Id>, String, String)> = account
        .connection
        .prepare("SELECT account_id, body, created_on FROM suggestions ORDER BY id")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(stored[0].0, Some(account.owner));
    assert_eq!(stored[0].1, "Modo oscuro por defecto");
    assert_eq!(stored[1].0, None);
    assert_eq!(stored[1].1, "Más temas");
    // solo el día, sin hora
    assert_eq!(stored[1].2.len(), "2026-11-15".len());
    assert_eq!(receipts(&account), 1);

    for text in [
        "   ".to_owned(),
        "a".repeat(anotau_protocol::MAX_SUGGESTION_BYTES + 1),
        "hola\u{1b}[2J".to_owned(),
    ] {
        let error = account
            .run(Command::Suggest {
                text,
                anonymous: false,
            })
            .unwrap_err();
        assert_eq!(error.status(), 400);
    }
    account
        .run(Command::Suggest {
            text: "línea uno\nlínea dos".into(),
            anonymous: false,
        })
        .unwrap();
}

#[test]
fn deleting_an_account_removes_every_trace_and_keeps_copies_others_accepted() {
    use anotau_protocol::Resolution;
    let mut account = Account::new();
    let other = register(&mut account.connection, "otra", "hash").unwrap();
    let note = account.create(None, "Compartida", EntryKind::Note);
    let folder = account.create(None, "Carpeta", EntryKind::Folder);
    account.create(Some(folder), "Dentro", EntryKind::Note);
    account.run(Command::Contact { recipient: other }).unwrap();
    let contact: Id = account
        .connection
        .query_row("SELECT id FROM contacts", [], |row| row.get(0))
        .unwrap();
    account
        .run_as(
            other,
            Command::ResolveContact {
                id: contact,
                resolution: Resolution::Accept,
            },
        )
        .unwrap();
    let revision = account.revision(note);
    account
        .run(Command::Offer {
            id: note,
            revision,
            recipient: other,
        })
        .unwrap();
    let transfer: Id = account
        .connection
        .query_row("SELECT id FROM transfers", [], |row| row.get(0))
        .unwrap();
    let workspace = snapshot(&mut account.connection, other).unwrap().workspaces[0].id;
    account
        .run_as(
            other,
            Command::ResolveTransfer {
                id: transfer,
                destination: Some(Destination {
                    workspace,
                    parent: None,
                }),
            },
        )
        .unwrap();
    for anonymous in [false, true] {
        account
            .run(Command::Suggest {
                text: "Idea".into(),
                anonymous,
            })
            .unwrap();
    }

    delete_account(&mut account.connection, account.owner).unwrap();
    let count = |sql: &str| -> i64 {
        account
            .connection
            .query_row(sql, [account.owner], |row| row.get(0))
            .unwrap()
    };
    for sql in [
        "SELECT count(*) FROM accounts WHERE id = ?1",
        "SELECT count(*) FROM sessions WHERE account_id = ?1",
        "SELECT count(*) FROM account_preferences WHERE account_id = ?1",
        "SELECT count(*) FROM workspaces WHERE account_id = ?1",
        "SELECT count(*) FROM contacts WHERE sender_id = ?1 OR recipient_id = ?1",
        "SELECT count(*) FROM transfers WHERE sender_id = ?1 OR recipient_id = ?1",
        "SELECT count(*) FROM operation_receipts WHERE account_id = ?1",
        "SELECT count(*) FROM suggestions WHERE account_id = ?1",
    ] {
        assert_eq!(count(sql), 0, "{sql}");
    }
    assert_eq!(
        count("SELECT count(*) FROM entries WHERE ?1 > 0"),
        2,
        "Bienvenida y copia ajenas"
    );
    assert_eq!(count("SELECT count(*) FROM suggestions WHERE ?1 > 0"), 1);
    let theirs = snapshot(&mut account.connection, other).unwrap();
    assert!(
        theirs
            .entries
            .iter()
            .any(|entry| entry.title == "Compartida")
    );
    assert!(theirs.directory.people.is_empty());
    // el nombre queda libre y la cuenta nueva no hereda nada
    let again = register(&mut account.connection, "yo", "hash").unwrap();
    assert_ne!(again, account.owner);
}

#[test]
fn an_account_holds_at_most_fifty_notes_and_a_refused_copy_changes_nothing() {
    let mut account = Account::new();
    let folder = account.create(None, "Ramos", EntryKind::Folder);
    // la Bienvenida cuenta como la primera
    for index in 2..=anotau_protocol::MAX_NOTES {
        account.create(Some(folder), &format!("Nota {index}"), EntryKind::Note);
    }
    let workspace = account.personal();
    let refused = account
        .run(Command::CreateEntry {
            workspace,
            parent: None,
            title: "Una más".into(),
            kind: EntryKind::Note,
        })
        .unwrap_err();
    assert_eq!(refused.status(), 400);
    assert!(refused.message().contains("límite de 50 notas"));
    // las carpetas no cuentan; copiar la carpeta llena se revierte entera
    account.create(None, "Vacía", EntryKind::Folder);
    let revision = account.revision(folder);
    let entries = account.state().entries.len();
    let copy = account.run(Command::CopyEntry {
        id: folder,
        revision,
        parent: None,
        drafts: Vec::new(),
    });
    assert_eq!(copy.unwrap_err().status(), 400);
    assert_eq!(account.state().entries.len(), entries);
}
