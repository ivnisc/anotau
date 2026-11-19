use super::*;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

fn state(revision: i64, version: i64) -> api::Snapshot {
    serde_json::from_value(serde_json::json!({
        "workspaces":[{"id":10,"title":"Personal","revision":version}],
        "entries":[{"id":100,"workspace":10,"parent":null,"title":"Nota","kind":"note","revision":revision,"updated_at":0}],
        "directory":{"people":[],"incoming":[]},"preferences":{"revision":1,"settings":{"autosave":false}},"next":null
    })).unwrap()
}

fn editor() -> Editor {
    let mut editor = Editor::from_remote(state(1, 2)).unwrap();
    editor.received_note(api::Note {
        id: 100,
        markdown: "Base".into(),
        revision: 1,
        updated_at: 0,
    });
    editor
}

fn press(editor: &mut Editor, code: KeyCode) {
    editor.handle(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
}

fn request(editor: &mut Editor) -> Arc<api::Mutation> {
    let Some(Job::Mutate(request)) = editor.next_job() else {
        panic!("se esperaba una escritura real")
    };
    request
}

fn acknowledge(editor: &mut Editor, revision: i64) {
    editor.changed(api::Change {
        outcome: Outcome::Saved {
            id: 100,
            revision,
            at: revision,
            workspace_revision: revision + 1,
        },
        state: None,
    });
}

#[test]
fn undo_during_a_save_is_reconciled_before_closing() {
    let mut editor = editor();
    press(&mut editor, KeyCode::Enter);
    editor.handle(Event::Paste("Cambio ".into()));
    editor.save_active(false);
    let first = request(&mut editor);
    assert!(matches!(
        &first.command,
        Command::Save { revision: 1, markdown, .. } if markdown == "Cambio Base"
    ));
    editor.handle(Event::Key(KeyEvent::new(
        KeyCode::Char('z'),
        KeyModifiers::CONTROL,
    )));
    assert_eq!(editor.note().unwrap().source.text(), "Base");
    assert!(editor.note().unwrap().modified);
    press(&mut editor, KeyCode::Esc);
    press(&mut editor, KeyCode::Char('q'));
    assert!(editor.palette.is_some());
    press(&mut editor, KeyCode::Enter);
    acknowledge(&mut editor, 2);
    assert!(editor.note().is_some());
    let second = request(&mut editor);
    assert!(matches!(
        &second.command,
        Command::Save { revision: 2, markdown, .. } if markdown == "Base"
    ));
    acknowledge(&mut editor, 3);
    assert!(editor.note().is_none());
    assert!(!editor.has_changes());
}

#[test]
fn unknown_result_reuses_the_exact_operation_and_arguments() {
    let mut editor = editor();
    press(&mut editor, KeyCode::Enter);
    editor.handle(Event::Paste("Primero ".into()));
    editor.save_active(false);
    let first = request(&mut editor);
    editor.failed(
        network::Origin::Mutate,
        network::Failure::new(Reason::Uncertain, "respuesta perdida"),
    );
    editor.handle(Event::Paste("Segundo ".into()));
    editor.save_active(false);
    let retry = request(&mut editor);
    assert!(Arc::ptr_eq(&first, &retry));
    acknowledge(&mut editor, 2);
    assert!(editor.note().unwrap().modified);
    let next = request(&mut editor);
    assert_ne!(next.operation_id, first.operation_id);
    assert!(matches!(
        &next.command,
        Command::Save { revision: 2, markdown, .. } if markdown == "Primero Segundo Base"
    ));
}

#[test]
fn refreshed_or_removed_popup_sources_cannot_be_confirmed() {
    for action in ['n', 'd'] {
        let mut editor = editor();
        press(&mut editor, KeyCode::F(2));
        if action == 'd' {
            press(&mut editor, KeyCode::Down);
        }
        press(&mut editor, KeyCode::Char(action));
        if action == 'n' {
            editor.handle(Event::Paste("Nueva".into()));
        }
        let mut changed = state(2, 3);
        if action == 'd' {
            changed.entries.clear();
        } else {
            changed.workspaces[0].id = 20;
            changed.entries[0].workspace = 20;
        }
        editor.refresh_remote(changed).unwrap();
        press(&mut editor, KeyCode::Enter);
        assert!(editor.remote.pending.is_none());
        assert!(editor.palette.is_some());
        assert!(editor.remote_error().is_some());
    }
}
