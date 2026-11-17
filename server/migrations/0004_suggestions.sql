-- Sugerencias enviadas desde Configuración. Se leen directamente en SQLite; la API no las expone.
-- Una sugerencia anónima no guarda la cuenta, y la fecha se registra solo con el día para que la
-- hora no permita asociarla con una sesión.
CREATE TABLE suggestions (
    id INTEGER PRIMARY KEY,
    account_id INTEGER REFERENCES accounts(id),
    body TEXT NOT NULL CHECK (length(CAST(body AS BLOB)) BETWEEN 1 AND 4096),
    created_on TEXT NOT NULL DEFAULT (date('now'))
) STRICT;
