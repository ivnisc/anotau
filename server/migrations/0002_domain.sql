CREATE TABLE accounts (
    id INTEGER PRIMARY KEY AUTOINCREMENT CHECK (id > 0),
    username TEXT NOT NULL UNIQUE CHECK (length(username) > 0),
    password_hash TEXT NOT NULL CHECK (length(password_hash) > 0),
    created_at INTEGER NOT NULL DEFAULT (unixepoch()) CHECK (created_at >= 0)
) STRICT;

CREATE TABLE sessions (
    token_hash BLOB PRIMARY KEY CHECK (length(token_hash) = 32),
    account_id INTEGER NOT NULL REFERENCES accounts(id),
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    last_activity_at INTEGER NOT NULL DEFAULT (unixepoch()),
    last_seen_at INTEGER NOT NULL DEFAULT (unixepoch()),
    revoked_at INTEGER,
    CHECK (created_at >= 0 AND last_activity_at >= created_at),
    CHECK (last_seen_at >= last_activity_at),
    CHECK (revoked_at IS NULL OR revoked_at >= created_at)
) STRICT;
CREATE INDEX sessions_account ON sessions(account_id, last_seen_at);

CREATE TABLE account_preferences (
    account_id INTEGER PRIMARY KEY REFERENCES accounts(id),
    format_version INTEGER NOT NULL DEFAULT 1 CHECK (format_version > 0),
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
    settings TEXT NOT NULL DEFAULT '{"autosave":true}'
        CHECK (json_valid(settings) AND json_type(settings) = 'object'),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch()) CHECK (updated_at >= 0)
) STRICT;

CREATE TABLE workspaces (
    id INTEGER PRIMARY KEY AUTOINCREMENT CHECK (id > 0),
    account_id INTEGER NOT NULL REFERENCES accounts(id),
    title TEXT NOT NULL CHECK (length(trim(title)) > 0),
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
    created_at INTEGER NOT NULL DEFAULT (unixepoch()) CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch()) CHECK (updated_at >= created_at),
    UNIQUE (account_id, title)
) STRICT;

CREATE TRIGGER workspaces_identity BEFORE UPDATE OF id, account_id ON workspaces
WHEN NEW.id != OLD.id OR NEW.account_id != OLD.account_id
BEGIN
    SELECT RAISE(ABORT, 'workspace identity is immutable');
END;

-- notas y carpetas comparten el espacio de nombres de cada directorio.
CREATE TABLE entries (
    id INTEGER PRIMARY KEY AUTOINCREMENT CHECK (id > 0),
    workspace_id INTEGER NOT NULL REFERENCES workspaces(id),
    parent_id INTEGER,
    parent_kind TEXT GENERATED ALWAYS AS ('folder') STORED,
    kind TEXT NOT NULL CHECK (kind IN ('note', 'folder')),
    title TEXT NOT NULL CHECK (length(trim(title)) > 0),
    markdown TEXT,
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision > 0),
    created_at INTEGER NOT NULL DEFAULT (unixepoch()) CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch()) CHECK (updated_at >= created_at),
    UNIQUE (workspace_id, id, kind),
    FOREIGN KEY (workspace_id, parent_id, parent_kind)
        REFERENCES entries(workspace_id, id, kind),
    CHECK (parent_id IS NULL OR parent_id != id),
    CHECK ((kind = 'note' AND markdown IS NOT NULL)
        OR (kind = 'folder' AND markdown IS NULL))
) STRICT;
CREATE UNIQUE INDEX entries_root_title ON entries(workspace_id, title) WHERE parent_id IS NULL;
CREATE UNIQUE INDEX entries_child_title ON entries(workspace_id, parent_id, title) WHERE parent_id IS NOT NULL;

CREATE TRIGGER entries_identity BEFORE UPDATE OF id, workspace_id, kind ON entries
WHEN NEW.id != OLD.id OR NEW.workspace_id != OLD.workspace_id OR NEW.kind != OLD.kind
BEGIN
    SELECT RAISE(ABORT, 'entry identity is immutable');
END;

CREATE TRIGGER entries_no_cycle BEFORE UPDATE OF parent_id ON entries
WHEN NEW.parent_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'folder cycle') WHERE EXISTS (
        WITH RECURSIVE ancestors(id) AS (
            SELECT NEW.parent_id
            UNION
            SELECT entries.parent_id FROM entries JOIN ancestors ON entries.id = ancestors.id
            WHERE entries.parent_id IS NOT NULL
        )
        SELECT 1 FROM ancestors WHERE id = NEW.id
    );
END;

CREATE TRIGGER entries_no_cycle_insert AFTER INSERT ON entries
WHEN NEW.parent_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'folder cycle') WHERE EXISTS (
        WITH RECURSIVE ancestors(id) AS (
            SELECT NEW.parent_id
            UNION
            SELECT entries.parent_id FROM entries JOIN ancestors ON entries.id = ancestors.id
            WHERE entries.parent_id IS NOT NULL
        )
        SELECT 1 FROM ancestors WHERE id = NEW.id
    );
END;

CREATE TABLE contacts (
    id INTEGER PRIMARY KEY AUTOINCREMENT CHECK (id > 0),
    sender_id INTEGER NOT NULL REFERENCES accounts(id),
    recipient_id INTEGER NOT NULL REFERENCES accounts(id),
    state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending', 'accepted', 'rejected')),
    created_at INTEGER NOT NULL DEFAULT (unixepoch()) CHECK (created_at >= 0),
    resolved_at INTEGER,
    CHECK (sender_id != recipient_id),
    CHECK ((state = 'pending' AND resolved_at IS NULL)
        OR (state != 'pending' AND resolved_at IS NOT NULL AND resolved_at >= created_at))
) STRICT;
CREATE UNIQUE INDEX contacts_pair ON contacts(min(sender_id, recipient_id), max(sender_id, recipient_id));
CREATE INDEX contacts_recipient ON contacts(recipient_id, state);
CREATE INDEX contacts_sender ON contacts(sender_id, state);

CREATE TRIGGER contacts_pending BEFORE INSERT ON contacts WHEN NEW.state != 'pending'
BEGIN
    SELECT RAISE(ABORT, 'contact must start pending');
END;
CREATE TRIGGER contacts_identity BEFORE UPDATE OF id, sender_id, recipient_id, created_at ON contacts
BEGIN
    SELECT RAISE(ABORT, 'contact identity is immutable');
END;
CREATE TRIGGER contacts_terminal BEFORE UPDATE ON contacts WHEN OLD.state != 'pending'
BEGIN
    SELECT RAISE(ABORT, 'contact already resolved');
END;

CREATE TABLE transfers (
    id INTEGER PRIMARY KEY AUTOINCREMENT CHECK (id > 0),
    sender_id INTEGER NOT NULL REFERENCES accounts(id),
    recipient_id INTEGER NOT NULL REFERENCES accounts(id),
    -- referencias históricas: borrar una nota no borra la oferta ni su comprobante.
    source_note_id INTEGER NOT NULL CHECK (source_note_id > 0),
    source_revision INTEGER NOT NULL CHECK (source_revision > 0),
    title TEXT NOT NULL CHECK (length(trim(title)) > 0),
    markdown TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending', 'accepted', 'rejected')),
    delivered_note_id INTEGER UNIQUE CHECK (delivered_note_id > 0),
    created_at INTEGER NOT NULL DEFAULT (unixepoch()) CHECK (created_at >= 0),
    resolved_at INTEGER,
    CHECK (sender_id != recipient_id),
    CHECK ((state = 'pending' AND resolved_at IS NULL AND delivered_note_id IS NULL)
        OR (state = 'rejected' AND resolved_at IS NOT NULL AND resolved_at >= created_at AND delivered_note_id IS NULL)
        OR (state = 'accepted' AND resolved_at IS NOT NULL AND resolved_at >= created_at AND delivered_note_id IS NOT NULL))
) STRICT;
CREATE INDEX transfers_inbox ON transfers(recipient_id, state, id);
CREATE INDEX transfers_sender ON transfers(sender_id, id);

CREATE TRIGGER transfers_contact BEFORE INSERT ON transfers
BEGIN
    SELECT RAISE(ABORT, 'transfer must start pending') WHERE NEW.state != 'pending';
    SELECT RAISE(ABORT, 'accepted contact required') WHERE NOT EXISTS (
        SELECT 1 FROM contacts
        WHERE min(sender_id, recipient_id) = min(NEW.sender_id, NEW.recipient_id)
          AND max(sender_id, recipient_id) = max(NEW.sender_id, NEW.recipient_id)
          AND state = 'accepted'
    );
    SELECT RAISE(ABORT, 'sender must own the offered note') WHERE NOT EXISTS (
        SELECT 1 FROM entries JOIN workspaces ON workspaces.id = entries.workspace_id
        WHERE entries.id = NEW.source_note_id AND entries.kind = 'note'
          AND entries.revision = NEW.source_revision AND workspaces.account_id = NEW.sender_id
    );
END;
CREATE TRIGGER transfers_snapshot BEFORE UPDATE OF id, sender_id, recipient_id, source_note_id,
    source_revision, title, markdown, created_at ON transfers
BEGIN
    SELECT RAISE(ABORT, 'transfer snapshot is immutable');
END;
CREATE TRIGGER transfers_terminal BEFORE UPDATE ON transfers WHEN OLD.state != 'pending'
BEGIN
    SELECT RAISE(ABORT, 'transfer already resolved');
END;
CREATE TRIGGER transfers_delivery BEFORE UPDATE OF state, delivered_note_id ON transfers
WHEN NEW.state = 'accepted'
BEGIN
    SELECT RAISE(ABORT, 'recipient must own the delivered copy') WHERE NOT EXISTS (
        SELECT 1 FROM entries JOIN workspaces ON workspaces.id = entries.workspace_id
        WHERE entries.id = NEW.delivered_note_id AND entries.kind = 'note'
          AND entries.id != NEW.source_note_id AND workspaces.account_id = NEW.recipient_id
          AND entries.markdown = NEW.markdown
    );
END;

CREATE TABLE operation_receipts (
    account_id INTEGER NOT NULL REFERENCES accounts(id),
    operation_id TEXT NOT NULL CHECK (length(operation_id) > 0),
    request_hash BLOB NOT NULL CHECK (length(request_hash) = 32),
    result TEXT NOT NULL CHECK (json_valid(result) AND json_type(result) = 'object'),
    created_at INTEGER NOT NULL DEFAULT (unixepoch()) CHECK (created_at >= 0),
    PRIMARY KEY (account_id, operation_id)
) STRICT;
CREATE TRIGGER operation_receipts_immutable BEFORE UPDATE ON operation_receipts
BEGIN
    SELECT RAISE(ABORT, 'operation receipt is immutable');
END;
