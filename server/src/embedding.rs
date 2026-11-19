//! Il modello di embedding scelto dal pannello `/admin` (`/admin/api/embedding`): quello locale
//! (EmbeddingGemma 2, predefinito) oppure un servizio compatibile con OpenAI su un indirizzo
//! interno.
//!
//! Ciò che protegge gli indici delle app:
//! - i vettori hanno sempre la lunghezza dell'indice ([`DIM`]): un servizio che ne restituisce
//!   un'altra non si salva (chiamata di prova) e, più tardi, non raggiunge mai un'app;
//! - il modello in uso è quello che `/health` dichiara: l'app che ha un indice fatto con un altro
//!   modello si ferma e propone di reindicizzare (spec del calcolo, D4). Se dietro lo stesso nome
//!   i vettori cambiano (un'altra build, un altro servizio), il nome dichiarato prende `#n`;
//! - dopo un cambio fatto dal pannello i vettori si fermano per un po' ([`EmbeddingHolder::switch`]):
//!   un messaggio che l'app aveva cominciato con il modello di prima non finisce con quello nuovo;
//! - con un modello remoto le immagini non si cercano: i loro vettori verrebbero da un altro modello.
//!
//! Il testo della posta resta sull'infrastruttura di chi gestisce il server: un indirizzo esterno
//! è rifiutato (anche un nome che si risolve in un indirizzo pubblico), perché l'app non può
//! ancora chiedere il consenso degli utenti. La spezzatura dei passaggi resta locale, con il
//! tokenizer di EmbeddingGemma 2.
//!
//! Nel log: mai chiavi, testi o vettori.

use std::fmt;
use std::net::{IpAddr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, TryLockError};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::compute::{ComputeBackend, ComputeError, DIM, MODEL_ID, ModelStatus};
use crate::config::is_local_or_private;
use crate::llm::{ApiKey, BaseUrl, key_hint, parse_base_url, save_private, trimmed};

/// Il file, nella cartella dei dati, con il modello di embedding scelto dal pannello. Scritto con
/// permessi 0600: può contenere la chiave del servizio.
pub const SETTINGS_FILE: &str = "embedding-settings.json";
pub const PROVIDER_BUILTIN: &str = "builtin";
pub const PROVIDER_OPENAI: &str = "openai-compatible";
/// L'app registra il nome dichiarato nell'indice in 47 byte: uno più lungo non coinciderebbe mai
/// con quello che il server dichiara, e l'app chiederebbe di reindicizzare senza fine. 40 per il
/// nome lasciano posto a `#n` ([`declared`]).
pub const MAX_MODEL_BYTES: usize = 40;
/// What the image route answers with a remote model (`document_unreadable`: that image only).
pub const IMAGES_UNAVAILABLE: &str = "Images are not searched with this server's embedding model.";

const TIMEOUT: Duration = Duration::from_secs(60);
/// La chiamata di prova finisce prima della richiesta del pannello (30 s), che altrimenti
/// direbbe «tempo scaduto» mentre il server cambia modello lo stesso.
const PROBE_TIMEOUT: Duration = Duration::from_secs(20);
const PROBE_UNREACHABLE: &str = "Il servizio non ha risposto entro 20 secondi a questo indirizzo: se sta ancora caricando il modello, riprova tra un minuto.";
const SAVE_IN_PROGRESS: &str = "Un altro salvataggio è in corso: riprova tra poco.";
/// Sotto questa somiglianza tra due vettori della stessa frase di prova, dietro il nome non c'è
/// più lo stesso modello.
const SAME_VECTORS: f64 = 0.99;
/// 64 testi di 256 valori sono qualche centinaio di KiB: oltre questo la risposta non è la nostra.
const MAX_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;
/// Pausa suggerita all'app quando il servizio non risponde o risponde male.
const FAILURE_RETRY_S: u64 = 60;
const BUSY_RETRY_S: u64 = 30;
const PROBE: &str = "Una frase di prova per il modello di embedding.";

/// Perché una chiamata al servizio non ha dato vettori buoni.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteError {
    Unreachable,
    KeyRefused,
    RateLimited { retry_after_s: Option<u64> },
    TooLarge,
    Status(u16),
    BadAnswer,
    /// Vettori di un'altra lunghezza (quella trovata).
    WrongLength(usize),
}

impl RemoteError {
    /// Per il log: mai un contenuto.
    fn label(&self) -> &'static str {
        match self {
            RemoteError::Unreachable => "unreachable",
            RemoteError::KeyRefused => "key_refused",
            RemoteError::RateLimited { .. } => "rate_limited",
            RemoteError::TooLarge => "too_large",
            RemoteError::Status(_) => "status",
            RemoteError::BadAnswer => "bad_answer",
            RemoteError::WrongLength(_) => "wrong_length",
        }
    }
}

/// Ciò che il pannello dice quando la chiamata di prova non riesce.
impl fmt::Display for RemoteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RemoteError::Unreachable => f.write_str("Il servizio non risponde a questo indirizzo."),
            RemoteError::KeyRefused => f.write_str("Il servizio ha rifiutato la chiave."),
            RemoteError::RateLimited { .. } => f.write_str("Il servizio è occupato: riprova tra poco."),
            RemoteError::TooLarge => f.write_str("Il servizio ha rifiutato la richiesta di prova perché troppo grande."),
            RemoteError::Status(status) => write!(f, "Il servizio ha risposto con l'errore {status}: controlla l'indirizzo e il nome del modello."),
            RemoteError::BadAnswer => f.write_str("La risposta del servizio non contiene vettori validi."),
            RemoteError::WrongLength(found) => write!(
                f,
                "Il modello ha restituito {found} valori per vettore: l'indice ne usa {DIM}. Scegli un modello che accetta `dimensions: {DIM}`."
            ),
        }
    }
}

/// Verso l'app: una pausa che non costa tentativi ai messaggi, tranne un testo troppo grande.
impl From<RemoteError> for ComputeError {
    fn from(error: RemoteError) -> Self {
        match error {
            RemoteError::TooLarge => ComputeError::PayloadTooLarge,
            RemoteError::RateLimited { retry_after_s } => ComputeError::RateLimited { retry_after_s: retry_after_s.unwrap_or(BUSY_RETRY_S).clamp(1, 600) },
            _ => ComputeError::PausedAfterFailure { retry_after_s: FAILURE_RETRY_S },
        }
    }
}

#[derive(Deserialize)]
struct Answer {
    data: Vec<Item>,
    #[serde(default)]
    usage: Option<AnswerUsage>,
}

#[derive(Deserialize)]
struct Item {
    #[serde(default)]
    index: Option<usize>,
    embedding: Vec<f32>,
}

#[derive(Deserialize)]
struct AnswerUsage {
    #[serde(default)]
    total_tokens: Option<u64>,
    #[serde(default)]
    prompt_tokens: Option<u64>,
}

/// Un vettore dell'indice: esattamente [`DIM`] valori finiti, portato a norma 1. Mai tagliato qui:
/// un vettore più lungo tagliato a mano non è quello che il modello avrebbe dato con `dimensions`.
fn normalized(vector: Vec<f32>) -> Result<Vec<f32>, RemoteError> {
    if vector.len() != DIM {
        return Err(RemoteError::WrongLength(vector.len()));
    }
    if vector.iter().any(|value| !value.is_finite()) {
        return Err(RemoteError::BadAnswer);
    }
    let norm = vector.iter().map(|value| f64::from(*value) * f64::from(*value)).sum::<f64>().sqrt();
    if !norm.is_finite() || norm <= f64::from(f32::EPSILON) {
        return Err(RemoteError::BadAnswer);
    }
    Ok(vector.into_iter().map(|value| (f64::from(value) / norm) as f32).collect())
}

/// I vettori di una risposta, nell'ordine degli ingressi, e i token dichiarati (0 se assenti).
fn parse_answer(raw: &[u8], count: usize) -> Result<(Vec<Vec<f32>>, u64), RemoteError> {
    let answer: Answer = serde_json::from_slice(raw).map_err(|_| RemoteError::BadAnswer)?;
    if answer.data.len() != count {
        return Err(RemoteError::BadAnswer);
    }
    let mut vectors: Vec<Option<Vec<f32>>> = vec![None; count];
    for (position, item) in answer.data.into_iter().enumerate() {
        let index = item.index.unwrap_or(position);
        if index >= count || vectors[index].is_some() {
            return Err(RemoteError::BadAnswer);
        }
        vectors[index] = Some(normalized(item.embedding)?);
    }
    let tokens = answer.usage.and_then(|usage| usage.total_tokens.or(usage.prompt_tokens)).unwrap_or(0);
    // Ogni posto è stato riempito una volta sola: `count` voci, indici distinti e sotto `count`.
    Ok((vectors.into_iter().flatten().collect(), tokens))
}

/// Il nome che `/health` dichiara e l'app registra: quello del modello, con `#n` da quando dietro
/// quel nome i vettori sono cambiati `n` volte.
fn declared(model: &str, generation: u32) -> String {
    if generation == 0 { model.to_owned() } else { format!("{model}#{generation}") }
}

/// Quanto si somigliano due vettori già a norma 1.
fn cosine(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() {
        return 0.0;
    }
    a.iter().zip(b).map(|(x, y)| f64::from(*x) * f64::from(*y)).sum()
}

/// Almeno un indirizzo, e tutti di loopback o privati.
fn all_internal(addresses: &[IpAddr]) -> bool {
    !addresses.is_empty() && addresses.iter().all(|ip| is_local_or_private(*ip))
}

/// Un nome è interno se lo sono gli indirizzi in cui si risolve, non per come è scritto: in un
/// container con un dominio di ricerca, `embeddings` può diventare un host pubblico.
fn resolves_internally(base: &BaseUrl) -> Result<(), String> {
    if base.host.parse::<IpAddr>().is_ok() {
        return Ok(());
    }
    let port = base.port.unwrap_or(if base.https { 443 } else { 80 });
    let addresses: Vec<IpAddr> = (base.host.as_str(), port).to_socket_addrs().map(|found| found.map(|address| address.ip()).collect()).unwrap_or_default();
    if addresses.is_empty() {
        return Err(format!("Il nome «{}» non si risolve da questo server.", base.host));
    }
    if !all_internal(&addresses) {
        return Err(format!("Il nome «{}» si risolve in un indirizzo pubblico: il testo della posta uscirebbe dal server.", base.host));
    }
    Ok(())
}

/// Il servizio compatibile con OpenAI che calcola i vettori (`POST <base>/embeddings`).
pub struct RemoteEmbedder {
    url: String,
    /// Il nome per il servizio.
    model: String,
    /// Il nome per le app ([`declared`]).
    identity: String,
    key: Option<ApiKey>,
    agent: ureq::Agent,
    /// L'ultima chiamata non riuscita, finché una riesce: ciò che il pannello mostra.
    last_error: Mutex<Option<(String, chrono::DateTime<chrono::Utc>)>>,
}

impl fmt::Debug for RemoteEmbedder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RemoteEmbedder").field("model", &self.identity).finish_non_exhaustive()
    }
}

impl RemoteEmbedder {
    fn new(url: String, model: String, identity: String, key: Option<ApiKey>, timeout: Duration) -> RemoteEmbedder {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .max_redirects(0)
            .max_redirects_will_error(false)
            .http_status_as_error(false)
            // Mai il proxy dell'ambiente (HTTP_PROXY...): l'indirizzo lo sceglie l'operatore.
            .proxy(None)
            .build()
            .into();
        RemoteEmbedder { url, model, identity, key, agent, last_error: Mutex::new(None) }
    }

    /// Il nome che le app vedono.
    pub fn identity(&self) -> &str {
        &self.identity
    }

    fn last_error(&self) -> Option<(String, chrono::DateTime<chrono::Utc>)> {
        self.last_error.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// I vettori per la richiesta di un'app. Un errore del servizio è una pausa che non costa
    /// tentativi, e resta scritto per il pannello finché una chiamata riesce.
    pub fn serve(&self, inputs: &[String]) -> Result<(Vec<Vec<f32>>, u64), ComputeError> {
        let failure = match self.embed(inputs) {
            Ok(answer) => {
                *self.last_error.lock().unwrap_or_else(|p| p.into_inner()) = None;
                return Ok(answer);
            }
            // Il servizio rifiuta questi ingressi ma non la frase di prova: il problema è il
            // contenuto (un passaggio troppo lungo per il modello), e l'app lo mette da parte.
            // Una pausa, invece, lo farebbe riprovare senza fine.
            Err(RemoteError::Status(400 | 422)) if self.embed(&[PROBE.to_owned()]).is_ok() => return Err(ComputeError::PayloadTooLarge),
            Err(failure) => failure,
        };
        tracing::warn!(reason = failure.label(), "il servizio di embedding non ha dato vettori validi");
        *self.last_error.lock().unwrap_or_else(|p| p.into_inner()) = Some((failure.to_string(), chrono::Utc::now()));
        Err(ComputeError::from(failure))
    }

    /// Un vettore a norma 1 di [`DIM`] valori per ingresso, e i token degli ingressi.
    fn embed(&self, inputs: &[String]) -> Result<(Vec<Vec<f32>>, u64), RemoteError> {
        let body = serde_json::to_vec(&serde_json::json!({
            "model": self.model,
            "input": inputs,
            "dimensions": DIM,
            "encoding_format": "float",
        }))
        .map_err(|_| RemoteError::BadAnswer)?;
        let mut request = self
            .agent
            .post(&self.url)
            .header("Content-Type", "application/json")
            .header("User-Agent", concat!("qamille-server/", env!("CARGO_PKG_VERSION")));
        if let Some(key) = &self.key {
            request = request.header("Authorization", format!("Bearer {}", key.expose()));
        }
        let mut response = request.send(&body[..]).map_err(|_| RemoteError::Unreachable)?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(match status {
                401 | 403 => RemoteError::KeyRefused,
                413 => RemoteError::TooLarge,
                429 => RemoteError::RateLimited {
                    retry_after_s: response
                        .headers()
                        .get("retry-after")
                        .and_then(|value| value.to_str().ok())
                        .and_then(|value| value.trim().parse::<u64>().ok()),
                },
                // Anche un redirect: non si segue mai.
                other => RemoteError::Status(other),
            });
        }
        let raw = response
            .body_mut()
            .with_config()
            .limit(MAX_RESPONSE_BYTES)
            .read_to_vec()
            .map_err(|_| RemoteError::BadAnswer)?;
        let (vectors, tokens) = parse_answer(&raw, inputs.len())?;
        if tokens > 0 {
            return Ok((vectors, tokens));
        }
        // shortcut: a service that declares no usage is counted as one token every four bytes, an estimate and not its tokenizer; count with the service's own tokenizer when a plan's quota must match it
        let bytes: u64 = inputs.iter().map(|input| input.len() as u64).sum();
        Ok((vectors, bytes.div_ceil(4)))
    }
}

/// Il servizio remoto in uso, sostituito a caldo dal pannello; `None`: il modello locale. Una
/// richiesta già partita finisce con il modello che aveva.
#[derive(Clone, Default)]
pub struct EmbeddingHolder(Arc<Held>);

#[derive(Default)]
struct Held {
    remote: RwLock<Option<Arc<RemoteEmbedder>>>,
    /// Fino a quando, dopo un cambio di modello dal pannello, non si danno vettori.
    quiet_until: Mutex<Option<Instant>>,
}

impl EmbeddingHolder {
    pub fn get(&self) -> Option<Arc<RemoteEmbedder>> {
        self.0.remote.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// Senza pausa: all'avvio, o quando il modello dichiarato resta lo stesso.
    fn replace(&self, remote: Option<Arc<RemoteEmbedder>>) {
        *self.0.remote.write().unwrap_or_else(|p| p.into_inner()) = remote;
    }

    /// Un altro modello, a server acceso. L'app controlla il modello una volta per messaggio e
    /// poi chiede i vettori a lotti: per `pause` nessun vettore, così un messaggio cominciato
    /// prima del cambio si ferma (una pausa che non gli costa tentativi) invece di finire con
    /// vettori di due modelli sotto il nome del primo. Quando riprova, vede il modello nuovo.
    fn switch(&self, remote: Option<Arc<RemoteEmbedder>>, pause: Duration) {
        *self.0.quiet_until.lock().unwrap_or_else(|p| p.into_inner()) = (!pause.is_zero()).then(|| Instant::now() + pause);
        self.replace(remote);
    }

    /// Dopo un cambio di modello, finché dura la pausa: `rate_limited` con i secondi che mancano.
    fn paused(&self) -> Result<(), ComputeError> {
        let until = *self.0.quiet_until.lock().unwrap_or_else(|p| p.into_inner());
        match until.and_then(|until| until.checked_duration_since(Instant::now())) {
            Some(left) => Err(ComputeError::RateLimited { retry_after_s: left.as_secs().clamp(1, 600) }),
            None => Ok(()),
        }
    }
}

/// Il calcolo locale, con i vettori dei testi dal servizio remoto quando il pannello ne ha scelto uno.
pub struct SwitchableBackend {
    local: Arc<dyn ComputeBackend>,
    remote: EmbeddingHolder,
}

impl SwitchableBackend {
    pub fn new(local: Arc<dyn ComputeBackend>, remote: EmbeddingHolder) -> SwitchableBackend {
        SwitchableBackend { local, remote }
    }
}

impl ComputeBackend for SwitchableBackend {
    /// Sempre locale: il tokenizer di EmbeddingGemma 2 taglia i passaggi per ogni modello.
    fn chunk(&self, text: &str) -> Result<Vec<String>, ComputeError> {
        self.local.chunk(text)
    }

    fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>, ComputeError> {
        self.embed_counted(inputs).map(|(vectors, _)| vectors)
    }

    fn embed_counted(&self, inputs: &[String]) -> Result<(Vec<Vec<f32>>, u64), ComputeError> {
        self.remote.paused()?;
        let Some(remote) = self.remote.get() else {
            return self.local.embed_counted(inputs);
        };
        // shortcut: remote calls wait at the local model's gate (one at a time), give the remote provider its own gate when a server needs parallel batches
        remote.serve(inputs)
    }

    fn embed_image(&self, bytes: &[u8]) -> Result<Vec<f32>, ComputeError> {
        if self.remote.get().is_some() {
            return Err(ComputeError::DocumentUnreadable(IMAGES_UNAVAILABLE.to_owned()));
        }
        self.remote.paused()?;
        self.local.embed_image(bytes)
    }

    fn extract(&self, name: &str, bytes: &[u8]) -> Result<String, ComputeError> {
        self.local.extract(name, bytes)
    }

    /// Il modello locale e, accanto, quello remoto in uso: l'app cerca tra questi il modello che
    /// `/health` dichiara e lo vuole pronto. Pronto solo se lo è anche quello locale, il cui
    /// tokenizer taglia i passaggi: senza, l'app deve vedere che manca un modello.
    fn models(&self) -> Vec<ModelStatus> {
        let mut models = self.local.models();
        if let Some(remote) = self.remote.get() {
            let local = models.iter().find(|model| model.id == MODEL_ID).map_or("missing", |model| model.state.as_str());
            let state = if matches!(local, "ready" | "loading") { "ready" } else { local }.to_owned();
            models.push(ModelStatus {
                id: remote.identity().to_owned(),
                kind: "embedding".to_owned(),
                state,
                percent: None,
                done_bytes: 0,
                total_bytes: 0,
                error: None,
                loaded: false,
                dimensions: DIM,
                quantization: String::new(),
            });
        }
        models
    }

    fn install(&self, id: &str) -> Result<ModelStatus, ComputeError> {
        self.local.install(id)
    }

    fn release_idle(&self) {
        self.local.release_idle();
    }

    fn shutdown(&self) {
        self.local.shutdown();
    }

    fn embedding_identity(&self) -> (String, String) {
        match self.remote.get() {
            Some(remote) => (remote.identity().to_owned(), String::new()),
            None => self.local.embedding_identity(),
        }
    }
}

/// Le impostazioni salvate dal pannello: solo per un servizio remoto (il modello locale non salva nulla).
#[derive(Clone, Serialize, Deserialize)]
struct EmbeddingSettings {
    provider: String,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    /// Quante volte, dietro questo nome, i vettori sono cambiati ([`declared`]).
    #[serde(default)]
    generation: u32,
    /// Il vettore della frase di prova all'ultimo salvataggio: il confronto per il prossimo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    probe: Option<Vec<f32>>,
}

impl EmbeddingSettings {
    /// Nessun file: `None`. Un file illeggibile o non valido vale come assente, con un avviso
    /// (mai il suo contenuto nel log).
    fn load(path: &Path) -> Option<EmbeddingSettings> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return None,
            Err(_) => {
                tracing::warn!(file = SETTINGS_FILE, "impostazioni del modello di embedding illeggibili: vale il modello locale");
                return None;
            }
        };
        match serde_json::from_slice(&bytes) {
            Ok(settings) => Some(settings),
            Err(_) => {
                tracing::warn!(file = SETTINGS_FILE, "impostazioni del modello di embedding non valide: vale il modello locale");
                None
            }
        }
    }

    /// Il servizio che queste impostazioni descrivono, o perché non si possono usare.
    fn embedder(&self, timeout: Duration) -> Result<RemoteEmbedder, String> {
        let (base, model) = self.checked()?;
        let key = self.api_key.as_deref().and_then(ApiKey::new);
        let identity = declared(&model, self.generation);
        Ok(RemoteEmbedder::new(base.endpoint("/embeddings"), model, identity, key, timeout))
    }

    /// L'indirizzo e il nome del modello, se rispettano le regole.
    fn checked(&self) -> Result<(BaseUrl, String), String> {
        if self.provider != PROVIDER_OPENAI {
            return Err(format!("Il provider è «{PROVIDER_BUILTIN}» o «{PROVIDER_OPENAI}»."));
        }
        let address = trimmed(self.base_url.as_deref()).ok_or("Manca l'indirizzo del servizio.")?;
        let base = parse_base_url(&address).map_err(|reason| format!("Indirizzo non valido: {reason}."))?;
        if !base.is_internal() {
            return Err("Solo un indirizzo interno (questo server, la rete privata, un servizio di compose): con un servizio esterno il testo della posta uscirebbe dal server, e l'app non può ancora chiedere il consenso degli utenti.".to_owned());
        }
        let model = trimmed(self.model.as_deref()).ok_or("Manca il nome del modello.")?;
        if model.len() > MAX_MODEL_BYTES || !model.bytes().all(|b| b.is_ascii_graphic() && b != b'#') {
            return Err(format!("Il nome del modello può avere al massimo {MAX_MODEL_BYTES} caratteri ASCII, senza spazi né «#»: è il nome che l'app registra nell'indice."));
        }
        // Lo stesso nome di un modello di questo server: l'indice non distinguerebbe i due.
        if model == MODEL_ID || model.starts_with(crate::compute::speech::ID_PREFIX) || model.starts_with(qamille_compute::llm_catalog::ID_PREFIX) {
            return Err(format!("Il nome «{model}» è riservato ai modelli di questo server: l'indice non distinguerebbe i due modelli."));
        }
        Ok((base, model))
    }
}

/// Il corpo di `PUT /admin/api/embedding`. Mai in un `Debug`: può portare la chiave.
#[derive(Deserialize)]
pub struct EmbeddingSettingsInput {
    pub provider: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// Una chiave nuova; vuota o assente: resta quella già salvata.
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub remove_api_key: bool,
}

/// Il modello di embedding dal pannello: cosa è salvato, cosa gira, e il cambio a caldo.
pub struct EmbeddingAdmin {
    holder: EmbeddingHolder,
    local: Arc<dyn ComputeBackend>,
    path: PathBuf,
    /// Quanto restano fermi i vettori dopo un cambio di modello ([`EmbeddingHolder::switch`]).
    pause: Duration,
    /// Un salvataggio alla volta.
    lock: Mutex<()>,
}

impl fmt::Debug for EmbeddingAdmin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EmbeddingAdmin").field("path", &self.path).finish_non_exhaustive()
    }
}

impl EmbeddingAdmin {
    /// Con le impostazioni salvate in `path` già applicate, senza chiamata di prova: il servizio
    /// può non essere ancora partito. Impostazioni non più valide: vale il modello locale.
    pub fn new(holder: EmbeddingHolder, local: Arc<dyn ComputeBackend>, path: PathBuf, pause: Duration) -> EmbeddingAdmin {
        if let Some(settings) = EmbeddingSettings::load(&path) {
            match settings.embedder(TIMEOUT) {
                Ok(remote) => {
                    tracing::info!(model = remote.identity(), "modello di embedding remoto scelto dal pannello");
                    holder.replace(Some(Arc::new(remote)));
                }
                Err(_) => tracing::error!(file = SETTINGS_FILE, "impostazioni del modello di embedding non valide: vale il modello locale"),
            }
        }
        EmbeddingAdmin { holder, local, path, pause, lock: Mutex::new(()) }
    }

    /// Un salvataggio alla volta; il secondo non aspetta il primo (la sua prova può durare 20 s).
    fn one_at_a_time(&self) -> Result<MutexGuard<'_, ()>, String> {
        match self.lock.try_lock() {
            Ok(guard) => Ok(guard),
            Err(TryLockError::Poisoned(poisoned)) => Ok(poisoned.into_inner()),
            Err(TryLockError::WouldBlock) => Err(SAVE_IN_PROGRESS.to_owned()),
        }
    }

    /// Mette in uso `remote` (`None`: il modello locale). Con un altro nome dichiarato, i vettori
    /// si fermano per la pausa.
    fn put_in_use(&self, remote: Option<Arc<RemoteEmbedder>>) {
        let name = |remote: &Option<Arc<RemoteEmbedder>>| remote.as_ref().map_or_else(|| MODEL_ID.to_owned(), |remote| remote.identity().to_owned());
        if name(&self.holder.get()) == name(&remote) {
            self.holder.replace(remote);
        } else {
            self.holder.switch(remote, self.pause);
        }
    }

    fn builtin_state(&self) -> String {
        self.local.models().into_iter().find(|model| model.id == MODEL_ID).map_or_else(|| "missing".to_owned(), |model| model.state)
    }

    /// Ciò che mostra `GET /admin/api/embedding`: della chiave solo le ultime quattro lettere.
    pub fn view(&self) -> serde_json::Value {
        // Impostazioni salvate ma non applicate (non più valide) non si mostrano: gira il modello locale.
        let active = self.holder.get();
        let saved = EmbeddingSettings::load(&self.path).filter(|_| active.is_some());
        let key = saved.as_ref().and_then(|settings| settings.api_key.as_deref()).and_then(ApiKey::new);
        serde_json::json!({
            "provider": if saved.is_some() { PROVIDER_OPENAI } else { PROVIDER_BUILTIN },
            "base_url": saved.as_ref().and_then(|settings| trimmed(settings.base_url.as_deref())),
            "model": saved.as_ref().and_then(|settings| trimmed(settings.model.as_deref())),
            "api_key": key.map(|key| serde_json::json!({"hint": key_hint(&key)})),
            "dimensions": DIM,
            "active": {
                "model": active.as_ref().map_or_else(|| MODEL_ID.to_owned(), |remote| remote.identity().to_owned()),
                "dimensions": DIM,
            },
            "builtin": {"model": MODEL_ID, "state": self.builtin_state()},
            // Mai una chiave, un testo o un vettore: solo che cosa non ha funzionato e quando.
            "last_error": active.and_then(|remote| remote.last_error()).map(|(message, at)| serde_json::json!({"message": message, "at": at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)})),
        })
    }

    /// Prova il servizio, poi salva e applica: se la prova non riesce non cambia nulla.
    pub fn update(&self, input: EmbeddingSettingsInput) -> Result<serde_json::Value, String> {
        let _one_at_a_time = self.one_at_a_time()?;
        let provider = input.provider.trim();
        if provider == PROVIDER_BUILTIN {
            return self.back_to_builtin();
        }
        let previous = EmbeddingSettings::load(&self.path);
        let base_url = trimmed(input.base_url.as_deref());
        let api_key = if input.remove_api_key {
            None
        } else {
            // La chiave salvata vale solo per l'indirizzo per cui è stata scritta: a un altro
            // indirizzo non si manda.
            trimmed(input.api_key.as_deref()).or_else(|| {
                previous.as_ref().filter(|saved| trimmed(saved.base_url.as_deref()) == base_url).and_then(|saved| saved.api_key.clone())
            })
        };
        let mut settings = EmbeddingSettings { provider: provider.to_owned(), base_url, model: trimmed(input.model.as_deref()), api_key, generation: 0, probe: None };
        let (base, model) = settings.checked()?;
        if !matches!(self.builtin_state().as_str(), "ready" | "loading") {
            return Err(format!("Installa prima il modello locale «{MODEL_ID}» dalla sezione «Modelli»: il suo tokenizer taglia i passaggi anche con un modello remoto."));
        }
        resolves_internally(&base)?;
        // Un testo fisso nostro, mai la posta di qualcuno.
        let (vectors, _) = settings.embedder(PROBE_TIMEOUT)?.embed(&[PROBE.to_owned()]).map_err(|error| match error {
            RemoteError::Unreachable => PROBE_UNREACHABLE.to_owned(),
            other => other.to_string(),
        })?;
        let probe = vectors.into_iter().next().ok_or_else(|| RemoteError::BadAnswer.to_string())?;
        // Lo stesso nome di prima: se i vettori non sono più gli stessi, per le app è un altro modello.
        if let Some(saved) = previous.filter(|saved| trimmed(saved.model.as_deref()).as_deref() == Some(model.as_str())) {
            let changed = saved.probe.as_ref().is_some_and(|old| cosine(old, &probe) < SAME_VECTORS);
            settings.generation = if changed { saved.generation.saturating_add(1) } else { saved.generation };
        }
        settings.probe = Some(probe);
        let remote = settings.embedder(TIMEOUT)?;
        let body = serde_json::to_vec_pretty(&settings).map_err(|_| "Le impostazioni non si possono salvare.".to_owned())?;
        save_private(&self.path, &body).map_err(|_| "Il file delle impostazioni non si scrive nella cartella dei dati.".to_owned())?;
        tracing::info!(model = remote.identity(), "modello di embedding remoto salvato dal pannello");
        self.put_in_use(Some(Arc::new(remote)));
        Ok(self.view())
    }

    /// Cancella le impostazioni salvate: torna il modello locale.
    pub fn reset(&self) -> Result<serde_json::Value, String> {
        let _one_at_a_time = self.one_at_a_time()?;
        self.back_to_builtin()
    }

    fn back_to_builtin(&self) -> Result<serde_json::Value, String> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("Il file delle impostazioni non si cancella.".to_owned()),
        }
        if self.holder.get().is_some() {
            tracing::info!(model = MODEL_ID, "modello di embedding locale ripristinato dal pannello");
        }
        self.put_in_use(None);
        Ok(self.view())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(vectors: &[Vec<f32>]) -> Vec<u8> {
        let data: Vec<_> = vectors.iter().enumerate().map(|(index, embedding)| serde_json::json!({"index": index, "embedding": embedding})).collect();
        serde_json::to_vec(&serde_json::json!({"data": data, "usage": {"prompt_tokens": 7}})).unwrap()
    }

    #[test]
    fn only_vectors_of_the_index_length_pass_and_leave_normalized() {
        let (vectors, tokens) = parse_answer(&answer(&[vec![2.0; DIM]]), 1).unwrap();
        assert_eq!(tokens, 7);
        assert_eq!(vectors[0].len(), DIM);
        let norm: f32 = vectors[0].iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-4, "{norm}");
        assert_eq!(parse_answer(&answer(&[vec![1.0; 768]]), 1), Err(RemoteError::WrongLength(768)));
        assert_eq!(parse_answer(&answer(&[vec![1.0; DIM - 1]]), 1), Err(RemoteError::WrongLength(DIM - 1)));
        // Un vettore nullo non ha direzione; un vettore in meno non è la risposta alla richiesta.
        assert_eq!(parse_answer(&answer(&[vec![0.0; DIM]]), 1), Err(RemoteError::BadAnswer));
        assert_eq!(parse_answer(&answer(&[vec![1.0; DIM]]), 2), Err(RemoteError::BadAnswer));
        assert_eq!(parse_answer(b"{\"data\":[{\"embedding\":\"AAAA\"}]}", 1), Err(RemoteError::BadAnswer));
    }

    #[test]
    fn the_answer_follows_the_declared_indexes() {
        let mut first = vec![0.0; DIM];
        first[0] = 3.0;
        let mut second = vec![0.0; DIM];
        second[1] = 5.0;
        let raw = serde_json::to_vec(&serde_json::json!({"data": [{"index": 1, "embedding": second}, {"index": 0, "embedding": first}]})).unwrap();
        let (vectors, tokens) = parse_answer(&raw, 2).unwrap();
        assert_eq!((vectors[0][0], vectors[1][1], tokens), (1.0, 1.0, 0));
        let twice = serde_json::to_vec(&serde_json::json!({"data": [{"index": 0, "embedding": vec![1.0; DIM]}, {"index": 0, "embedding": vec![1.0; DIM]}]})).unwrap();
        assert_eq!(parse_answer(&twice, 2), Err(RemoteError::BadAnswer));
    }

    fn settings(base_url: &str, model: &str) -> EmbeddingSettings {
        EmbeddingSettings { provider: PROVIDER_OPENAI.to_owned(), base_url: Some(base_url.to_owned()), model: Some(model.to_owned()), api_key: None, generation: 0, probe: None }
    }

    #[test]
    fn only_an_internal_address_and_a_name_the_index_can_hold() {
        assert!(settings("http://127.0.0.1:11434/v1", "nomic-embed-text").checked().is_ok());
        assert!(settings("http://ollama:11434/v1", "bge-m3").checked().is_ok());
        for external in ["https://api.openai.com/v1", "http://8.8.8.8/v1", "https://embeddings.example.com"] {
            let refused = settings(external, "text-embedding-3-small").checked().unwrap_err();
            assert!(refused.contains("indirizzo interno"), "{external}: {refused}");
        }
        let long = "m".repeat(MAX_MODEL_BYTES + 1);
        for name in [MODEL_ID, "whisper-base", "llm-medio", "due parole", long.as_str(), "modèle", "bge#1"] {
            assert!(settings("http://127.0.0.1:1/v1", name).checked().is_err(), "{name}");
        }
        assert!(settings("http://127.0.0.1:1/v1", &"m".repeat(MAX_MODEL_BYTES)).checked().is_ok());
        // Il nome dichiarato, anche con il suo numero, sta nei 47 byte dell'indice dell'app.
        assert!(declared(&"m".repeat(MAX_MODEL_BYTES), 999_999).len() <= 47);
        assert_eq!((declared("bge-m3", 0), declared("bge-m3", 2)), ("bge-m3".to_owned(), "bge-m3#2".to_owned()));
    }

    #[test]
    fn a_name_is_internal_by_where_it_resolves() {
        let private: IpAddr = "10.0.0.5".parse().unwrap();
        let public: IpAddr = "93.184.216.34".parse().unwrap();
        assert!(all_internal(&[private, "127.0.0.1".parse().unwrap(), "fd00::1".parse().unwrap()]));
        // Un solo indirizzo pubblico tra quelli del nome basta a rifiutarlo; nessun indirizzo, anche.
        assert!(!all_internal(&[private, public]));
        assert!(!all_internal(&[]));
        assert!(resolves_internally(&parse_base_url("http://localhost:11434/v1").unwrap()).is_ok());
        assert!(resolves_internally(&parse_base_url("http://10.0.0.5:11434/v1").unwrap()).is_ok());
    }

    #[test]
    fn the_pause_after_a_switch_ends_and_a_plain_replace_has_none() {
        let holder = EmbeddingHolder::default();
        holder.replace(None);
        assert!(holder.paused().is_ok());
        holder.switch(None, Duration::from_secs(660));
        assert!(matches!(holder.paused(), Err(ComputeError::RateLimited { retry_after_s }) if (1..=600).contains(&retry_after_s)));
        holder.switch(None, Duration::ZERO);
        assert!(holder.paused().is_ok());
    }

    #[test]
    fn two_vectors_of_one_sentence_tell_if_the_model_is_the_same() {
        let mut a = vec![0.0f32; DIM];
        a[0] = 1.0;
        let mut b = vec![0.0f32; DIM];
        b[1] = 1.0;
        assert!(cosine(&a, &a) >= SAME_VECTORS);
        assert!(cosine(&a, &b) < SAME_VECTORS);
        assert!(cosine(&a, &b[..8]) < SAME_VECTORS);
    }
}
