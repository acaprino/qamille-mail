//! Riga di comando condivisa del server del core e di quello dell'enterprise. Parsing a
//! mano, senza dipendenze: i due binari chiamano [`run`] con il loro [`Cli`].
//!
//! Codici d'uscita: 0 riuscito, 1 errore dell'operazione, 2 uso sbagliato
//! (o modalità non ancora disponibile).

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::{Accounts, Server, ServerError};

/// Ciò che distingue un binario dall'altro.
pub struct Cli {
    /// Il nome del programma, per l'aiuto e la versione.
    pub program: &'static str,
    /// La versione del crate del binario.
    pub version: &'static str,
    /// Carica la configurazione (file indicato o predefinito) e apre il server. L'errore è il
    /// messaggio completo da scrivere sullo standard error.
    #[allow(clippy::type_complexity)]
    pub open: Box<dyn Fn(Option<&Path>) -> Result<Server, String>>,
}

const USAGE: &str = "\
qamille-server: server di Qamille (utenti, dispositivi e accesso con password)

Uso:
  qamille-server [--config <file>] serve
  qamille-server [--config <file>] user create <username> [--email <email>] [--max-devices <N>]
  qamille-server [--config <file>] user reset-password <username>
  qamille-server [--config <file>] user disable <username>
  qamille-server [--config <file>] user enable <username>
  qamille-server [--config <file>] user delete <username> --yes
  qamille-server [--config <file>] user list
  qamille-server [--config <file>] user devices <username>
  qamille-server [--config <file>] user revoke-device <username> <device_id>
  qamille-server [--config <file>] user max-devices <username> <N|default>
  qamille-server [--config <file>] model list
  qamille-server [--config <file>] model install <id>
  qamille-server [--config <file>] model remove <id> --yes
  qamille-server sync keygen
  qamille-server [--config <file>] sync rekey
  qamille-server [--config <file>] sync reset --yes
  qamille-server [--config <file>] sync after-restore
  qamille-server --help | --version

`user create` e `user reset-password` stampano sullo standard output solo la
password provvisoria; tutto il resto va sullo standard error.

`model install` scarica un modello (per esempio llm-medio) e aspetta la fine,
con l'avanzamento sullo standard error; `model list` mostra stato, dimensione e
memoria richiesta di ogni modello. Nessuno dei due apre un ascolto.

Configurazione: `server.toml` (da --config, altrimenti <data_dir>/server.toml) e
variabili QAMILLE_<CHIAVE>, che vincono sul file. La chiave amministratore sta
solo in QAMILLE_ADMIN_KEY. Vedi server/README.md.
";

const EXIT_FAILURE: u8 = 1;
const EXIT_USAGE: u8 = 2;

enum Command {
    Help,
    Version,
    Serve,
    /// Processo sacrificabile dei lettori di documenti (INV-11): non è per le persone.
    ParserWorker(Option<u32>),
    User(UserCommand),
    Model(ModelCommand),
    Sync(SyncCommand),
}

/// `model list`, `model install <id>`, `model remove <id> --yes`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelCommand {
    List,
    Install(String),
    Remove { id: String, yes: bool },
}

/// `sync keygen`, `sync rekey`, `sync reset --yes`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SyncCommand {
    Keygen,
    Rekey,
    AfterRestore,
    Reset { yes: bool },
}

enum UserCommand {
    Create {
        username: String,
        email: Option<String>,
        max_devices: Option<u32>,
    },
    ResetPassword(String),
    Disable(String),
    Enable(String),
    Delete(String),
    List,
    Devices(String),
    RevokeDevice(String, String),
    MaxDevices(String, Option<u32>),
}

struct Usage(String);

fn usage(message: impl Into<String>) -> Usage {
    Usage(message.into())
}

fn parse(args: Vec<String>) -> Result<(Option<PathBuf>, Command), Usage> {
    // `--config <file>` vale in qualunque posizione.
    let mut config = None;
    let mut rest = Vec::new();
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        if arg == "--config" {
            let path = iter
                .next()
                .ok_or_else(|| usage("--config vuole il percorso di un file"))?;
            config = Some(PathBuf::from(path));
        } else if let Some(path) = arg.strip_prefix("--config=") {
            config = Some(PathBuf::from(path));
        } else {
            rest.push(arg);
        }
    }
    let words: Vec<&str> = rest.iter().map(String::as_str).collect();
    let command = match words.as_slice() {
        [] | ["--help"] | ["-h"] | ["help"] => Command::Help,
        ["--version"] | ["-V"] => Command::Version,
        ["serve"] => Command::Serve,
        ["--parser-worker"] => Command::ParserWorker(None),
        ["--parser-worker", "--parent", pid] => Command::ParserWorker(Some(parse_count(pid)?)),
        ["serve", other, ..] => {
            return Err(usage(format!("opzione sconosciuta per serve: {other}")));
        }
        ["user", tail @ ..] => Command::User(parse_user(tail)?),
        ["model", tail @ ..] => Command::Model(parse_model(tail)?),
        ["sync", tail @ ..] => Command::Sync(parse_sync(tail)?),
        [other, ..] => return Err(usage(format!("comando sconosciuto: {other}"))),
    };
    Ok((config, command))
}

fn parse_count(value: &str) -> Result<u32, Usage> {
    value
        .parse()
        .map_err(|_| usage(format!("«{value}» non è un numero intero non negativo")))
}

fn parse_model(words: &[&str]) -> Result<ModelCommand, Usage> {
    let id = |rest: &[&str], what: &str| -> Result<String, Usage> {
        match rest {
            [id] if !id.starts_with("--") => Ok((*id).to_owned()),
            _ => Err(usage(format!("uso: {what}"))),
        }
    };
    match words {
        ["list"] => Ok(ModelCommand::List),
        ["list", ..] => Err(usage("uso: model list")),
        ["install", rest @ ..] => Ok(ModelCommand::Install(id(rest, "model install <id>")?)),
        ["remove", id, "--yes"] | ["remove", "--yes", id] if !id.starts_with("--") => {
            Ok(ModelCommand::Remove { id: (*id).to_owned(), yes: true })
        }
        ["remove", id] if !id.starts_with("--") => Ok(ModelCommand::Remove { id: (*id).to_owned(), yes: false }),
        ["remove", ..] => Err(usage("uso: model remove <id> --yes (la rimozione cancella i file del modello)")),
        [other, ..] => Err(usage(format!("comando model sconosciuto: {other}"))),
        [] => Err(usage("manca il comando model (list, install, remove)")),
    }
}

fn parse_user(words: &[&str]) -> Result<UserCommand, Usage> {
    let one = |name: &str, rest: &[&str]| -> Result<String, Usage> {
        match rest {
            [username] if !username.starts_with("--") => Ok((*username).to_owned()),
            _ => Err(usage(format!("uso: user {name} <username>"))),
        }
    };
    match words {
        ["create", rest @ ..] => {
            let mut username = None;
            let mut email = None;
            let mut max_devices = None;
            let mut i = 0;
            while i < rest.len() {
                match rest[i] {
                    "--email" => {
                        email = Some(
                            rest.get(i + 1)
                                .ok_or_else(|| usage("--email vuole un indirizzo"))?
                                .to_string(),
                        );
                        i += 2;
                    }
                    "--max-devices" => {
                        let value = rest
                            .get(i + 1)
                            .ok_or_else(|| usage("--max-devices vuole un numero"))?;
                        max_devices = Some(parse_count(value)?);
                        i += 2;
                    }
                    flag if flag.starts_with("--") => {
                        return Err(usage(format!(
                            "opzione sconosciuta per user create: {flag}"
                        )));
                    }
                    value if username.is_none() => {
                        username = Some(value.to_owned());
                        i += 1;
                    }
                    value => return Err(usage(format!("argomento in più: {value}"))),
                }
            }
            let username = username.ok_or_else(|| {
                usage("uso: user create <username> [--email <email>] [--max-devices <N>]")
            })?;
            Ok(UserCommand::Create {
                username,
                email,
                max_devices,
            })
        }
        ["reset-password", rest @ ..] => {
            Ok(UserCommand::ResetPassword(one("reset-password", rest)?))
        }
        ["disable", rest @ ..] => Ok(UserCommand::Disable(one("disable", rest)?)),
        ["enable", rest @ ..] => Ok(UserCommand::Enable(one("enable", rest)?)),
        ["delete", username, "--yes"] | ["delete", "--yes", username]
            if !username.starts_with("--") =>
        {
            Ok(UserCommand::Delete((*username).to_owned()))
        }
        ["delete", ..] => Err(usage(
            "uso: user delete <username> --yes (la cancellazione non si annulla)",
        )),
        ["list"] => Ok(UserCommand::List),
        ["devices", rest @ ..] => Ok(UserCommand::Devices(one("devices", rest)?)),
        ["revoke-device", username, device_id] => Ok(UserCommand::RevokeDevice(
            (*username).to_owned(),
            (*device_id).to_owned(),
        )),
        ["revoke-device", ..] => Err(usage("uso: user revoke-device <username> <device_id>")),
        ["max-devices", username, value] => {
            let max = if *value == "default" {
                None
            } else {
                Some(parse_count(value)?)
            };
            Ok(UserCommand::MaxDevices((*username).to_owned(), max))
        }
        ["max-devices", ..] => Err(usage("uso: user max-devices <username> <N|default>")),
        [other, ..] => Err(usage(format!("comando user sconosciuto: {other}"))),
        [] => Err(usage("manca il comando user (create, list, …)")),
    }
}

/// Il testo d'aiuto di un binario: il nome del programma al posto di `qamille-server`.
fn usage_text(cli: &Cli) -> String {
    USAGE.replace("qamille-server", cli.program)
}

/// Esegue la riga di comando `args` (senza il nome del programma).
pub fn run(args: Vec<String>, cli: Cli) -> ExitCode {
    let (config_path, command) = match parse(args) {
        Ok(parsed) => parsed,
        Err(Usage(message)) => {
            eprintln!("errore: {message}
");
            eprint!("{}", usage_text(&cli));
            return ExitCode::from(EXIT_USAGE);
        }
    };
    match command {
        Command::Help => {
            print!("{}", usage_text(&cli));
            ExitCode::SUCCESS
        }
        Command::Version => {
            println!("{} {}", cli.program, cli.version);
            ExitCode::SUCCESS
        }
        Command::ParserWorker(parent) => {
            // Un panic è già una risposta (frames::catch_panic): niente su stderr.
            std::panic::set_hook(Box::new(|_| {}));
            // Un lettore bloccato non legge più lo stdin: il processo finisce col server.
            if let Some(parent) = parent {
                let _ = std::thread::Builder::new().name("parent-watch".to_owned()).spawn(move || {
                    while crate::process::parent_alive(parent) {
                        std::thread::sleep(std::time::Duration::from_secs(2));
                    }
                    std::process::exit(0);
                });
            }
            qamille_compute::frames::serve(&mut std::io::stdin().lock(), &mut std::io::stdout().lock());
            ExitCode::SUCCESS
        }
        Command::Serve => serve(&cli, config_path),
        Command::User(user) => run_user(&cli, config_path, user),
        Command::Model(model) => run_model_command(&cli, config_path, model),
        Command::Sync(sync) => run_sync_command(&cli, config_path, sync),
    }
}

/// Apre il server senza ascolto, con il calcolo vero se non ce l'ha già, ed esegue `command`.
fn run_model_command(cli: &Cli, config_path: Option<PathBuf>, command: ModelCommand) -> ExitCode {
    let server = match (cli.open)(config_path.as_deref()) {
        Ok(server) => server,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(EXIT_FAILURE);
        }
    };
    let server = if server.compute_state().is_some() {
        server
    } else {
        match server.with_real_compute() {
            Ok(server) => server,
            Err(e) => {
                eprintln!("errore: {e}");
                return ExitCode::from(EXIT_FAILURE);
            }
        }
    };
    let Some(state) = server.compute_state() else {
        eprintln!("errore: il calcolo non è disponibile");
        return ExitCode::from(EXIT_FAILURE);
    };
    let code = run_model(&state, command, &mut std::io::stdout(), &mut std::io::stderr());
    ExitCode::from(code)
}

/// The longest `model install` waits for the end of a download.
const INSTALL_LIMIT: std::time::Duration = std::time::Duration::from_secs(24 * 3600);

/// Esegue un comando `model`; il codice d'uscita è 0, 1 (errore dell'operazione) o 2 (uso).
/// Nessun segreto, nessun indirizzo e nessun percorso in uscita.
pub fn run_model(state: &crate::compute::ComputeState, command: ModelCommand, out: &mut dyn Write, err: &mut dyn Write) -> u8 {
    run_model_polling(state, command, out, err, std::time::Duration::from_secs(1))
}

/// Come [`run_model`], con l'intervallo tra due letture dell'avanzamento (i test lo accorciano).
pub fn run_model_polling(
    state: &crate::compute::ComputeState,
    command: ModelCommand,
    out: &mut dyn Write,
    err: &mut dyn Write,
    poll: std::time::Duration,
) -> u8 {
    match command {
        ModelCommand::List => {
            let _ = writeln!(out, "id\tstato\tavanzamento\tdimensione\tmemoria richiesta");
            for (model, details) in state.models_with_details() {
                let percent = model.percent.map_or_else(|| "-".to_owned(), |p| format!("{p}%"));
                let size = if model.total_bytes > 0 { format!("{} MB", model.total_bytes / 1_000_000) } else { "-".to_owned() };
                let memory = details.map_or_else(|| "-".to_owned(), |d| format!("{} MB", d.ram_required_mb));
                let _ = writeln!(out, "{}\t{}\t{percent}\t{size}\t{memory}", model.id, model.state);
            }
            0
        }
        ModelCommand::Install(id) => {
            let mut status = match state.install_model(&id) {
                Ok(status) => status,
                Err(error) => {
                    let _ = writeln!(err, "errore: {}", error.message());
                    return EXIT_FAILURE;
                }
            };
            if status.state != "ready"
                && let Some(details) = state.llm_models.details(&id)
            {
                let _ = writeln!(out, "{id}: licenza {}, condizioni: {}", details.license, details.license_url);
            }
            let started = std::time::Instant::now();
            let mut last = None;
            loop {
                match status.state.as_str() {
                    "ready" => {
                        let _ = writeln!(out, "{id}: installato");
                        return 0;
                    }
                    "downloading" => {
                        let percent = status.percent.unwrap_or(0);
                        if last != Some(percent) {
                            let _ = writeln!(err, "{id}: {percent}%");
                            last = Some(percent);
                        }
                    }
                    _ => {
                        let reason = status.error.clone().unwrap_or_else(|| "l'installazione si è fermata".to_owned());
                        let _ = writeln!(err, "errore: {reason}");
                        return EXIT_FAILURE;
                    }
                }
                if started.elapsed() > INSTALL_LIMIT {
                    let _ = writeln!(err, "errore: l'installazione non è finita in 24 ore");
                    return EXIT_FAILURE;
                }
                std::thread::sleep(poll);
                match state.all_models().into_iter().find(|model| model.id == id) {
                    Some(next) => status = next,
                    None => {
                        let _ = writeln!(err, "errore: il modello non c'è più");
                        return EXIT_FAILURE;
                    }
                }
            }
        }
        ModelCommand::Remove { id, yes } => {
            if !yes {
                let _ = writeln!(err, "errore: uso: model remove <id> --yes (la rimozione cancella i file del modello)");
                return EXIT_USAGE;
            }
            match state.remove_model(&id) {
                Ok(status) => {
                    let _ = writeln!(out, "{}\t{}", status.id, status.state);
                    0
                }
                Err(error) => {
                    let _ = writeln!(err, "errore: {}", error.message());
                    EXIT_FAILURE
                }
            }
        }
    }
}

fn serve(cli: &Cli, config_path: Option<PathBuf>) -> ExitCode {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .with_ansi(std::io::stderr().is_terminal())
        .init();
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("errore: non riesco ad avviare il runtime: {e}");
            return ExitCode::from(EXIT_FAILURE);
        }
    };
    let result: Result<(), String> = runtime.block_on(async {
        let server = (cli.open)(config_path.as_deref())?;
        let server = server.with_real_compute().map_err(|e| format!("errore: {e}"))?;
        server.serve(shutdown_signal()).await.map_err(|e| format!("errore: {e}"))
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::from(EXIT_FAILURE)
        }
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

fn run_user(cli: &Cli, config_path: Option<PathBuf>, command: UserCommand) -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("errore: non riesco ad avviare il runtime: {e}");
            return ExitCode::from(EXIT_FAILURE);
        }
    };
    let server = match (cli.open)(config_path.as_deref()) {
        Ok(server) => server,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(EXIT_FAILURE);
        }
    };
    let accounts = server.accounts().clone();
    match runtime.block_on(user_command(&accounts, command)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("errore: {e}");
            ExitCode::from(EXIT_FAILURE)
        }
    }
}

fn print_secret_line(secret: &str) -> Result<(), ServerError> {
    let mut out = std::io::stdout().lock();
    writeln!(out, "{secret}")
        .and_then(|_| out.flush())
        .map_err(|e| ServerError::internal(format!("stdout: {e}")))
}

fn or_dash(value: Option<&str>) -> &str {
    value.unwrap_or("-")
}

async fn user_command(accounts: &Accounts, command: UserCommand) -> Result<(), ServerError> {
    match command {
        UserCommand::Create {
            username,
            email,
            max_devices,
        } => {
            let created = accounts
                .create_user(&username, email.as_deref(), max_devices)
                .await?;
            eprintln!(
                "Utente «{}» creato (id {}). Password provvisoria, valida fino a {}:",
                created.username, created.id, created.temporary_expires_at
            );
            print_secret_line(&created.temporary_password)
        }
        UserCommand::ResetPassword(username) => {
            let id = accounts.user_id_by_username(&username).await?;
            let temporary = accounts.reset_password(&id).await?;
            eprintln!(
                "Nuova password provvisoria per «{username}», valida fino a {} (i dispositivi sono stati disconnessi):",
                temporary.temporary_expires_at
            );
            print_secret_line(&temporary.temporary_password)
        }
        UserCommand::Disable(username) => {
            let id = accounts.user_id_by_username(&username).await?;
            accounts.disable_user(&id).await?;
            eprintln!("Utente «{username}» disattivato.");
            Ok(())
        }
        UserCommand::Enable(username) => {
            let id = accounts.user_id_by_username(&username).await?;
            accounts.enable_user(&id).await?;
            eprintln!("Utente «{username}» riattivato.");
            Ok(())
        }
        UserCommand::Delete(username) => {
            let id = accounts.user_id_by_username(&username).await?;
            accounts.delete_user(&id).await?;
            eprintln!("Utente «{username}» cancellato con i suoi dispositivi.");
            Ok(())
        }
        UserCommand::List => {
            let users = accounts.list_users().await?;
            let default_max = accounts.default_max_devices();
            println!("username\temail\tstato\tdispositivi\tmassimo\tcreato");
            for user in users {
                let state = if user.disabled_at.is_some() {
                    "disattivato"
                } else if user.password_temporary {
                    "provvisoria"
                } else {
                    "attivo"
                };
                let max = match user.max_devices.unwrap_or(default_max) {
                    0 => "illimitato".to_owned(),
                    n => n.to_string(),
                };
                println!(
                    "{}\t{}\t{}\t{}\t{}\t{}",
                    user.username,
                    or_dash(user.email.as_deref()),
                    state,
                    user.active_devices,
                    max,
                    user.created_at
                );
            }
            Ok(())
        }
        UserCommand::Devices(username) => {
            let id = accounts.user_id_by_username(&username).await?;
            let devices = accounts.list_devices(&id).await?;
            println!("id\tnome\tcreato\tultimo accesso");
            for device in devices {
                println!(
                    "{}\t{}\t{}\t{}",
                    device.id,
                    device.name,
                    device.created_at,
                    or_dash(device.last_seen_at.as_deref())
                );
            }
            Ok(())
        }
        UserCommand::RevokeDevice(username, device_id) => {
            let id = accounts.user_id_by_username(&username).await?;
            accounts.revoke_device_admin(&id, &device_id).await?;
            eprintln!("Dispositivo {device_id} di «{username}» disattivato.");
            Ok(())
        }
        UserCommand::MaxDevices(username, max) => {
            let id = accounts.user_id_by_username(&username).await?;
            accounts.set_max_devices(&id, max).await?;
            match max {
                Some(0) => eprintln!("«{username}»: nessun limite di dispositivi."),
                Some(n) => eprintln!("«{username}»: al massimo {n} dispositivi."),
                None => eprintln!(
                    "«{username}»: vale il massimo predefinito ({}).",
                    accounts.default_max_devices()
                ),
            }
            Ok(())
        }
    }
}

fn parse_sync(words: &[&str]) -> Result<SyncCommand, Usage> {
    match words {
        ["after-restore"] => Ok(SyncCommand::AfterRestore),
        ["keygen"] => Ok(SyncCommand::Keygen),
        ["keygen", ..] => Err(usage("uso: sync keygen")),
        ["rekey"] => Ok(SyncCommand::Rekey),
        ["rekey", ..] => Err(usage("uso: sync rekey (con QAMILLE_SYNC_KEY e QAMILLE_SYNC_KEY_OLD)")),
        ["reset", "--yes"] => Ok(SyncCommand::Reset { yes: true }),
        ["reset"] => Ok(SyncCommand::Reset { yes: false }),
        ["reset", ..] => Err(usage("uso: sync reset --yes (cancella tutte le caselle salvate sul server)")),
        [other, ..] => Err(usage(format!("comando sync sconosciuto: {other}"))),
        [] => Err(usage("manca il comando sync (keygen, rekey, reset, after-restore)")),
    }
}

/// `sync keygen` non apre né la configurazione né il database; `rekey` e `reset` aprono il
/// server senza ascolto e lavorano sul suo database. Le chiavi non si stampano mai, tranne la
/// nuova di `keygen`.
fn run_sync_command(cli: &Cli, config_path: Option<PathBuf>, command: SyncCommand) -> ExitCode {
    if command == SyncCommand::Keygen {
        let key = crate::sync::SyncKey::generate();
        if print_secret_line(&key.to_hex()).is_err() {
            return ExitCode::from(EXIT_FAILURE);
        }
        eprintln!(
            "Metti questa chiave in QAMILLE_SYNC_KEY, nell'ambiente del server: fuori dal database e dai backup di server.db."
        );
        return ExitCode::SUCCESS;
    }
    if command == (SyncCommand::Reset { yes: false }) {
        eprintln!("errore: uso: sync reset --yes (cancella tutte le caselle salvate sul server)");
        return ExitCode::from(EXIT_USAGE);
    }
    let server = match (cli.open)(config_path.as_deref()) {
        Ok(server) => server,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(EXIT_FAILURE);
        }
    };
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("errore: non riesco ad avviare il runtime: {e}");
            return ExitCode::from(EXIT_FAILURE);
        }
    };
    let db = server.accounts().db().clone();
    let sync = server.sync_config().clone();
    let result: Result<String, String> = match command {
        SyncCommand::Rekey => match (sync.key, sync.old_key) {
            (Some(new), Some(old)) if new.same_as(&old) => {
                Err("QAMILLE_SYNC_KEY e QAMILLE_SYNC_KEY_OLD sono uguali".to_owned())
            }
            (Some(new), Some(old)) => runtime
                .block_on(db.blocking(move |conn| {
                    crate::sync::store::rekey(conn, &old, &new).map_err(ServerError::from)
                }))
                .map(|n| {
                    format!("{n} elementi ricifrati con la nuova chiave. Togli QAMILLE_SYNC_KEY_OLD.")
                })
                .map_err(|e| format!("{e}")),
            _ => Err("servono QAMILLE_SYNC_KEY (la nuova) e QAMILLE_SYNC_KEY_OLD (la vecchia)".to_owned()),
        },
        SyncCommand::AfterRestore => runtime
            .block_on(db.blocking(|conn| crate::sync::store::after_restore(conn).map_err(ServerError::from)))
            .map(|n| format!("Nuova epoca di sincronizzazione per {n} utenti"))
            .map_err(|e| format!("{e}")),
        SyncCommand::Reset { .. } => runtime
            .block_on(db.blocking(|conn| crate::sync::store::reset(conn).map_err(ServerError::from)))
            .map(|n| format!("{n} righe cancellate"))
            .map_err(|e| format!("{e}")),
        SyncCommand::Keygen => unreachable!("gestito sopra"),
    };
    match result {
        Ok(message) => {
            eprintln!("{message}");
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("errore: {message}");
            ExitCode::from(EXIT_FAILURE)
        }
    }
}
