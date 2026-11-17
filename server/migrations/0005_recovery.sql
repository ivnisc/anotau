-- Solicitudes de recuperación de contraseña. La API solo las crea; el administrador las lista y
-- resuelve con `anotau-server tickets` y `anotau-server reset-password <usuario>`, tras verificar la
-- identidad por el contacto indicado. Una cuenta tiene a lo sumo una solicitud pendiente.
CREATE TABLE recovery_tickets (
    id INTEGER PRIMARY KEY AUTOINCREMENT CHECK (id > 0),
    account_id INTEGER NOT NULL REFERENCES accounts(id),
    contact TEXT NOT NULL CHECK (length(contact) BETWEEN 1 AND 254),
    created_at INTEGER NOT NULL DEFAULT (unixepoch()) CHECK (created_at >= 0),
    resolved_at INTEGER CHECK (resolved_at IS NULL OR resolved_at >= created_at)
) STRICT;
CREATE UNIQUE INDEX recovery_tickets_pending ON recovery_tickets(account_id)
    WHERE resolved_at IS NULL;
