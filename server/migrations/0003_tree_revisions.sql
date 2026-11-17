ALTER TABLE accounts ADD COLUMN state_revision INTEGER NOT NULL DEFAULT 1 CHECK (state_revision > 0);

CREATE TRIGGER entries_workspace_insert AFTER INSERT ON entries
BEGIN
    UPDATE workspaces SET revision=revision+1, updated_at=unixepoch() WHERE id=NEW.workspace_id;
END;
CREATE TRIGGER entries_workspace_update AFTER UPDATE ON entries
BEGIN
    UPDATE workspaces SET revision=revision+1, updated_at=unixepoch() WHERE id=NEW.workspace_id;
END;
CREATE TRIGGER entries_workspace_delete AFTER DELETE ON entries
BEGIN
    UPDATE workspaces SET revision=revision+1, updated_at=unixepoch() WHERE id=OLD.workspace_id;
END;

CREATE TRIGGER workspaces_account_insert AFTER INSERT ON workspaces
BEGIN
    UPDATE accounts SET state_revision=state_revision+1 WHERE id=NEW.account_id;
END;
CREATE TRIGGER workspaces_account_update AFTER UPDATE ON workspaces
BEGIN
    UPDATE accounts SET state_revision=state_revision+1 WHERE id=NEW.account_id;
END;
CREATE TRIGGER workspaces_account_delete AFTER DELETE ON workspaces
BEGIN
    UPDATE accounts SET state_revision=state_revision+1 WHERE id=OLD.account_id;
END;
CREATE TRIGGER preferences_account_update AFTER UPDATE ON account_preferences
BEGIN
    UPDATE accounts SET state_revision=state_revision+1 WHERE id=NEW.account_id;
END;
