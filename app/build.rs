// Fails the build when `QAMILLE_PRESET_SERVER` is set but is not a valid server address
// (spec A6): a Professional build must never ship as self-hosted by mistake.
mod server_url {
    #![allow(dead_code)]
    include!("src/domain/server_url.rs");
}

fn main() {
    println!("cargo:rerun-if-env-changed=QAMILLE_PRESET_SERVER");
    println!("cargo:rerun-if-changed=src/domain/server_url.rs");
    if let Ok(value) = std::env::var("QAMILLE_PRESET_SERVER") {
        if !value.trim().is_empty() {
            if let Err(error) = server_url::normalize_server_url(&value) {
                panic!("QAMILLE_PRESET_SERVER is not a valid server address ({}): {}", error.code(), error.message());
            }
        }
    }
}
