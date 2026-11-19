//! Incrusta en el cliente la CA de una instancia, como `ANOTAU_DEFAULT_SERVER_URL` incrusta su
//! URL: con `ANOTAU_DEFAULT_CA_CERT=<ruta a ca.crt>` al compilar, el binario confía en esa CA sin
//! configurar nada. Sin la variable queda vacía y solo cuentan las raíces del sistema.

use std::{env, fs, path::PathBuf};

fn main() {
    println!("cargo::rerun-if-env-changed=ANOTAU_DEFAULT_CA_CERT");
    let certificate = match env::var_os("ANOTAU_DEFAULT_CA_CERT") {
        Some(path) => {
            let path = PathBuf::from(path);
            println!("cargo::rerun-if-changed={}", path.display());
            let pem = fs::read(&path).unwrap_or_else(|error| {
                panic!(
                    "ANOTAU_DEFAULT_CA_CERT: no se pudo leer {}: {error}",
                    path.display()
                )
            });
            // solo un certificado público: una clave privada no debe terminar en el binario
            let text = String::from_utf8_lossy(&pem);
            assert!(
                text.contains("-----BEGIN CERTIFICATE-----") && !text.contains("PRIVATE KEY"),
                "ANOTAU_DEFAULT_CA_CERT debe ser un certificado PEM, sin claves privadas"
            );
            pem
        }
        None => Vec::new(),
    };
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("cargo define OUT_DIR"));
    fs::write(out.join("default_ca.pem"), certificate).expect("no se pudo escribir la CA");
}
