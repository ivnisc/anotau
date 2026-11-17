-- Vencimiento de una contraseña temporal asignada con `anotau-server reset-password`. Mientras
-- no se cambie, solo sirve hasta esta fecha (segundos Unix); NULL es una contraseña definitiva.
ALTER TABLE accounts ADD COLUMN password_expires_at INTEGER
    CHECK (password_expires_at IS NULL OR password_expires_at >= 0);
