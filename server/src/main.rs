//! `qamille-server`: il server del core (utenti creati dall'amministratore, accesso con
//! password, dispositivi e calcolo). La riga di comando è in `qamille_server::cli`.

use std::path::Path;
use std::process::ExitCode;

use qamille_server::cli::{self, Cli};
use qamille_server::{Config, Server};

fn open(config_path: Option<&Path>) -> Result<Server, String> {
    let config =
        Config::load(config_path).map_err(|e| format!("errore di configurazione: {e}"))?;
    let ignored = Config::ignored_env(qamille_server::config::env_var);
    if !ignored.is_empty() {
        tracing::warn!(
            "variabili enterprise ignorate da questo server: {}",
            ignored.join(", ")
        );
    }
    if let Some(notice) = config.plaintext_notice() {
        tracing::warn!("{notice}");
    }
    Server::open(config).map_err(|e| format!("errore: {e}"))
}

fn main() -> ExitCode {
    cli::run(
        std::env::args().skip(1).collect(),
        Cli {
            program: "qamille-server",
            version: env!("CARGO_PKG_VERSION"),
            open: Box::new(open),
        },
    )
}
