//! Servidor de Anotau: API HTTPS de dominio sobre `SQLite`.
//!
//! El binario `anotau-server` usa estos módulos; los tests del cliente los usan
//! para ejercitar el recorrido real contra una base local.
#![expect(
    clippy::missing_errors_doc,
    reason = "biblioteca interna del workspace; cada error ya es la respuesta HTTP"
)]
pub mod api;
pub mod backup;
pub mod database;
pub mod files;
pub mod limits;
pub mod recovery;
pub mod store;
