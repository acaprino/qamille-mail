//! Configurazione del server: `server.toml` facoltativo e variabili `QAMILLE_*`,
//! che vincono sul file. I segreti stanno solo nell'ambiente.

use std::collections::HashSet;
use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};

use serde::Deserialize;

use qamille_compute::download::{HfToken, Mirror};

use crate::llm::{ApiKey, BUILTIN_TIMEOUT_SECS, LlmConfig, LlmProvider};
use crate::sync::{SyncConfig, SyncKey};

/// Nome del file di configurazione dentro la cartella dei dati.
pub const CONFIG_FILE_NAME: &str = "server.toml";
/// Lunghezza minima della chiave amministratore, in caratteri.
pub const ADMIN_KEY_MIN_CHARS: usize = 32;
/// Numero minimo di caratteri distinti della chiave amministratore.
pub const ADMIN_KEY_MIN_DISTINCT: usize = 8;

/// Edizione dichiarata dal server (`QAMILLE_EDITION`), solo informativa.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edition {
    SelfHosted,
    Professional,
}

impl Edition {
    pub fn as_str(self) -> &'static str {
        match self {
            Edition::SelfHosted => "self-hosted",
            Edition::Professional => "professional",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "self-hosted" => Some(Edition::SelfHosted),
            "professional" => Some(Edition::Professional),
            _ => None,
        }
    }
}

/// Una chiave che non sembra casuale: un carattere che torna troppo (più di un terzo), una
/// sequenza di sei caratteri uguali, un blocco di otto caratteri o meno ripetuto, o troppo
/// poca entropia per carattere (meno di 3 bit: una chiave generata, per esempio `openssl rand
/// -hex 32`, ne ha quasi 4). Non è una prova di casualità, ma ferma le chiavi scritte a mano.
fn looks_guessable(value: &str) -> bool {
    let chars: Vec<char> = value.chars().collect();
    let total = chars.len() as f64;
    let mut counts: std::collections::HashMap<char, usize> = std::collections::HashMap::new();
    for c in &chars {
        *counts.entry(*c).or_default() += 1;
    }
    let most = counts.values().copied().max().unwrap_or(0) as f64;
    let entropy: f64 = counts
        .values()
        .map(|&n| {
            let p = n as f64 / total;
            -p * p.log2()
        })
        .sum();
    let longest_run = chars
        .chunk_by(|a, b| a == b)
        .map(<[char]>::len)
        .max()
        .unwrap_or(0);
    let repeated_block = (1..=8).any(|period| {
        chars.len() > period && chars.iter().enumerate().all(|(i, c)| *c == chars[i % period])
    });
    most > total / 3.0 || longest_run >= 6 || repeated_block || entropy < 3.0
}

/// La chiave amministratore. Il `Debug` non la stampa mai.
#[derive(Clone)]
pub struct AdminKey(String);

impl AdminKey {
    /// Valida la chiave: almeno 32 caratteri e almeno 8 caratteri distinti.
    pub fn new(value: &str) -> Result<Self, ConfigError> {
        let chars = value.chars().count();
        let distinct = value.chars().collect::<HashSet<_>>().len();
        if chars < ADMIN_KEY_MIN_CHARS || distinct < ADMIN_KEY_MIN_DISTINCT || looks_guessable(value) {
            return Err(ConfigError::WeakAdminKey);
        }
        Ok(AdminKey(value.to_owned()))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for AdminKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AdminKey(<nascosta>)")
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub bind: SocketAddr,
    /// Vuoto: nessun ascolto amministrativo.
    pub admin_bind: Option<SocketAddr>,
    pub data_dir: PathBuf,
    /// 0 = nessun limite.
    pub default_max_devices: u32,
    pub trusted_proxies: Vec<IpAddr>,
    pub login_max_failures_per_origin: u32,
    pub login_failure_window_secs: u64,
    pub admin_max_failures: u32,
    pub admin_failure_window_secs: u64,
    pub temporary_password_days: u32,
    /// Giorni senza uso dopo i quali la chiave di un dispositivo non vale più (si accede di
    /// nuovo). 0 = mai.
    pub device_inactive_days: u32,
    /// Solo da `QAMILLE_ADMIN_KEY`. Vuota: nessun ascolto amministrativo.
    pub admin_key: Option<AdminKey>,
    pub edition: Edition,
    /// Indirizzi da cui l'ascolto amministrativo accetta connessioni in chiaro
    /// oltre al loopback (per esempio il gateway della rete Docker, da cui arriva
    /// la porta pubblicata su `127.0.0.1` dell'host).
    pub admin_plaintext_from: Vec<IpAddr>,
    /// Indirizzi da cui l'ascolto pubblico accetta il chiaro oltre al loopback (per
    /// esempio il gateway di Docker, quando il server gira sullo stesso computer).
    /// Solo indirizzi di loopback o privati.
    pub plaintext_from: Vec<IpAddr>,
    /// Modelli di calcolo (`embeddinggemma-2/`); vuoto: `<data_dir>/models`.
    pub models_dir: Option<PathBuf>,
    /// Modelli ggml di whisper.cpp; vuoto: `<models>/speech`.
    pub speech_models_dir: Option<PathBuf>,
    /// Trascrizioni insieme; vuoto: i core della macchina.
    pub transcription_parallelism: Option<usize>,
    /// Richieste di calcolo in volo insieme per utente, su tutti i suoi dispositivi (D2, A2: 2).
    pub compute_max_in_flight_per_user: u32,
    /// Chiavi di dispositivo sbagliate per origine al minuto sulle rotte di calcolo, prima di
    /// `429 rate_limited` (ogni chiave sbagliata costa una query al database).
    pub compute_max_auth_failures_per_origin: u32,
    /// Gateway LLM (spento di base).
    pub llm: LlmConfig,
    /// Specchio di Hugging Face e GitHub per i modelli e i motori (`models_mirror_url`).
    pub models_mirror_url: Option<String>,
    /// Il token di Hugging Face, solo da `QAMILLE_HF_TOKEN` (mai dal file): solo per le voci
    /// del catalogo che lo dichiarano, solo verso `huggingface.co`.
    pub hf_token: Option<HfToken>,
    /// Sincronizzazione delle caselle (spenta senza `QAMILLE_SYNC_KEY`).
    pub sync: SyncConfig,
}

impl Config {
    /// Configurazione predefinita con la cartella dei dati indicata.
    pub fn with_data_dir(data_dir: impl Into<PathBuf>) -> Self {
        Config {
            bind: SocketAddr::from(([0, 0, 0, 0], 8787)),
            admin_bind: None,
            data_dir: data_dir.into(),
            default_max_devices: 3,
            trusted_proxies: Vec::new(),
            login_max_failures_per_origin: 30,
            login_failure_window_secs: 600,
            // Alta di proposito: la chiave (almeno 32 caratteri) non si indovina per
            // tentativi, e una soglia bassa permetterebbe a chiunque raggiunga
            // l'ascolto (nel compose, ogni utente dell'host arriva dallo stesso
            // gateway) di chiudere fuori l'amministratore.
            admin_max_failures: 300,
            admin_failure_window_secs: 60,
            temporary_password_days: 7,
            device_inactive_days: 180,
            admin_key: None,
            edition: Edition::SelfHosted,
            admin_plaintext_from: Vec::new(),
            plaintext_from: Vec::new(),
            models_dir: None,
            speech_models_dir: None,
            transcription_parallelism: None,
            compute_max_in_flight_per_user: 2,
            compute_max_auth_failures_per_origin: 120,
            llm: LlmConfig::default(),
            models_mirror_url: None,
            hf_token: None,
            sync: SyncConfig::default(),
        }
    }

    /// Lo specchio dei modelli, già validato (`validate` lo controlla).
    pub fn models_mirror(&self) -> Option<Mirror> {
        Mirror::parse(self.models_mirror_url.as_deref()?).ok()
    }

    /// La cartella dei modelli di calcolo.
    pub fn models_dir(&self) -> PathBuf {
        self.models_dir.clone().unwrap_or_else(|| self.data_dir.join("models"))
    }

    /// La cartella dei modelli di trascrizione.
    pub fn speech_models_dir(&self) -> PathBuf {
        self.speech_models_dir.clone().unwrap_or_else(|| self.models_dir().join("speech"))
    }

    /// Il listener amministrativo si apre solo con indirizzo e chiave.
    pub fn admin_listener(&self) -> Option<(SocketAddr, &AdminKey)> {
        Some((self.admin_bind?, self.admin_key.as_ref()?))
    }

    pub fn database_path(&self) -> PathBuf {
        self.data_dir.join("server.db")
    }

    /// Carica dal processo: `--config` se indicato, altrimenti `<data_dir>/server.toml`
    /// se esiste; le variabili d'ambiente vincono sul file.
    pub fn load(config_path: Option<&Path>) -> Result<Self, ConfigError> {
        Self::load_with(config_path, env_var)
    }

    /// Come [`Config::load`], con l'ambiente iniettato (per i test). Il core rifiuta ogni
    /// chiave del file che non è sua, come una chiave sconosciuta.
    pub fn load_with(
        config_path: Option<&Path>,
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, ConfigError> {
        let (config, rest, source) = Self::load_inner(config_path, env)?;
        if let Some(key) = rest.keys().next() {
            return Err(ConfigError::Parse {
                path: source.unwrap_or_default(),
                reason: format!("unknown field `{key}`"),
            });
        }
        Ok(config)
    }

    /// Carica le chiavi del core e restituisce la tabella delle altre chiavi del file, per
    /// chi ci sta sopra (l'enterprise): la loro validità non è affare del core. Le variabili
    /// d'ambiente non sue le ignora, vedi [`Config::ignored_env`].
    pub fn load_split(
        config_path: Option<&Path>,
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<(Self, toml::Table), ConfigError> {
        let (config, rest, _) = Self::load_inner(config_path, env)?;
        Ok((config, rest))
    }

    /// I nomi delle variabili d'ambiente dell'enterprise.
    pub const ENTERPRISE_ENV: &'static [&'static str] = &[
        "QAMILLE_SMTP_HOST",
        "QAMILLE_SMTP_PORT",
        "QAMILLE_SMTP_USERNAME",
        "QAMILLE_SMTP_PASSWORD",
        "QAMILLE_SMTP_FROM",
        "QAMILLE_SMTP_STARTTLS",
        "QAMILLE_SIGNUP",
        "QAMILLE_SIGNUP_MAX_PER_ORIGIN",
        "QAMILLE_SIGNUP_CONFIRM_MAX_PER_ORIGIN",
        "QAMILLE_PASSWORD_RESET_MAX_PER_ORIGIN",
        "QAMILLE_PASSWORD_RESET_CONFIRM_MAX_PER_ORIGIN",
        "QAMILLE_PUBLIC_URL",
        "QAMILLE_GOOGLE_CLIENT_ID",
        "QAMILLE_GOOGLE_CLIENT_SECRET",
        "QAMILLE_GOOGLE_ALLOWED_DOMAINS",
        "QAMILLE_GOOGLE_AUTH_URL",
        "QAMILLE_GOOGLE_TOKEN_URL",
        "QAMILLE_BILLING_PROVIDER",
        "QAMILLE_TRIAL_DAYS",
        "QAMILLE_BILLING_GRACE_DAYS",
        "QAMILLE_BILLING_CURRENCY",
        "QAMILLE_ALLOW_FAKE_BILLING",
        "QAMILLE_BILLING_WEBHOOK_SECRET",
    ];

    /// La riga di log all'avvio quando `plaintext_from` non e' vuoto: il chiaro e' ammesso solo per
    /// una porta pubblicata su loopback.
    pub fn plaintext_notice(&self) -> Option<String> {
        (!self.plaintext_from.is_empty()).then(|| {
            "plaintext_from e' impostato: il server accetta il chiaro da quegli indirizzi; va usato solo con una porta pubblicata su loopback (127.0.0.1)".to_owned()
        })
    }

    /// L'avviso all'avvio quando `trusted_proxies` è vuoto e le origini rischiano di coincidere:
    /// dietro un proxy o un gateway ogni richiesta arriva dal suo indirizzo, e tutti gli utenti
    /// dividono gli stessi limiti per origine (e, sul loopback, la connessione conta come sicura).
    pub fn origin_notice(&self) -> Option<String> {
        if !self.trusted_proxies.is_empty() {
            return None;
        }
        Some(if self.bind.ip().is_loopback() {
            "trusted_proxies è vuoto e l'ascolto è sul loopback: se un proxy sullo stesso computer inoltra le richieste, tutte arrivano da 127.0.0.1, dividono gli stessi limiti per origine e valgono come connessioni sicure; indica il proxy in trusted_proxies".to_owned()
        } else {
            "trusted_proxies è vuoto e l'ascolto non è sul loopback: se il server sta dietro un proxy o un gateway, tutte le richieste arrivano dal suo indirizzo e dividono gli stessi limiti per origine (X-Forwarded-For e X-Forwarded-Proto si ignorano); indica il proxy in trusted_proxies, oppure ignora l'avviso se i client si collegano direttamente".to_owned()
        })
    }

    /// Le variabili dell'enterprise impostate (non vuote) in questo ambiente: il core non le
    /// legge e non può rifiutarle, ma all'avvio le segnala nel log.
    pub fn ignored_env(env: impl Fn(&str) -> Option<String>) -> Vec<&'static str> {
        Self::ENTERPRISE_ENV
            .iter()
            .copied()
            .filter(|name| env(name).is_some_and(|value| !value.trim().is_empty()))
            .collect()
    }

    fn load_inner(
        config_path: Option<&Path>,
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<(Self, toml::Table, Option<PathBuf>), ConfigError> {
        let env_data_dir = env("QAMILLE_DATA_DIR").filter(|v| !v.trim().is_empty());
        let loaded = match config_path {
            Some(path) => Some((path.to_path_buf(), read_file(path)?)),
            None => {
                let dir = env_data_dir
                    .as_deref()
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("data"));
                let path = dir.join(CONFIG_FILE_NAME);
                if path.is_file() {
                    let file = read_file(&path)?;
                    Some((path, file))
                } else {
                    None
                }
            }
        };
        let (source, (file, rest)) = match loaded {
            Some((path, parts)) => (Some(path), parts),
            None => (None, Default::default()),
        };

        let mut config = Config::with_data_dir("data");

        // Prima il file, poi l'ambiente: l'ambiente vince.
        file.llm_into(&mut config.llm)?;
        let timeout_given = file.llm_timeout_secs.is_some();
        if let Some(v) = &file.models_mirror_url {
            config.models_mirror_url = non_empty(v);
        }
        if let Some(v) = file.bind {
            config.bind = parse_socket("bind", &v)?;
        }
        if let Some(v) = file.admin_bind {
            config.admin_bind = parse_optional_socket("admin_bind", &v)?;
        }
        if let Some(v) = file.data_dir {
            config.data_dir = PathBuf::from(v);
        }
        if let Some(v) = file.default_max_devices {
            config.default_max_devices = v;
        }
        if let Some(v) = file.trusted_proxies {
            config.trusted_proxies = v.iter().map(|s| parse_ip(s)).collect::<Result<_, _>>()?;
        }
        if let Some(v) = file.login_max_failures_per_origin {
            config.login_max_failures_per_origin = v;
        }
        if let Some(v) = file.login_failure_window_secs {
            config.login_failure_window_secs = v;
        }
        if let Some(v) = file.admin_max_failures {
            config.admin_max_failures = v;
        }
        if let Some(v) = file.admin_failure_window_secs {
            config.admin_failure_window_secs = v;
        }
        if let Some(v) = file.temporary_password_days {
            config.temporary_password_days = v;
        }
        if let Some(v) = file.device_inactive_days {
            config.device_inactive_days = v;
        }
        if let Some(v) = file.admin_plaintext_from {
            config.admin_plaintext_from =
                v.iter().map(|s| parse_ip(s)).collect::<Result<_, _>>()?;
        }
        if let Some(v) = file.plaintext_from {
            config.plaintext_from = v.iter().map(|s| parse_ip(s)).collect::<Result<_, _>>()?;
        }
        if let Some(v) = file.models_dir {
            config.models_dir = non_empty(&v).map(PathBuf::from);
        }
        if let Some(v) = file.speech_models_dir {
            config.speech_models_dir = non_empty(&v).map(PathBuf::from);
        }
        if let Some(v) = file.transcription_parallelism {
            config.transcription_parallelism = Some(v);
        }
        if let Some(v) = file.compute_max_in_flight_per_user {
            config.compute_max_in_flight_per_user = v;
        }
        if let Some(v) = file.sync_max_items {
            config.sync.max_items = v;
        }
        if let Some(v) = file.sync_max_item_bytes {
            config.sync.max_item_bytes = v;
        }
        if let Some(v) = file.compute_max_auth_failures_per_origin {
            config.compute_max_auth_failures_per_origin = v;
        }

        if let Some(v) = env("QAMILLE_BIND") {
            config.bind = parse_socket("QAMILLE_BIND", &v)?;
        }
        if let Some(v) = env("QAMILLE_ADMIN_BIND") {
            config.admin_bind = parse_optional_socket("QAMILLE_ADMIN_BIND", &v)?;
        }
        if let Some(v) = env_data_dir {
            config.data_dir = PathBuf::from(v);
        }
        if let Some(v) = env("QAMILLE_DEFAULT_MAX_DEVICES") {
            config.default_max_devices = parse_number("QAMILLE_DEFAULT_MAX_DEVICES", &v)?;
        }
        if let Some(v) = env("QAMILLE_TRUSTED_PROXIES") {
            config.trusted_proxies = parse_ip_list(&v)?;
        }
        if let Some(v) = env("QAMILLE_LOGIN_MAX_FAILURES_PER_ORIGIN") {
            config.login_max_failures_per_origin =
                parse_number("QAMILLE_LOGIN_MAX_FAILURES_PER_ORIGIN", &v)?;
        }
        if let Some(v) = env("QAMILLE_LOGIN_FAILURE_WINDOW_SECS") {
            config.login_failure_window_secs =
                parse_number("QAMILLE_LOGIN_FAILURE_WINDOW_SECS", &v)?;
        }
        if let Some(v) = env("QAMILLE_ADMIN_MAX_FAILURES") {
            config.admin_max_failures = parse_number("QAMILLE_ADMIN_MAX_FAILURES", &v)?;
        }
        if let Some(v) = env("QAMILLE_ADMIN_FAILURE_WINDOW_SECS") {
            config.admin_failure_window_secs =
                parse_number("QAMILLE_ADMIN_FAILURE_WINDOW_SECS", &v)?;
        }
        if let Some(v) = env("QAMILLE_TEMPORARY_PASSWORD_DAYS") {
            config.temporary_password_days = parse_number("QAMILLE_TEMPORARY_PASSWORD_DAYS", &v)?;
        }
        if let Some(v) = env("QAMILLE_DEVICE_INACTIVE_DAYS") {
            config.device_inactive_days = parse_number("QAMILLE_DEVICE_INACTIVE_DAYS", &v)?;
        }
        if let Some(v) = env("QAMILLE_EDITION").filter(|v| !v.trim().is_empty()) {
            config.edition = Edition::parse(&v).ok_or(ConfigError::InvalidEdition)?;
        }
        if let Some(v) = env("QAMILLE_ADMIN_KEY").filter(|v| !v.is_empty()) {
            config.admin_key = Some(AdminKey::new(&v)?);
        }
        if let Some(v) = env("QAMILLE_ADMIN_PLAINTEXT_FROM") {
            config.admin_plaintext_from = parse_ip_list(&v)?;
        }
        if let Some(v) = env("QAMILLE_PLAINTEXT_FROM") {
            config.plaintext_from = parse_ip_list(&v)?;
        }
        // Le variabili vuote (come le lascia un `.env` del compose) non contano.
        let set = |name: &str| env(name).filter(|v| !v.trim().is_empty());

        if let Some(v) = set("QAMILLE_MODELS_DIR") {
            config.models_dir = Some(PathBuf::from(v.trim()));
        }
        if let Some(v) = set("QAMILLE_SPEECH_MODELS_DIR") {
            config.speech_models_dir = Some(PathBuf::from(v.trim()));
        }
        if let Some(v) = set("QAMILLE_TRANSCRIPTION_PARALLELISM") {
            config.transcription_parallelism =
                Some(parse_number("QAMILLE_TRANSCRIPTION_PARALLELISM", &v)?);
        }
        if let Some(v) = set("QAMILLE_COMPUTE_MAX_AUTH_FAILURES_PER_ORIGIN") {
            config.compute_max_auth_failures_per_origin =
                parse_number("QAMILLE_COMPUTE_MAX_AUTH_FAILURES_PER_ORIGIN", &v)?;
        }
        if let Some(v) = set("QAMILLE_COMPUTE_MAX_IN_FLIGHT_PER_USER") {
            config.compute_max_in_flight_per_user =
                parse_number("QAMILLE_COMPUTE_MAX_IN_FLIGHT_PER_USER", &v)?;
        }
        llm_from_env(&mut config.llm, &set)?;
        if let Some(v) = set("QAMILLE_MODELS_MIRROR_URL") {
            config.models_mirror_url = non_empty(&v);
        }
        config.hf_token = set("QAMILLE_HF_TOKEN").and_then(|v| HfToken::new(&v));
        sync_from_env(&mut config.sync, &set)?;
        if config.llm.provider == LlmProvider::Builtin && !timeout_given && set("QAMILLE_LLM_TIMEOUT_SECS").is_none() {
            config.llm.timeout_secs = BUILTIN_TIMEOUT_SECS;
        }

        config.validate()?;
        Ok((config, rest, source))
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.login_max_failures_per_origin == 0 {
            return Err(ConfigError::Invalid {
                key: "login_max_failures_per_origin",
                reason: "deve essere almeno 1",
            });
        }
        if self.login_failure_window_secs == 0 {
            return Err(ConfigError::Invalid {
                key: "login_failure_window_secs",
                reason: "deve essere almeno 1",
            });
        }
        if self.admin_max_failures == 0 {
            return Err(ConfigError::Invalid {
                key: "admin_max_failures",
                reason: "deve essere almeno 1",
            });
        }
        if self.admin_failure_window_secs == 0 {
            return Err(ConfigError::Invalid {
                key: "admin_failure_window_secs",
                reason: "deve essere almeno 1",
            });
        }
        if self.temporary_password_days == 0 {
            return Err(ConfigError::Invalid {
                key: "temporary_password_days",
                reason: "deve essere almeno 1",
            });
        }
        if let Some(admin) = self.admin_bind
            && admin == self.bind
        {
            return Err(ConfigError::Invalid {
                key: "admin_bind",
                reason: "deve essere diverso da bind",
            });
        }
        if self.compute_max_auth_failures_per_origin == 0 {
            return Err(ConfigError::Invalid {
                key: "compute_max_auth_failures_per_origin",
                reason: "deve essere almeno 1",
            });
        }
        if self.compute_max_in_flight_per_user == 0 {
            return Err(ConfigError::Invalid {
                key: "compute_max_in_flight_per_user",
                reason: "deve essere almeno 1",
            });
        }
        if self.transcription_parallelism == Some(0) {
            return Err(ConfigError::Invalid {
                key: "transcription_parallelism",
                reason: "deve essere almeno 1",
            });
        }
        self.validate_admin_plaintext()?;
        self.validate_plaintext()?;
        self.llm.validate()?;
        self.sync.validate()?;
        if let Some(url) = &self.models_mirror_url {
            Mirror::parse(url).map_err(|reason| ConfigError::Invalid { key: "models_mirror_url", reason })?;
        }
        Ok(())
    }

    /// `plaintext_from` accetta solo indirizzi di loopback o privati: uno pubblico
    /// farebbe entrare chiunque in chiaro. Non chiede un `bind` privato, perché nel
    /// container il server ascolta su `0.0.0.0`.
    fn validate_plaintext(&self) -> Result<(), ConfigError> {
        if self.plaintext_from.iter().all(|ip| is_local_or_private(*ip)) {
            return Ok(());
        }
        Err(ConfigError::Invalid {
            key: "plaintext_from",
            reason: "accetta solo indirizzi di loopback o privati",
        })
    }

    /// Il chiaro sull'ascolto amministrativo si accetta solo se quell'ascolto è
    /// legato a un indirizzo di loopback o privato (mai `0.0.0.0` o un indirizzo
    /// pubblico) e se le sorgenti ammesse sono anch'esse di loopback o private.
    /// Altrimenti il server non parte: la chiave amministratore viaggerebbe in
    /// chiaro su una rete qualsiasi.
    fn validate_admin_plaintext(&self) -> Result<(), ConfigError> {
        if self.admin_plaintext_from.is_empty() {
            return Ok(());
        }
        let bind_ok = self
            .admin_bind
            .is_some_and(|addr| is_local_or_private(addr.ip()));
        if !bind_ok {
            return Err(ConfigError::Invalid {
                key: "admin_plaintext_from",
                reason: "richiede admin_bind su un indirizzo di loopback o privato (non 0.0.0.0 né pubblico)",
            });
        }
        if !self
            .admin_plaintext_from
            .iter()
            .all(|ip| is_local_or_private(*ip))
        {
            return Err(ConfigError::Invalid {
                key: "admin_plaintext_from",
                reason: "accetta solo indirizzi di loopback o privati",
            });
        }
        Ok(())
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    bind: Option<String>,
    admin_bind: Option<String>,
    data_dir: Option<String>,
    default_max_devices: Option<u32>,
    trusted_proxies: Option<Vec<String>>,
    login_max_failures_per_origin: Option<u32>,
    login_failure_window_secs: Option<u64>,
    admin_max_failures: Option<u32>,
    admin_failure_window_secs: Option<u64>,
    temporary_password_days: Option<u32>,
    device_inactive_days: Option<u32>,
    admin_plaintext_from: Option<Vec<String>>,
    plaintext_from: Option<Vec<String>>,
    models_dir: Option<String>,
    speech_models_dir: Option<String>,
    transcription_parallelism: Option<usize>,
    compute_max_in_flight_per_user: Option<u32>,
    compute_max_auth_failures_per_origin: Option<u32>,
    sync_max_items: Option<u32>,
    sync_max_item_bytes: Option<usize>,
    llm_provider: Option<String>,
    llm_base_url: Option<String>,
    llm_model: Option<String>,
    llm_allowed_models: Option<Vec<String>>,
    llm_max_output_tokens: Option<u32>,
    llm_context_tokens: Option<u32>,
    llm_max_request_bytes: Option<usize>,
    llm_max_tokens_per_day_per_user: Option<u64>,
    llm_timeout_secs: Option<u64>,
    llm_max_in_flight_per_user: Option<u32>,
    llm_max_in_flight_total: Option<u32>,
    llm_label: Option<String>,
    llm_external: Option<bool>,
    llm_builtin_model: Option<String>,
    llm_builtin_context: Option<u32>,
    llm_builtin_idle_secs: Option<u64>,
    llm_builtin_slots: Option<u32>,
    llm_builtin_program: Option<String>,
    llm_accept_license: Option<String>,
    models_mirror_url: Option<String>,
}

impl FileConfig {
    fn llm_into(&self, llm: &mut LlmConfig) -> Result<(), ConfigError> {
        if let Some(v) = &self.llm_provider {
            llm.provider = LlmProvider::parse(v)?;
        }
        if let Some(v) = &self.llm_base_url {
            llm.base_url = non_empty(v);
        }
        if let Some(v) = &self.llm_model {
            llm.model = non_empty(v);
        }
        if let Some(v) = &self.llm_allowed_models {
            llm.allowed_models = v.iter().map(|s| s.trim().to_owned()).collect();
        }
        if let Some(v) = self.llm_max_output_tokens {
            llm.max_output_tokens = v;
        }
        if let Some(v) = self.llm_context_tokens {
            llm.context_tokens = Some(v);
        }
        if let Some(v) = self.llm_max_request_bytes {
            llm.max_request_bytes = v;
        }
        if let Some(v) = self.llm_max_tokens_per_day_per_user {
            llm.max_tokens_per_day_per_user = v;
        }
        if let Some(v) = self.llm_timeout_secs {
            llm.timeout_secs = v;
        }
        if let Some(v) = self.llm_max_in_flight_per_user {
            llm.max_in_flight_per_user = v;
        }
        if let Some(v) = self.llm_max_in_flight_total {
            llm.max_in_flight_total = v;
        }
        // Un'etichetta vuota nel file e' un errore (la valida `LlmConfig::validate`).
        if let Some(v) = &self.llm_label {
            llm.label = Some(v.trim().to_owned());
        }
        if let Some(v) = self.llm_external {
            llm.external = Some(v);
        }
        if let Some(v) = &self.llm_builtin_model {
            llm.builtin_model = non_empty(v);
        }
        if let Some(v) = self.llm_builtin_context {
            llm.builtin_context = v;
        }
        if let Some(v) = self.llm_builtin_idle_secs {
            llm.builtin_idle_secs = v;
        }
        if let Some(v) = self.llm_builtin_slots {
            llm.builtin_slots = v;
        }
        if let Some(v) = &self.llm_builtin_program {
            llm.builtin_program = non_empty(v).map(PathBuf::from);
        }
        if let Some(v) = &self.llm_accept_license {
            llm.accept_license = non_empty(v);
        }
        Ok(())
    }
}

/// Le variabili `QAMILLE_LLM_*`; quelle vuote non contano. La chiave del provider sta solo qui.
/// Le variabili della sincronizzazione. Le chiavi del server stanno solo qui: un formato
/// sbagliato ferma il caricamento, e il motivo non riporta mai il valore.
fn sync_from_env(
    sync: &mut SyncConfig,
    set: &impl Fn(&str) -> Option<String>,
) -> Result<(), ConfigError> {
    for (name, slot) in [
        ("QAMILLE_SYNC_KEY", &mut sync.key),
        ("QAMILLE_SYNC_KEY_OLD", &mut sync.old_key),
    ] {
        if let Some(v) = set(name) {
            *slot = Some(
                SyncKey::parse(&v).map_err(|e| ConfigError::Invalid { key: name, reason: e.reason() })?,
            );
        }
    }
    if let Some(v) = set("QAMILLE_SYNC_MAX_ITEMS") {
        sync.max_items = parse_number("QAMILLE_SYNC_MAX_ITEMS", &v)?;
    }
    if let Some(v) = set("QAMILLE_SYNC_MAX_ITEM_BYTES") {
        sync.max_item_bytes = parse_number("QAMILLE_SYNC_MAX_ITEM_BYTES", &v)?;
    }
    Ok(())
}

fn llm_from_env(
    llm: &mut LlmConfig,
    set: &impl Fn(&str) -> Option<String>,
) -> Result<(), ConfigError> {
    if let Some(v) = set("QAMILLE_LLM_PROVIDER") {
        llm.provider = LlmProvider::parse(&v)?;
    }
    if let Some(v) = set("QAMILLE_LLM_BASE_URL") {
        llm.base_url = non_empty(&v);
    }
    if let Some(v) = set("QAMILLE_LLM_MODEL") {
        llm.model = non_empty(&v);
    }
    if let Some(v) = set("QAMILLE_LLM_ALLOWED_MODELS") {
        llm.allowed_models = v.split(',').map(|s| s.trim().to_owned()).collect();
    }
    if let Some(v) = set("QAMILLE_LLM_MAX_OUTPUT_TOKENS") {
        llm.max_output_tokens = parse_number("QAMILLE_LLM_MAX_OUTPUT_TOKENS", &v)?;
    }
    if let Some(v) = set("QAMILLE_LLM_CONTEXT_TOKENS") {
        llm.context_tokens = Some(parse_number("QAMILLE_LLM_CONTEXT_TOKENS", &v)?);
    }
    if let Some(v) = set("QAMILLE_LLM_MAX_REQUEST_BYTES") {
        llm.max_request_bytes = parse_number("QAMILLE_LLM_MAX_REQUEST_BYTES", &v)?;
    }
    if let Some(v) = set("QAMILLE_LLM_MAX_TOKENS_PER_DAY_PER_USER") {
        llm.max_tokens_per_day_per_user =
            parse_number("QAMILLE_LLM_MAX_TOKENS_PER_DAY_PER_USER", &v)?;
    }
    if let Some(v) = set("QAMILLE_LLM_TIMEOUT_SECS") {
        llm.timeout_secs = parse_number("QAMILLE_LLM_TIMEOUT_SECS", &v)?;
    }
    if let Some(v) = set("QAMILLE_LLM_MAX_IN_FLIGHT_PER_USER") {
        llm.max_in_flight_per_user = parse_number("QAMILLE_LLM_MAX_IN_FLIGHT_PER_USER", &v)?;
    }
    if let Some(v) = set("QAMILLE_LLM_MAX_IN_FLIGHT_TOTAL") {
        llm.max_in_flight_total = parse_number("QAMILLE_LLM_MAX_IN_FLIGHT_TOTAL", &v)?;
    }
    if let Some(v) = set("QAMILLE_LLM_LABEL") {
        llm.label = Some(v.trim().to_owned());
    }
    if let Some(v) = set("QAMILLE_LLM_EXTERNAL") {
        llm.external = Some(parse_flag("QAMILLE_LLM_EXTERNAL", &v)?);
    }
    if let Some(v) = set("QAMILLE_LLM_BUILTIN_MODEL") {
        llm.builtin_model = non_empty(&v);
    }
    if let Some(v) = set("QAMILLE_LLM_BUILTIN_CONTEXT") {
        llm.builtin_context = parse_number("QAMILLE_LLM_BUILTIN_CONTEXT", &v)?;
    }
    if let Some(v) = set("QAMILLE_LLM_BUILTIN_IDLE_SECS") {
        llm.builtin_idle_secs = parse_number("QAMILLE_LLM_BUILTIN_IDLE_SECS", &v)?;
    }
    if let Some(v) = set("QAMILLE_LLM_BUILTIN_SLOTS") {
        llm.builtin_slots = parse_number("QAMILLE_LLM_BUILTIN_SLOTS", &v)?;
    }
    if let Some(v) = set("QAMILLE_LLM_BUILTIN_PROGRAM") {
        llm.builtin_program = non_empty(&v).map(PathBuf::from);
    }
    if let Some(v) = set("QAMILLE_LLM_ACCEPT_LICENSE") {
        llm.accept_license = non_empty(&v);
    }
    if let Some(v) = set("QAMILLE_LLM_API_KEY") {
        llm.api_key = ApiKey::new(&v);
    }
    Ok(())
}

/// Legge `QAMILLE_<CHIAVE>` dal processo; se manca o è vuota, usa `QEMILLE_<CHIAVE>` e poi
/// `SPARKLONE_<CHIAVE>`, i nomi di prima dei cambi di nome (installazioni e `.env` già configurati).
pub fn env_var(name: &str) -> Option<String> {
    with_legacy_name(name, |name| std::env::var(name).ok())
}

/// La regola di [`env_var`] su un ambiente qualsiasi (per i test).
pub fn with_legacy_name(
    name: &str,
    env: impl Fn(&str) -> Option<String>,
) -> Option<String> {
    let current = env(name).filter(|value| !value.is_empty());
    let key = name.strip_prefix("QAMILLE_");
    current.or_else(|| {
        ["QEMILLE_", "SPARKLONE_"]
            .iter()
            .find_map(|prefix| env(&format!("{prefix}{}", key?)).filter(|value| !value.is_empty()))
    })
}

/// Le chiavi del file che appartengono al core.
const CORE_FILE_KEYS: &[&str] = &[
    "bind",
    "admin_bind",
    "data_dir",
    "default_max_devices",
    "trusted_proxies",
    "login_max_failures_per_origin",
    "login_failure_window_secs",
    "admin_max_failures",
    "admin_failure_window_secs",
    "temporary_password_days",
    "device_inactive_days",
    "admin_plaintext_from",
    "plaintext_from",
    "models_dir",
    "speech_models_dir",
    "transcription_parallelism",
    "compute_max_in_flight_per_user",
    "compute_max_auth_failures_per_origin",
    "sync_max_items",
    "sync_max_item_bytes",
    "llm_provider",
    "llm_base_url",
    "llm_model",
    "llm_allowed_models",
    "llm_max_output_tokens",
    "llm_context_tokens",
    "llm_max_request_bytes",
    "llm_max_tokens_per_day_per_user",
    "llm_timeout_secs",
    "llm_max_in_flight_per_user",
    "llm_max_in_flight_total",
    "llm_label",
    "llm_external",
    "llm_builtin_model",
    "llm_builtin_context",
    "llm_builtin_idle_secs",
    "llm_builtin_slots",
    "llm_builtin_program",
    "llm_accept_license",
    "models_mirror_url",
];

/// Legge il file: le chiavi del core, e la tabella di tutte le altre.
fn read_file(path: &Path) -> Result<(FileConfig, toml::Table), ConfigError> {
    let text = std::fs::read_to_string(path).map_err(|e| ConfigError::Read {
        path: path.to_path_buf(),
        reason: e.to_string(),
    })?;
    let parse_error = |e: toml::de::Error| ConfigError::Parse {
        path: path.to_path_buf(),
        reason: e.message().to_owned(),
    };
    let mut table: toml::Table = toml::from_str(&text).map_err(parse_error)?;
    let mut core = toml::Table::new();
    for key in CORE_FILE_KEYS {
        if let Some(value) = table.remove(*key) {
            core.insert((*key).to_owned(), value);
        }
    }
    let file = core.try_into().map_err(parse_error)?;
    Ok((file, table))
}

fn parse_socket(key: &'static str, value: &str) -> Result<SocketAddr, ConfigError> {
    value.trim().parse().map_err(|_| ConfigError::Invalid {
        key,
        reason: "indirizzo non valido (atteso ip:porta)",
    })
}

fn parse_optional_socket(
    key: &'static str,
    value: &str,
) -> Result<Option<SocketAddr>, ConfigError> {
    if value.trim().is_empty() {
        Ok(None)
    } else {
        parse_socket(key, value).map(Some)
    }
}

pub fn parse_number<T: std::str::FromStr>(key: &'static str, value: &str) -> Result<T, ConfigError> {
    value.trim().parse().map_err(|_| ConfigError::Invalid {
        key,
        reason: "numero non valido",
    })
}

/// Loopback, privato (RFC 1918) o IPv6 locale unico (`fc00::/7`).
pub fn is_local_or_private(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private(),
        IpAddr::V6(v6) => v6.is_loopback() || (v6.segments()[0] & 0xfe00) == 0xfc00,
    }
}

pub fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

pub fn parse_flag(key: &'static str, value: &str) -> Result<bool, ConfigError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" => Ok(true),
        "0" | "false" => Ok(false),
        _ => Err(ConfigError::Invalid {
            key,
            reason: "atteso 1 o 0",
        }),
    }
}

fn parse_ip_list(value: &str) -> Result<Vec<IpAddr>, ConfigError> {
    value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(parse_ip)
        .collect()
}

fn parse_ip(value: &str) -> Result<IpAddr, ConfigError> {
    value
        .trim()
        .parse::<IpAddr>()
        .map(|ip| ip.to_canonical())
        .map_err(|_| ConfigError::InvalidTrustedProxy(value.trim().to_owned()))
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("non riesco a leggere {path}: {reason}")]
    Read { path: PathBuf, reason: String },
    #[error("{path} non è valido: {reason}")]
    Parse { path: PathBuf, reason: String },
    #[error("valore non valido per {key}: {reason}")]
    Invalid {
        key: &'static str,
        reason: &'static str,
    },
    #[error("trusted_proxies, admin_plaintext_from o plaintext_from contiene «{0}», che non è un indirizzo IP")]
    InvalidTrustedProxy(String),
    #[error(
        "QAMILLE_ADMIN_KEY è troppo debole: servono almeno {ADMIN_KEY_MIN_CHARS} caratteri, almeno {ADMIN_KEY_MIN_DISTINCT} diversi e nessuna ripetizione evidente (per esempio `openssl rand -hex 32`)"
    )]
    WeakAdminKey,
    #[error("QAMILLE_EDITION deve essere «self-hosted» o «professional»")]
    InvalidEdition,
}
