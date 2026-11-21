//! Gateway LLM: configurazione, validazione, inoltro al provider (ureq su `spawn_blocking`,
//! nessun redirect, risposte limitate) e traduzione degli errori. La chiave del provider sta
//! nell'ambiente o nelle impostazioni scelte dal pannello `/admin` (INV-32, [`SETTINGS_FILE`]) e
//! non compare mai in un `Debug`, in un log o in una risposta.

use std::collections::HashMap;
use std::fmt;
use std::io::Write;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use serde::{Deserialize, Serialize};

use crate::accounts::Accounts;
use crate::compute::{ComputeError, Usage, UsageSink};
use crate::config::{ConfigError, is_local_or_private};
use crate::llm_builtin::LlmSupervisor;

/// Predefinito di `llm_max_in_flight_total`.
pub const DEFAULT_MAX_IN_FLIGHT_TOTAL: u32 = 16;

/// Il provider a cui il server inoltra le richieste di chat.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum LlmProvider {
    /// Nessun inoltro: la rotta non esiste.
    #[default]
    None,
    /// Un servizio compatibile con l'API di OpenAI (Ollama, vLLM, llama.cpp, OpenAI).
    OpenAiCompatible,
    /// Un modello del catalogo scaricato da Hugging Face e acceso dal server come processo figlio.
    Builtin,
}

impl LlmProvider {
    pub fn parse(value: &str) -> Result<LlmProvider, ConfigError> {
        match value.trim() {
            "none" => Ok(LlmProvider::None),
            "openai-compatible" => Ok(LlmProvider::OpenAiCompatible),
            "builtin" => Ok(LlmProvider::Builtin),
            _ => Err(ConfigError::Invalid {
                key: "llm_provider",
                reason: "atteso «none», «builtin» o «openai-compatible»",
            }),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            LlmProvider::None => "none",
            LlmProvider::OpenAiCompatible => "openai-compatible",
            LlmProvider::Builtin => "builtin",
        }
    }
}

/// La chiave del provider. Il `Debug` non la stampa mai.
#[derive(Clone)]
pub struct ApiKey(String);

impl ApiKey {
    /// Spazi tolti; vuota: `None`.
    pub fn new(value: &str) -> Option<ApiKey> {
        let trimmed = value.trim();
        (!trimmed.is_empty()).then(|| ApiKey(trimmed.to_owned()))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey(<nascosta>)")
    }
}

/// La configurazione del gateway LLM.
#[derive(Clone, Debug)]
pub struct LlmConfig {
    pub provider: LlmProvider,
    pub base_url: Option<String>,
    pub model: Option<String>,
    pub allowed_models: Vec<String>,
    pub max_output_tokens: u32,
    pub context_tokens: Option<u32>,
    pub max_request_bytes: usize,
    pub timeout_secs: u64,
    pub max_in_flight_per_user: u32,
    /// Richieste in volo insieme su tutto il server, di tutti gli utenti.
    pub max_in_flight_total: u32,
    /// 0 = nessun tetto.
    pub max_tokens_per_day_per_user: u64,
    pub label: Option<String>,
    pub external: Option<bool>,
    /// Solo da `QAMILLE_LLM_API_KEY`.
    pub api_key: Option<ApiKey>,
    /// `builtin`: l'id del modello del catalogo (`llm-medio`).
    pub builtin_model: Option<String>,
    /// `builtin`: contesto di ogni slot, in token.
    pub builtin_context: u32,
    /// `builtin`: secondi senza richieste dopo i quali il figlio si ferma.
    pub builtin_idle_secs: u64,
    /// `builtin`: richieste servite insieme dal figlio (il contesto totale è contesto × slot).
    pub builtin_slots: u32,
    /// `builtin` su Linux: il percorso assoluto di `llama-server` (su Windows è ignorato).
    pub builtin_program: Option<PathBuf>,
    /// `builtin`: the licence the operator accepts (`QAMILLE_LLM_ACCEPT_LICENSE`, e.g. `gemma`);
    /// without it a model that asks for it is not downloaded by the server on its own.
    pub accept_license: Option<String>,
}

/// L'etichetta del consenso con `builtin` (ASCII, senza «;»).
pub const BUILTIN_LABEL: &str = "questo server";
/// Il tempo massimo di una chiamata con `builtin`, se non è dato.
pub const BUILTIN_TIMEOUT_SECS: u64 = 600;

impl Default for LlmConfig {
    fn default() -> Self {
        LlmConfig {
            provider: LlmProvider::None,
            base_url: None,
            model: None,
            allowed_models: Vec::new(),
            max_output_tokens: 4096,
            context_tokens: None,
            max_request_bytes: 2 * 1024 * 1024,
            timeout_secs: 120,
            max_in_flight_per_user: 2,
            max_in_flight_total: DEFAULT_MAX_IN_FLIGHT_TOTAL,
            max_tokens_per_day_per_user: 0,
            label: None,
            external: None,
            api_key: None,
            builtin_model: None,
            builtin_context: 8192,
            builtin_idle_secs: 600,
            builtin_slots: 1,
            builtin_program: None,
            accept_license: None,
        }
    }
}

fn invalid(key: &'static str, reason: &'static str) -> ConfigError {
    ConfigError::Invalid { key, reason }
}

impl LlmConfig {
    pub fn enabled(&self) -> bool {
        self.provider != LlmProvider::None
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.max_output_tokens == 0 {
            return Err(invalid("llm_max_output_tokens", "deve essere almeno 1"));
        }
        if self.max_request_bytes == 0 {
            return Err(invalid("llm_max_request_bytes", "deve essere almeno 1"));
        }
        if self.timeout_secs == 0 {
            return Err(invalid("llm_timeout_secs", "deve essere almeno 1"));
        }
        if self.max_in_flight_per_user == 0 {
            return Err(invalid("llm_max_in_flight_per_user", "deve essere almeno 1"));
        }
        if self.max_in_flight_total == 0 {
            return Err(invalid("llm_max_in_flight_total", "deve essere almeno 1"));
        }
        if self.allowed_models.iter().any(|m| m.trim().is_empty()) {
            return Err(invalid("llm_allowed_models", "contiene un nome vuoto"));
        }
        if let Some(label) = &self.label {
            let plain = label.bytes().all(|b| (0x20..0x7f).contains(&b) && b != b';');
            let edges = label.starts_with(' ') || label.ends_with(' ');
            if !(1..=64).contains(&label.len()) || !plain || edges {
                return Err(invalid(
                    "llm_label",
                    "da 1 a 64 caratteri ASCII stampabili, senza «;» e senza spazi ai bordi",
                ));
            }
        }
        // Il valore della chiave non compare mai nell'errore.
        if let Some(key) = &self.api_key
            && !key.expose().bytes().all(|b| (0x21..=0x7e).contains(&b))
        {
            return Err(invalid(
                "QAMILLE_LLM_API_KEY",
                "solo caratteri ASCII visibili, senza spazi né caratteri di controllo",
            ));
        }
        if !self.enabled() {
            return Ok(());
        }
        if self.provider == LlmProvider::Builtin {
            return self.validate_builtin();
        }
        let Some(url) = &self.base_url else {
            return Err(invalid("llm_base_url", "manca: serve con questo provider"));
        };
        let base = parse_base_url(url).map_err(|reason| invalid("llm_base_url", reason))?;
        if self.model.as_deref().is_none_or(|m| m.trim().is_empty()) {
            return Err(invalid("llm_model", "manca: serve con questo provider"));
        }
        if !base.https && !base.is_internal() {
            return Err(invalid(
                "llm_base_url",
                "http è ammesso solo verso host interni: per un servizio esterno usa https",
            ));
        }
        Ok(())
    }

    fn validate_builtin(&self) -> Result<(), ConfigError> {
        let Some(model) = self.builtin_model.as_deref() else {
            return Err(invalid("llm_builtin_model", "manca: serve con questo provider (per esempio llm-medio)"));
        };
        if !qamille_compute::llm_catalog::CATALOG.iter().any(|spec| spec.id == model) {
            return Err(invalid("llm_builtin_model", "non è un modello del catalogo"));
        }
        if !(1024..=262_144).contains(&self.builtin_context) {
            return Err(invalid("llm_builtin_context", "da 1024 a 262144 token"));
        }
        if self.builtin_idle_secs < 10 {
            return Err(invalid("llm_builtin_idle_secs", "deve essere almeno 10"));
        }
        if !(1..=4).contains(&self.builtin_slots) {
            return Err(invalid("llm_builtin_slots", "da 1 a 4"));
        }
        if self.builtin_program.as_deref().is_some_and(|p| !p.is_absolute()) {
            return Err(invalid("llm_builtin_program", "deve essere un percorso assoluto"));
        }
        if self.base_url.is_some() {
            return Err(invalid("llm_base_url", "non si usa con builtin: il modello gira su questo server"));
        }
        if self.context_tokens.is_some() {
            return Err(invalid("llm_context_tokens", "non si usa con builtin: vale llm_builtin_context"));
        }
        if self.external == Some(true) {
            return Err(invalid("llm_external", "builtin non è un servizio esterno"));
        }
        if self.model.as_deref().is_some_and(|m| m.trim() != model) {
            return Err(invalid("llm_model", "con builtin il modello è llm_builtin_model"));
        }
        if self.allowed_models.iter().any(|m| m.trim() != model) {
            return Err(invalid("llm_allowed_models", "con builtin è ammesso solo llm_builtin_model"));
        }
        Ok(())
    }

    /// L'indirizzo a valle, se c'è ed è valido.
    pub fn base(&self) -> Option<BaseUrl> {
        parse_base_url(self.base_url.as_deref()?).ok()
    }

    /// Il provider è un servizio esterno alla rete del server? Dedotto dall'host se non forzato.
    pub fn is_external(&self) -> bool {
        if self.provider == LlmProvider::Builtin {
            return false;
        }
        self.external
            .unwrap_or_else(|| self.base().is_some_and(|b| !b.is_internal()))
    }

    /// Il nome mostrato all'utente nel consenso.
    pub fn display_label(&self) -> String {
        if self.provider == LlmProvider::Builtin {
            return BUILTIN_LABEL.to_owned();
        }
        match &self.label {
            Some(label) => label.clone(),
            None => self.base().map(|b| b.default_label()).unwrap_or_default(),
        }
    }

    /// I modelli offerti: il predefinito per primo, poi gli altri senza doppioni.
    pub fn models(&self) -> Vec<String> {
        if self.provider == LlmProvider::Builtin {
            return self.builtin_model.iter().cloned().collect();
        }
        let mut out: Vec<String> = Vec::new();
        for name in self.model.iter().chain(self.allowed_models.iter()) {
            let name = name.trim();
            if !name.is_empty() && !out.iter().any(|m| m == name) {
                out.push(name.to_owned());
            }
        }
        out
    }
}

/// L'indirizzo di base a valle, già scomposto.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BaseUrl {
    pub https: bool,
    /// Minuscolo; un indirizzo IPv6 senza parentesi quadre.
    pub host: String,
    pub port: Option<u16>,
    /// Vuoto o con barra iniziale, mai con barra finale.
    pub path: String,
}

impl BaseUrl {
    /// Loopback, indirizzo privato, `localhost` o un nome di servizio senza punti (come i
    /// servizi di compose, `ollama`). Un nome senza punti fatto di sole cifre o con prefisso
    /// `0x` non è un nome: i resolver lo leggono come un IPv4 (`134744072`, `0x08080808` sono
    /// 8.8.8.8) e non è interno.
    pub fn is_internal(&self) -> bool {
        match self.host.parse::<IpAddr>() {
            Ok(ip) => is_local_or_private(ip),
            Err(_) => {
                self.host == "localhost"
                    || (!self.host.contains('.')
                        && !self.host.bytes().all(|b| b.is_ascii_digit())
                        && !self.host.starts_with("0x"))
            }
        }
    }

    pub fn chat_url(&self) -> String {
        self.endpoint("/chat/completions")
    }

    /// L'indirizzo di una rotta del servizio (`/chat/completions`, `/embeddings`).
    pub fn endpoint(&self, route: &str) -> String {
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        let port = self.port.map(|p| format!(":{p}")).unwrap_or_default();
        let scheme = if self.https { "https" } else { "http" };
        format!("{scheme}://{host}{port}{}{route}", self.path)
    }

    pub fn default_label(&self) -> String {
        if self.host == "api.openai.com" {
            "OpenAI".to_owned()
        } else if self.host.ends_with(".openai.azure.com") {
            "Azure OpenAI".to_owned()
        } else {
            self.host.clone()
        }
    }

    /// I modelli nuovi di OpenAI rifiutano `max_tokens`.
    pub fn wants_completion_tokens(&self) -> bool {
        self.host == "api.openai.com" || self.host.ends_with(".openai.azure.com")
    }
}

/// Scompone `http(s)://host[:porta][/percorso]`. Rifiuta credenziali, query, frammento, spazi
/// e ogni altro schema.
pub fn parse_base_url(value: &str) -> Result<BaseUrl, &'static str> {
    let value = value.trim();
    if value.is_empty() {
        return Err("indirizzo vuoto");
    }
    if value.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("l'indirizzo non può contenere spazi");
    }
    let (https, rest) = if let Some(rest) = value.strip_prefix("https://") {
        (true, rest)
    } else if let Some(rest) = value.strip_prefix("http://") {
        (false, rest)
    } else {
        return Err("lo schema deve essere http o https");
    };
    if rest.contains('?') || rest.contains('#') {
        return Err("l'indirizzo non può avere query né frammento");
    }
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    if authority.contains('@') {
        return Err("l'indirizzo non può contenere credenziali");
    }
    let (host, port_text) = if let Some(inner) = authority.strip_prefix('[') {
        let end = inner.find(']').ok_or("indirizzo IPv6 non chiuso")?;
        let after = &inner[end + 1..];
        let port = match after {
            "" => None,
            _ => Some(after.strip_prefix(':').ok_or("indirizzo non valido")?),
        };
        let host = &inner[..end];
        if host.parse::<std::net::Ipv6Addr>().is_err() {
            return Err("indirizzo IPv6 non valido");
        }
        (host.to_ascii_lowercase(), port)
    } else {
        match authority.split_once(':') {
            Some((host, port)) => (host.to_ascii_lowercase(), Some(port)),
            None => (authority.to_ascii_lowercase(), None),
        }
    };
    if host.is_empty() {
        return Err("manca l'host");
    }
    if !host.contains(':')
        && !host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_')
    {
        return Err("host non valido");
    }
    let port = match port_text {
        None => None,
        Some(text) => {
            let port: u16 = text.parse().map_err(|_| "porta non valida")?;
            if port == 0 {
                return Err("porta non valida");
            }
            Some(port)
        }
    };
    let path = path.trim_end_matches('/').to_owned();
    Ok(BaseUrl { https, host, port, path })
}

// --- Richiesta, risposta ed errori (logica pura: niente rete, niente database) ----------------

/// Pausa suggerita quando il provider non risponde o rifiuta la nostra chiave.
pub const UNAVAILABLE_RETRY_S: u64 = 30;
/// Pausa suggerita quando i posti in volo dell'utente sono pieni.
pub const BUSY_RETRY_S: u64 = 2;
/// Il `json_schema` di `response_format` non può superare questa dimensione.
pub const MAX_JSON_SCHEMA_BYTES: usize = 64 * 1024;
/// Il corpo di una risposta del provider è tagliato qui.
pub const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
/// Dal corpo di un errore del provider si legge al massimo questo.
pub const MAX_ERROR_BODY_BYTES: usize = 64 * 1024;
/// Margine della rotta oltre il tempo concesso alla chiamata a valle.
pub const ROUTE_EXTRA_SECS: u64 = 10;

/// Perché una richiesta all'assistente non è andata a buon fine. Né il `Debug` né il `Display`
/// contengono mai testo del provider (INV-33).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LlmError {
    InvalidRequest(&'static str),
    ContextExceeded,
    Unavailable { retry_after_s: u64 },
    /// The built-in model needs more free memory than the server has right now.
    NotEnoughMemory { retry_after_s: u64 },
    /// The built-in model is being downloaded by the server (`503 llm_unavailable`, with a pause).
    Preparing { retry_after_s: u64 },
    RateLimited { retry_after_s: u64 },
}

impl fmt::Display for LlmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LlmError::InvalidRequest(message) => f.write_str(message),
            LlmError::ContextExceeded => f.write_str("conversation too long for the model"),
            LlmError::Unavailable { .. } => f.write_str("assistant provider unavailable"),
            LlmError::NotEnoughMemory { .. } => f.write_str("not enough free memory for the assistant model"),
            LlmError::Preparing { .. } => f.write_str("the assistant model is being prepared"),
            LlmError::RateLimited { .. } => f.write_str("assistant provider rate limited"),
        }
    }
}

impl std::error::Error for LlmError {}

impl From<LlmError> for ComputeError {
    fn from(error: LlmError) -> Self {
        match error {
            LlmError::InvalidRequest(message) => ComputeError::InvalidRequest(message.to_owned()),
            LlmError::ContextExceeded => ComputeError::LlmContextExceeded,
            LlmError::Unavailable { retry_after_s } => ComputeError::LlmUnavailable { retry_after_s },
            LlmError::NotEnoughMemory { retry_after_s } => ComputeError::LlmNotEnoughMemory { retry_after_s },
            LlmError::Preparing { retry_after_s } => ComputeError::LlmPreparing { retry_after_s },
            LlmError::RateLimited { retry_after_s } => ComputeError::RateLimited { retry_after_s },
        }
    }
}

/// La richiesta da inoltrare, costruita da zero.
#[derive(Debug)]
pub struct Outgoing {
    pub model: String,
    pub body: Vec<u8>,
    /// Caratteri dei messaggi: servono a stimare i token se il provider non li dichiara.
    pub prompt_chars: usize,
}

/// La risposta filtrata e i suoi token.
#[derive(Debug)]
pub struct Completion {
    pub body: Vec<u8>,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

/// Oltre questi token dichiarati dal provider per una sola chiamata (prompt + risposta) non si
/// crede al provider e si usa la stima.
pub const MAX_DECLARED_TOKENS: u64 = 10_000_000;

/// `ceil(caratteri / 4)`.
pub fn estimate_tokens(chars: usize) -> u64 {
    (chars as u64).div_ceil(4)
}

/// Lo SHA-256 esadecimale dell'id interno dell'utente: è l'unico `user` che arriva a valle.
pub fn user_hash(user_id: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(user_id.as_bytes()))
}

const BAD_BODY: &str = "The request is not a valid chat completion.";
const BAD_MODEL: &str = "The model is not available on this server.";
const BAD_MESSAGES: &str = "The messages must be a non-empty list of system, user or assistant text.";
const BAD_OPTION: &str = "A request option has an unsupported value.";
const UNSUPPORTED: &str = "Streaming, tools, functions, several answers, images and audio are not supported.";

/// Controlla la richiesta dell'app e ne costruisce quella a valle: solo i campi ammessi, il
/// `user` sostituito, il tetto delle risposte ridotto a `max_output_tokens`.
pub fn validate_request(
    config: &LlmConfig,
    body: &[u8],
    user_id: &str,
) -> Result<Outgoing, LlmError> {
    let value: serde_json::Value =
        serde_json::from_slice(body).map_err(|_| LlmError::InvalidRequest(BAD_BODY))?;
    let request = value.as_object().ok_or(LlmError::InvalidRequest(BAD_BODY))?;

    for key in ["tools", "functions", "tool_choice", "function_call"] {
        if request.contains_key(key) {
            return Err(LlmError::InvalidRequest(UNSUPPORTED));
        }
    }
    if let Some(stream) = request.get("stream")
        && !matches!(stream, serde_json::Value::Null | serde_json::Value::Bool(false))
    {
        return Err(LlmError::InvalidRequest(UNSUPPORTED));
    }
    if let Some(n) = request.get("n")
        && !n.is_null()
        && n.as_u64() != Some(1)
    {
        return Err(LlmError::InvalidRequest(UNSUPPORTED));
    }

    let models = config.models();
    let model = match request.get("model") {
        None | Some(serde_json::Value::Null) => {
            models.first().cloned().ok_or(LlmError::InvalidRequest(BAD_MODEL))?
        }
        Some(serde_json::Value::String(name)) => {
            if !models.iter().any(|m| m == name) {
                return Err(LlmError::InvalidRequest(BAD_MODEL));
            }
            name.clone()
        }
        Some(_) => return Err(LlmError::InvalidRequest(BAD_MODEL)),
    };

    let raw_messages = request
        .get("messages")
        .and_then(|m| m.as_array())
        .filter(|m| !m.is_empty())
        .ok_or(LlmError::InvalidRequest(BAD_MESSAGES))?;
    let mut messages = Vec::with_capacity(raw_messages.len());
    let mut prompt_chars = 0usize;
    for raw in raw_messages {
        let message = raw.as_object().ok_or(LlmError::InvalidRequest(BAD_MESSAGES))?;
        let role = match message.get("role").and_then(|r| r.as_str()) {
            Some(role @ ("system" | "user" | "assistant")) => role,
            _ => return Err(LlmError::InvalidRequest(BAD_MESSAGES)),
        };
        let content = match message.get("content") {
            Some(serde_json::Value::String(text)) => text.clone(),
            Some(serde_json::Value::Array(parts)) => {
                let mut texts = Vec::with_capacity(parts.len());
                for part in parts {
                    let part = part.as_object().ok_or(LlmError::InvalidRequest(BAD_MESSAGES))?;
                    match (part.get("type").and_then(|t| t.as_str()), part.get("text")) {
                        (Some("text"), Some(serde_json::Value::String(text))) => texts.push(text.as_str()),
                        _ => return Err(LlmError::InvalidRequest(UNSUPPORTED)),
                    }
                }
                texts.join("\n")
            }
            _ => return Err(LlmError::InvalidRequest(BAD_MESSAGES)),
        };
        prompt_chars += content.chars().count();
        messages.push(serde_json::json!({"role": role, "content": content}));
    }

    let mut out = serde_json::Map::new();
    out.insert("model".to_owned(), model.clone().into());
    out.insert("messages".to_owned(), messages.into());

    if let Some(value) = request.get("temperature").filter(|v| !v.is_null()) {
        let t = value.as_f64().filter(|t| (0.0..=2.0).contains(t)).ok_or(LlmError::InvalidRequest(BAD_OPTION))?;
        out.insert("temperature".to_owned(), serde_json::json!(t));
    }
    if let Some(value) = request.get("top_p").filter(|v| !v.is_null()) {
        let p = value.as_f64().filter(|p| (0.0..=1.0).contains(p)).ok_or(LlmError::InvalidRequest(BAD_OPTION))?;
        out.insert("top_p".to_owned(), serde_json::json!(p));
    }

    // Il tetto delle risposte: il più basso fra quello chiesto (con i due nomi) e quello del server.
    let cap = u64::from(config.max_output_tokens);
    let mut ceiling = cap;
    for key in ["max_tokens", "max_completion_tokens"] {
        if let Some(value) = request.get(key).filter(|v| !v.is_null()) {
            let asked = value.as_u64().filter(|n| *n >= 1).ok_or(LlmError::InvalidRequest(BAD_OPTION))?;
            ceiling = ceiling.min(asked);
        }
    }
    let field = match config.base() {
        Some(base) if base.wants_completion_tokens() => "max_completion_tokens",
        _ => "max_tokens",
    };
    out.insert(field.to_owned(), ceiling.into());

    if let Some(value) = request.get("stop").filter(|v| !v.is_null()) {
        let valid = match value {
            serde_json::Value::String(_) => true,
            serde_json::Value::Array(items) => items.len() <= 4 && items.iter().all(|i| i.is_string()),
            _ => false,
        };
        if !valid {
            return Err(LlmError::InvalidRequest(BAD_OPTION));
        }
        out.insert("stop".to_owned(), value.clone());
    }
    if let Some(value) = request.get("response_format").filter(|v| !v.is_null()) {
        let format = value.as_object().ok_or(LlmError::InvalidRequest(BAD_OPTION))?;
        let kind = format.get("type").and_then(|t| t.as_str());
        let clean = match kind {
            Some("text") => serde_json::json!({"type": "text"}),
            Some("json_object") => serde_json::json!({"type": "json_object"}),
            Some("json_schema") => {
                let schema = format
                    .get("json_schema")
                    .and_then(|s| s.as_object())
                    .ok_or(LlmError::InvalidRequest(BAD_OPTION))?;
                // Solo `name`, `strict` e `schema`, di tipo giusto e in tutto non oltre 64 KB.
                let mut clean = serde_json::Map::new();
                for (key, valid) in [
                    ("name", serde_json::Value::is_string as fn(&serde_json::Value) -> bool),
                    ("strict", serde_json::Value::is_boolean),
                    ("schema", serde_json::Value::is_object),
                ] {
                    if let Some(value) = schema.get(key).filter(|v| !v.is_null()) {
                        if !valid(value) {
                            return Err(LlmError::InvalidRequest(BAD_OPTION));
                        }
                        clean.insert(key.to_owned(), value.clone());
                    }
                }
                let size = serde_json::to_vec(&clean).map_or(usize::MAX, |bytes| bytes.len());
                if size > MAX_JSON_SCHEMA_BYTES {
                    return Err(LlmError::InvalidRequest(BAD_OPTION));
                }
                serde_json::json!({"type": "json_schema", "json_schema": clean})
            }
            _ => return Err(LlmError::InvalidRequest(BAD_OPTION)),
        };
        out.insert("response_format".to_owned(), clean);
    }
    if let Some(value) = request.get("seed").filter(|v| !v.is_null()) {
        let seed = value.as_i64().ok_or(LlmError::InvalidRequest(BAD_OPTION))?;
        out.insert("seed".to_owned(), seed.into());
    }
    out.insert("user".to_owned(), user_hash(user_id).into());

    let body = serde_json::to_vec(&serde_json::Value::Object(out))
        .map_err(|_| LlmError::InvalidRequest(BAD_BODY))?;
    Ok(Outgoing { model, body, prompt_chars })
}

/// Tiene della risposta del provider solo i campi ammessi e ne ricava i token (stimati se il
/// provider non li dichiara). Una risposta che non ha la forma attesa è `Unavailable`.
pub fn filter_response(raw: &[u8], prompt_chars: usize) -> Result<Completion, LlmError> {
    let unavailable = LlmError::Unavailable { retry_after_s: UNAVAILABLE_RETRY_S };
    let value: serde_json::Value = serde_json::from_slice(raw).map_err(|_| unavailable.clone())?;
    let choices = value
        .get("choices")
        .and_then(|c| c.as_array())
        .filter(|c| !c.is_empty())
        .ok_or_else(|| unavailable.clone())?;

    let mut clean = Vec::with_capacity(choices.len());
    let mut completion_chars = 0usize;
    for (position, choice) in choices.iter().enumerate() {
        let choice = choice.as_object().ok_or_else(|| unavailable.clone())?;
        let content = choice
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or("");
        completion_chars += content.chars().count();
        let index = choice.get("index").and_then(|i| i.as_u64()).unwrap_or(position as u64);
        let finish = choice
            .get("finish_reason")
            .filter(|f| f.is_string())
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        clean.push(serde_json::json!({
            "index": index,
            "message": {"role": "assistant", "content": content},
            "finish_reason": finish,
        }));
    }

    let declared = value.get("usage").and_then(|u| {
        Some((u.get("prompt_tokens")?.as_u64()?, u.get("completion_tokens")?.as_u64()?))
    });
    // Un provider ostile o difettoso può dichiarare numeri enormi: oltre il limite per chiamata
    // vale la stima.
    let declared = declared.filter(|(p, c)| p.saturating_add(*c) <= MAX_DECLARED_TOKENS);
    let (prompt_tokens, completion_tokens) =
        declared.unwrap_or((estimate_tokens(prompt_chars), estimate_tokens(completion_chars)));

    let text = |key: &str| value.get(key).and_then(|v| v.as_str()).unwrap_or("").to_owned();
    let body = serde_json::json!({
        "id": text("id"),
        "object": "chat.completion",
        "created": value.get("created").and_then(|c| c.as_u64()).unwrap_or(0),
        "model": text("model"),
        "choices": clean,
        "usage": {
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens,
            "total_tokens": prompt_tokens.saturating_add(completion_tokens),
        },
    });
    let body = serde_json::to_vec(&body).map_err(|_| unavailable)?;
    Ok(Completion { body, prompt_tokens, completion_tokens })
}

/// Traduce lo stato di una risposta d'errore del provider. Del corpo si legge solo il codice
/// dell'errore (mai il testo, che non arriva né all'app né al log).
pub fn classify_status(status: u16, retry_after: Option<u64>, body: &[u8]) -> LlmError {
    match status {
        429 => LlmError::RateLimited {
            retry_after_s: retry_after.unwrap_or(UNAVAILABLE_RETRY_S).clamp(1, 3600),
        },
        400 if exceeds_context(&body[..body.len().min(MAX_ERROR_BODY_BYTES)]) => {
            LlmError::ContextExceeded
        }
        400 => LlmError::InvalidRequest("The assistant could not process this request."),
        _ => LlmError::Unavailable { retry_after_s: UNAVAILABLE_RETRY_S },
    }
}

fn exceeds_context(body: &[u8]) -> bool {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return false;
    };
    let error = value.get("error").unwrap_or(&value);
    let field = |name: &str| error.get(name).and_then(|v| v.as_str());
    let code = |name: &str| matches!(field(name), Some("context_length_exceeded" | "exceed_context_size_error"));
    if code("code") || code("type") {
        return true;
    }
    field("message").is_some_and(|message| {
        let message = message.to_ascii_lowercase();
        ["context length", "context window", "maximum context"].iter().any(|k| message.contains(k))
    })
}

// --- Inoltro a valle --------------------------------------------------------------------------

/// Ciò che `GET /v1/info` dichiara dell'assistente (mai la chiave né l'indirizzo).
#[derive(Clone, Debug, Serialize)]
pub struct LlmInfo {
    pub model: String,
    pub models: Vec<String>,
    pub external: bool,
    pub label: String,
    pub context_tokens: Option<u32>,
}

impl LlmInfo {
    pub fn from_config(config: &LlmConfig) -> LlmInfo {
        LlmInfo {
            model: match config.provider {
                LlmProvider::Builtin => config.builtin_model.clone().unwrap_or_default(),
                _ => config.model.clone().unwrap_or_default(),
            },
            models: config.models(),
            external: config.is_external(),
            label: config.display_label(),
            context_tokens: match config.provider {
                LlmProvider::Builtin => Some(config.builtin_context),
                _ => config.context_tokens,
            },
        }
    }
}

/// Il giorno UTC (`YYYY-MM-DD`) e i token già spesi da un utente in quel giorno.
type DayCount = (String, u64);

/// Dove va la richiesta: un provider con indirizzo fisso, o il figlio interno, il cui indirizzo e
/// la cui chiave cambiano a ogni avvio e si chiedono al supervisore a ogni richiesta.
enum Downstream {
    Fixed { base: BaseUrl },
    Builtin(Arc<LlmSupervisor>),
}

/// Il gateway verso il provider: un agente ureq senza redirect, i posti in volo per utente e il
/// conto giornaliero in memoria.
pub struct LlmGateway {
    config: LlmConfig,
    downstream: Downstream,
    agent: ureq::Agent,
    accounts: Arc<Accounts>,
    sink: Arc<dyn UsageSink>,
    days: Mutex<HashMap<String, DayCount>>,
    in_flight: Mutex<HashMap<String, usize>>,
    /// I posti globali (`llm_max_in_flight_total`): presi con `try_acquire`, mai in attesa.
    total: Arc<Semaphore>,
}

impl fmt::Debug for LlmGateway {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LlmGateway").field("provider", &self.config.provider).finish_non_exhaustive()
    }
}

/// Un posto tra le richieste in volo di un utente: si libera alla fine (anche se l'app se ne
/// è andata prima che il provider rispondesse).
pub struct LlmSlot {
    gateway: Arc<LlmGateway>,
    user_id: String,
    /// Il posto globale: si libera con il posto dell'utente.
    _permit: OwnedSemaphorePermit,
}

impl LlmSlot {
    /// Il gateway con cui la richiesta è entrata: resta quello anche se il pannello lo cambia
    /// mentre la richiesta è in volo.
    pub fn gateway(&self) -> &Arc<LlmGateway> {
        &self.gateway
    }
}

impl Drop for LlmSlot {
    fn drop(&mut self) {
        let mut map = self.gateway.in_flight.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(count) = map.get_mut(&self.user_id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                map.remove(&self.user_id);
            }
        }
    }
}

fn error_code(error: &LlmError) -> &'static str {
    match error {
        LlmError::InvalidRequest(_) => "invalid_request",
        LlmError::ContextExceeded => "llm_context_exceeded",
        LlmError::Unavailable { .. } | LlmError::NotEnoughMemory { .. } | LlmError::Preparing { .. } => "llm_unavailable",
        LlmError::RateLimited { .. } => "rate_limited",
    }
}

impl LlmGateway {
    pub fn new(
        config: &LlmConfig,
        accounts: Arc<Accounts>,
        sink: Arc<dyn UsageSink>,
    ) -> Result<LlmGateway, ConfigError> {
        config.validate()?;
        if config.provider == LlmProvider::Builtin {
            return Err(invalid("llm_provider", "builtin si monta con LlmGateway::with_supervisor"));
        }
        let base = config
            .base()
            .ok_or_else(|| invalid("llm_base_url", "manca l'indirizzo del provider"))?;
        Ok(Self::build(config, Downstream::Fixed { base }, accounts, sink))
    }

    /// Il gateway con il provider `builtin`: la chiamata a valle va al figlio che il supervisore
    /// avvia con la prima richiesta.
    pub fn with_supervisor(
        config: &LlmConfig,
        accounts: Arc<Accounts>,
        sink: Arc<dyn UsageSink>,
        supervisor: Arc<LlmSupervisor>,
    ) -> Result<LlmGateway, ConfigError> {
        config.validate()?;
        if config.provider != LlmProvider::Builtin {
            return Err(invalid("llm_provider", "il supervisore serve solo con builtin"));
        }
        Ok(Self::build(config, Downstream::Builtin(supervisor), accounts, sink))
    }

    /// Ferma il figlio interno, se c'è (dopo gli ascolti, alla chiusura del server).
    pub fn shutdown(&self) {
        if let Downstream::Builtin(supervisor) = &self.downstream {
            supervisor.shutdown();
        }
    }

    fn build(config: &LlmConfig, downstream: Downstream, accounts: Arc<Accounts>, sink: Arc<dyn UsageSink>) -> LlmGateway {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(config.timeout_secs)))
            .max_redirects(0)
            .max_redirects_will_error(false)
            .http_status_as_error(false)
            // Mai il proxy dell'ambiente (HTTP_PROXY...): l'indirizzo lo sceglie l'operatore.
            .proxy(None)
            .build()
            .into();
        LlmGateway {
            config: config.clone(),
            downstream,
            agent,
            accounts,
            sink,
            days: Mutex::new(HashMap::new()),
            in_flight: Mutex::new(HashMap::new()),
            total: Arc::new(Semaphore::new(config.max_in_flight_total as usize)),
        }
    }

    pub fn info(&self) -> LlmInfo {
        LlmInfo::from_config(&self.config)
    }

    /// Una riga di log all'avvio: provider, modello, `external` e solo l'host (mai l'indirizzo
    /// intero né la chiave); con un provider esterno e senza chiave, un avviso.
    pub fn log_startup(&self) {
        match &self.downstream {
            Downstream::Fixed { base } => tracing::info!(
                provider = self.config.provider.as_str(),
                model = self.config.model.as_deref().unwrap_or(""),
                external = self.config.is_external(),
                host = %base.host,
                "assistente (gateway LLM) montato"
            ),
            Downstream::Builtin(_) => tracing::info!(
                provider = self.config.provider.as_str(),
                model = self.config.builtin_model.as_deref().unwrap_or(""),
                external = false,
                "assistente (gateway LLM) montato"
            ),
        }
        if self.config.is_external() && self.config.api_key.is_none() {
            tracing::warn!("provider LLM esterno senza QAMILLE_LLM_API_KEY: il provider probabilmente rifiuterà le richieste");
        }
    }

    pub fn models(&self) -> Vec<String> {
        self.config.models()
    }

    pub fn config(&self) -> &LlmConfig {
        &self.config
    }

    /// Il consenso dell'app, `<external>;<label>`, confrontato byte a byte con il provider attuale.
    pub fn consent_matches(&self, header: Option<&[u8]>) -> bool {
        let expected = format!("{};{}", self.config.is_external(), self.config.display_label());
        header == Some(expected.as_bytes())
    }

    /// Un posto tra le richieste in volo di `user_id`, o `None` oltre il limite dell'utente o
    /// quello globale. Mai in attesa. Il posto globale si prende per primo: un rifiuto per
    /// l'utente lo rilascia da sé.
    pub fn enter(self: &Arc<Self>, user_id: &str) -> Option<LlmSlot> {
        let permit = Arc::clone(&self.total).try_acquire_owned().ok()?;
        let mut map = self.in_flight.lock().unwrap_or_else(|p| p.into_inner());
        let count = map.entry(user_id.to_owned()).or_insert(0);
        if *count >= self.config.max_in_flight_per_user as usize {
            return None;
        }
        *count += 1;
        Some(LlmSlot { gateway: Arc::clone(self), user_id: user_id.to_owned(), _permit: permit })
    }

    pub fn in_flight(&self, user_id: &str) -> usize {
        self.in_flight.lock().map(|map| map.get(user_id).copied().unwrap_or(0)).unwrap_or(0)
    }

    /// Soglia morbida: `RateLimited` fino a mezzanotte UTC se l'utente ha già speso il tetto del
    /// giorno. Il conto si legge dal database una volta per utente e giorno; se la lettura
    /// fallisce, `Unavailable` (503) per quella richiesta e nessun valore in memoria.
    pub async fn within_daily_cap(&self, user_id: &str) -> Result<(), LlmError> {
        let cap = self.config.max_tokens_per_day_per_user;
        if cap == 0 {
            return Ok(());
        }
        let now = self.accounts.now();
        let day = now.format("%Y-%m-%d").to_string();
        let known = {
            let days = self.days.lock().unwrap_or_else(|p| p.into_inner());
            days.get(user_id).filter(|(d, _)| *d == day).map(|(_, tokens)| *tokens)
        };
        let spent = match known {
            Some(tokens) => tokens,
            None => {
                // Se il conto non si legge, la richiesta non passa e nulla si ricorda (niente
                // zero in memoria fino a mezzanotte): fallisce chiusa.
                let Ok(stored) = self.accounts.llm_tokens_on(user_id, &day).await else {
                    tracing::warn!("conto giornaliero dell'assistente non letto: richiesta rifiutata");
                    return Err(LlmError::Unavailable { retry_after_s: UNAVAILABLE_RETRY_S });
                };
                let mut days = self.days.lock().unwrap_or_else(|p| p.into_inner());
                let entry = days.entry(user_id.to_owned()).or_insert_with(|| (day.clone(), stored));
                if entry.0 != day {
                    *entry = (day.clone(), stored);
                }
                entry.1
            }
        };
        if spent < cap {
            return Ok(());
        }
        let midnight = now
            .date_naive()
            .succ_opt()
            .and_then(|d| d.and_hms_opt(0, 0, 0))
            .map(|d| d.and_utc());
        let retry_after_s = midnight.map_or(3600, |m| (m - now).num_seconds().max(1) as u64);
        Err(LlmError::RateLimited { retry_after_s })
    }

    fn add_day_tokens(&self, user_id: &str, tokens: u64) {
        let day = self.accounts.now().format("%Y-%m-%d").to_string();
        let mut days = self.days.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(entry) = days.get_mut(user_id)
            && entry.0 == day
        {
            entry.1 = entry.1.saturating_add(tokens);
        }
    }

    /// Valida la richiesta, la inoltra e filtra la risposta. La chiamata a valle gira su
    /// `spawn_blocking` e possiede il posto: se l'app se ne va, il posto si libera solo quando il
    /// provider ha risposto, e i token si contano lo stesso.
    pub async fn complete(
        self: &Arc<Self>,
        user_id: &str,
        slot: Arc<LlmSlot>,
        body: Vec<u8>,
    ) -> Result<Vec<u8>, LlmError> {
        let outgoing = validate_request(&self.config, &body, user_id)?;
        drop(body);
        let gateway = Arc::clone(self);
        let user_id = user_id.to_owned();
        tokio::task::spawn_blocking(move || {
            let _slot = slot;
            gateway.call(&user_id, outgoing)
        })
        .await
        .map_err(|_| LlmError::Unavailable { retry_after_s: UNAVAILABLE_RETRY_S })?
    }

    fn call(&self, user_id: &str, outgoing: Outgoing) -> Result<Vec<u8>, LlmError> {
        let started = Instant::now();
        let result = self.forward(&outgoing);
        let duration_ms = started.elapsed().as_millis() as u64;
        match result {
            Ok(completion) => {
                tracing::info!(
                    model = %outgoing.model,
                    prompt_tokens = completion.prompt_tokens,
                    completion_tokens = completion.completion_tokens,
                    duration_ms,
                    "llm"
                );
                self.add_day_tokens(
                    user_id,
                    completion.prompt_tokens.saturating_add(completion.completion_tokens),
                );
                self.sink.record(
                    user_id,
                    Usage::new(0, 0.0, 0).with_llm(completion.prompt_tokens, completion.completion_tokens),
                );
                Ok(completion.body)
            }
            Err(error) => {
                tracing::warn!(code = error_code(&error), duration_ms, "llm");
                Err(error)
            }
        }
    }

    fn forward(&self, outgoing: &Outgoing) -> Result<Completion, LlmError> {
        let unavailable = || LlmError::Unavailable { retry_after_s: UNAVAILABLE_RETRY_S };
        // Con `builtin` l'avvio del figlio (secondi) sta qui, prima del tempo massimo della
        // chiamata, e l'`Endpoint` resta in vita fino alla fine: il figlio non si ferma per
        // inattività mentre la richiesta è in volo. Siamo già su `spawn_blocking`.
        let (url, bearer, _endpoint) = match &self.downstream {
            Downstream::Fixed { base } => (base.chat_url(), self.config.api_key.as_ref().map(|k| k.expose().to_owned()), None),
            Downstream::Builtin(supervisor) => {
                let endpoint = supervisor.endpoint()?;
                (endpoint.chat_url(), Some(endpoint.key().expose().to_owned()), Some(endpoint))
            }
        };
        let mut request = self
            .agent
            .post(&url)
            .header("Content-Type", "application/json")
            .header("User-Agent", concat!("qamille-server/", env!("CARGO_PKG_VERSION")));
        if let Some(key) = &bearer {
            request = request.header("Authorization", format!("Bearer {key}"));
        }
        let mut response = request.send(&outgoing.body[..]).map_err(|_| unavailable())?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<u64>().ok());
            let body = response
                .body_mut()
                .with_config()
                .limit(MAX_ERROR_BODY_BYTES as u64)
                .read_to_vec()
                .unwrap_or_default();
            if matches!(status, 401 | 403) {
                tracing::warn!("chiave del provider LLM rifiutata");
            }
            return Err(classify_status(status, retry_after, &body));
        }
        let raw = response
            .body_mut()
            .with_config()
            .limit(MAX_RESPONSE_BYTES as u64)
            .read_to_vec()
            .map_err(|_| unavailable())?;
        filter_response(&raw, outgoing.prompt_chars)
    }
}

// --- Impostazioni dal pannello /admin ----------------------------------------------------------

/// Il file, nella cartella dei dati, con le impostazioni dell'assistente scelte dal pannello
/// `/admin`: all'avvio e a ogni salvataggio hanno la precedenza sulla configurazione (file e
/// ambiente). Scritto con permessi 0600: può contenere la chiave del provider.
pub const SETTINGS_FILE: &str = "llm-settings.json";

/// L'assistente in uso, sostituito a caldo quando il pannello salva impostazioni nuove. Una
/// richiesta già entrata tiene il gateway del suo [`LlmSlot`] fino alla fine.
#[derive(Clone, Default)]
pub struct LlmHolder(Arc<RwLock<Option<Arc<LlmGateway>>>>);

impl fmt::Debug for LlmHolder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("LlmHolder").field(&self.get().is_some()).finish()
    }
}

impl LlmHolder {
    /// Il gateway in uso, se c'è un provider.
    pub fn get(&self) -> Option<Arc<LlmGateway>> {
        self.0.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// Mette `gateway` al posto di quello in uso e restituisce il precedente.
    pub fn replace(&self, gateway: Option<Arc<LlmGateway>>) -> Option<Arc<LlmGateway>> {
        std::mem::replace(&mut *self.0.write().unwrap_or_else(|p| p.into_inner()), gateway)
    }
}

pub(crate) fn trimmed(value: Option<&str>) -> Option<String> {
    value.map(str::trim).filter(|v| !v.is_empty()).map(str::to_owned)
}

/// Un file di impostazioni scritto per intero o per niente: un file temporaneo (0600 su Unix:
/// può contenere una chiave), poi lo spostamento al suo posto.
pub(crate) fn save_private(path: &Path, body: &[u8]) -> std::io::Result<()> {
    let temporary = path.with_extension("json.tmp");
    {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(body)?;
        file.sync_all()?;
    }
    std::fs::rename(&temporary, path)
}

/// Le impostazioni dell'assistente scelte dal pannello. `api_key` `None`: vale la chiave della
/// configurazione (`QAMILLE_LLM_API_KEY`), se c'è; nessuna chiave dell'ambiente viene copiata qui.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LlmSettings {
    /// `none`, `openai-compatible` o `builtin`.
    pub provider: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub allowed_models: Vec<String>,
    /// 0 = nessun tetto.
    #[serde(default)]
    pub max_tokens_per_day_per_user: u64,
    #[serde(default)]
    pub builtin_model: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
}

impl fmt::Debug for LlmSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LlmSettings")
            .field("provider", &self.provider)
            .field("model", &self.model)
            .field("builtin_model", &self.builtin_model)
            .field("api_key", &self.api_key.as_ref().map(|_| "<nascosta>"))
            .finish_non_exhaustive()
    }
}

impl LlmSettings {
    /// Ciò che dice la configurazione, senza la sua chiave (che resta dov'è).
    pub fn from_config(config: &LlmConfig) -> LlmSettings {
        LlmSettings {
            provider: config.provider.as_str().to_owned(),
            base_url: config.base_url.clone(),
            model: config.model.clone(),
            allowed_models: config.allowed_models.clone(),
            max_tokens_per_day_per_user: config.max_tokens_per_day_per_user,
            builtin_model: config.builtin_model.clone(),
            api_key: None,
        }
    }

    /// Le impostazioni salvate in `path`; nessun file: `None`. Un file illeggibile o non valido
    /// vale come assente, con un avviso (mai il suo contenuto nel log).
    pub fn load(path: &Path) -> Option<LlmSettings> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
            Err(_) => {
                tracing::warn!(file = SETTINGS_FILE, "impostazioni dell'assistente illeggibili: valgono quelle della configurazione");
                return None;
            }
        };
        match serde_json::from_slice(&bytes) {
            Ok(settings) => Some(settings),
            Err(_) => {
                tracing::warn!(file = SETTINGS_FILE, "impostazioni dell'assistente non valide: valgono quelle della configurazione");
                None
            }
        }
    }

    /// Scritte in `path` per intero o per niente: un file temporaneo (0600 su Unix), poi lo
    /// spostamento al suo posto.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let body = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        save_private(path, &body)
    }

    /// `base` con queste impostazioni sopra, validata come la configurazione all'avvio.
    pub fn applied_to(&self, base: &LlmConfig) -> Result<LlmConfig, ConfigError> {
        let mut config = base.clone();
        config.provider = LlmProvider::parse(&self.provider)?;
        config.max_tokens_per_day_per_user = self.max_tokens_per_day_per_user;
        match config.provider {
            LlmProvider::None => {}
            LlmProvider::OpenAiCompatible => {
                let same_service = base.provider == LlmProvider::OpenAiCompatible
                    && trimmed(base.base_url.as_deref()) == trimmed(self.base_url.as_deref());
                config.base_url = trimmed(self.base_url.as_deref());
                config.model = trimmed(self.model.as_deref());
                config.allowed_models = self.allowed_models.iter().filter_map(|m| trimmed(Some(m))).collect();
                if let Some(key) = self.api_key.as_deref().and_then(ApiKey::new) {
                    config.api_key = Some(key);
                }
                if !same_service {
                    // Nome, natura e contesto dichiarati per un altro servizio ingannerebbero il consenso.
                    config.label = None;
                    config.external = None;
                    config.context_tokens = None;
                }
            }
            LlmProvider::Builtin => {
                config.builtin_model = trimmed(self.builtin_model.as_deref());
                config.base_url = None;
                config.model = None;
                config.allowed_models = Vec::new();
                config.context_tokens = None;
                config.external = None;
                config.label = None;
                if base.provider != LlmProvider::Builtin {
                    config.timeout_secs = config.timeout_secs.max(BUILTIN_TIMEOUT_SECS);
                }
            }
        }
        config.validate()?;
        Ok(config)
    }
}

/// Il corpo di `PUT /admin/api/llm`. Mai in un `Debug`: può portare la chiave.
#[derive(Deserialize)]
pub struct LlmSettingsInput {
    pub provider: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub allowed_models: Vec<String>,
    #[serde(default)]
    pub max_tokens_per_day_per_user: u64,
    #[serde(default)]
    pub builtin_model: Option<String>,
    /// Una chiave nuova; vuota o assente: resta quella già impostata dal pannello.
    #[serde(default)]
    pub api_key: Option<String>,
    /// Toglie la chiave impostata dal pannello (torna a valere quella della configurazione, se c'è).
    #[serde(default)]
    pub remove_api_key: bool,
}

/// Ciò che le impostazioni chiedono: un gateway (o nessuno) subito, o un riavvio per un modello
/// locale che questo server non ha acceso.
enum Next {
    Run(Option<Arc<LlmGateway>>),
    Restart,
}

/// Le impostazioni dell'assistente dal pannello: cosa è salvato, cosa gira, e il cambio a caldo.
pub struct LlmAdmin {
    holder: LlmHolder,
    /// La configurazione del file e dell'ambiente, prima delle impostazioni salvate.
    base: LlmConfig,
    path: PathBuf,
    accounts: Arc<Accounts>,
    sink: Arc<dyn UsageSink>,
    /// Il modello locale con cui il server è partito e il suo supervisore: un modello locale si
    /// applica subito solo se è questo.
    builtin: Option<(String, Arc<LlmSupervisor>)>,
    /// Un salvataggio alla volta.
    lock: Mutex<()>,
    /// Le impostazioni salvate nominano un modello locale che parte al prossimo avvio.
    restart_required: AtomicBool,
}

impl fmt::Debug for LlmAdmin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LlmAdmin").field("path", &self.path).finish_non_exhaustive()
    }
}

/// Le ultime quattro lettere di una chiave lunga, per riconoscerla; di una corta, nessuna.
pub(crate) fn key_hint(key: &ApiKey) -> String {
    let chars: Vec<char> = key.expose().chars().collect();
    if chars.len() < 16 {
        return "…".to_owned();
    }
    format!("…{}", chars[chars.len() - 4..].iter().collect::<String>())
}

impl LlmAdmin {
    pub fn new(
        holder: LlmHolder,
        base: LlmConfig,
        path: PathBuf,
        accounts: Arc<Accounts>,
        sink: Arc<dyn UsageSink>,
        builtin: Option<(String, Arc<LlmSupervisor>)>,
    ) -> LlmAdmin {
        LlmAdmin { holder, base, path, accounts, sink, builtin, lock: Mutex::new(()), restart_required: AtomicBool::new(false) }
    }

    /// Ciò che mostra `GET /admin/api/llm`: della chiave solo da dove viene e le sue ultime
    /// quattro lettere, mai il valore.
    pub fn view(&self) -> serde_json::Value {
        let saved = LlmSettings::load(&self.path);
        let source = if saved.is_some() { "admin" } else { "config" };
        let settings = saved.unwrap_or_else(|| LlmSettings::from_config(&self.base));
        let key = match settings.api_key.as_deref().and_then(ApiKey::new) {
            Some(key) => Some(("admin", key)),
            None => self.base.api_key.clone().map(|key| ("config", key)),
        };
        let active = self.holder.get().map(|gateway| {
            let info = gateway.info();
            serde_json::json!({"provider": gateway.config().provider.as_str(), "model": info.model, "label": info.label, "external": info.external})
        });
        serde_json::json!({
            "source": source,
            "provider": settings.provider,
            "base_url": settings.base_url,
            "model": settings.model,
            "allowed_models": settings.allowed_models,
            "max_tokens_per_day_per_user": settings.max_tokens_per_day_per_user,
            "builtin_model": settings.builtin_model,
            "api_key": key.map(|(from, key)| serde_json::json!({"from": from, "hint": key_hint(&key)})),
            "active": active,
            "builtin_now": self.builtin.as_ref().map(|(model, _)| model.clone()),
            "restart_required": self.restart_required.load(Ordering::SeqCst),
        })
    }

    /// Salva le impostazioni e le applica: un errore di validazione non salva né cambia nulla.
    pub fn update(&self, input: LlmSettingsInput) -> Result<serde_json::Value, ConfigError> {
        let _one_at_a_time = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        let previous = LlmSettings::load(&self.path);
        let api_key = if input.remove_api_key {
            None
        } else {
            trimmed(input.api_key.as_deref()).or_else(|| previous.and_then(|p| p.api_key))
        };
        let settings = LlmSettings {
            provider: input.provider.trim().to_owned(),
            base_url: input.base_url,
            model: input.model,
            allowed_models: input.allowed_models,
            max_tokens_per_day_per_user: input.max_tokens_per_day_per_user,
            builtin_model: input.builtin_model,
            api_key,
        };
        let config = settings.applied_to(&self.base)?;
        let next = self.next_for(&config)?;
        settings
            .save(&self.path)
            .map_err(|_| invalid("llm_settings", "il file delle impostazioni non si scrive nella cartella dei dati"))?;
        self.switch(next);
        tracing::info!(provider = config.provider.as_str(), "impostazioni dell'assistente salvate dal pannello");
        Ok(self.view())
    }

    /// Cancella le impostazioni salvate: torna a valere la configurazione del file e dell'ambiente.
    pub fn reset(&self) -> Result<serde_json::Value, ConfigError> {
        let _one_at_a_time = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        let next = self.next_for(&self.base)?;
        match std::fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(invalid("llm_settings", "il file delle impostazioni non si cancella")),
        }
        self.switch(next);
        tracing::info!(provider = self.base.provider.as_str(), "impostazioni dell'assistente tolte dal pannello: vale la configurazione");
        Ok(self.view())
    }

    fn next_for(&self, config: &LlmConfig) -> Result<Next, ConfigError> {
        match config.provider {
            LlmProvider::None => Ok(Next::Run(None)),
            LlmProvider::OpenAiCompatible => {
                let gateway = LlmGateway::new(config, Arc::clone(&self.accounts), Arc::clone(&self.sink))?;
                Ok(Next::Run(Some(Arc::new(gateway))))
            }
            LlmProvider::Builtin => match &self.builtin {
                Some((model, supervisor)) if config.builtin_model.as_deref() == Some(model.as_str()) => {
                    let gateway = LlmGateway::with_supervisor(config, Arc::clone(&self.accounts), Arc::clone(&self.sink), Arc::clone(supervisor))?;
                    Ok(Next::Run(Some(Arc::new(gateway))))
                }
                _ => Ok(Next::Restart),
            },
        }
    }

    fn switch(&self, next: Next) {
        match next {
            Next::Run(gateway) => {
                // Il figlio locale non si ferma qui: il suo supervisore lo spegne dopo l'inattività.
                self.holder.replace(gateway);
                self.restart_required.store(false, Ordering::SeqCst);
            }
            Next::Restart => self.restart_required.store(true, Ordering::SeqCst),
        }
    }
}
