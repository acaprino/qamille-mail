//! Il modello di embedding scelto dal pannello `/admin` (`/admin/api/embedding`): quello locale
//! (EmbeddingGemma 2, predefinito) oppure un servizio compatibile con OpenAI su un indirizzo
//! interno.
//!
//! Ciò che protegge gli indici delle app:
//! - i vettori hanno sempre la lunghezza dell'indice ([`DIM`]): un servizio che ne restituisce
//!   un'altra non si salva (chiamata di prova) e, più tardi, non raggiunge mai un'app;
//! - il modello in uso è quello che `/health` dichiara: l'app che ha un indice fatto con un altro
//!   modello si ferma e propone di reindicizzare (spec del calcolo, D4). Se dietro lo stesso nome
//!   i vettori cambiano (un'altra build, un altro servizio), il nome dichiarato prende `#n`, e lo
//!   tiene anche se nel frattempo si torna al modello locale o se ne salva un altro ([`RECORDS_FILE`]);
//! - dopo un cambio fatto dal pannello i vettori si fermano per un po' ([`EmbeddingHolder::switch`],
//!   anche dopo un riavvio): un messaggio che l'app aveva cominciato con il modello di prima non
//!   finisce con quello nuovo; le app di questa versione mandano anche il modello che hanno
//!   controllato ([`crate::compute::EXPECTED_MODEL_HEADER`]);
//! - con un modello remoto le immagini non si cercano: i loro vettori verrebbero da un altro modello.
//!
//! Il testo della posta resta sull'infrastruttura di chi gestisce il server: un indirizzo esterno
//! è rifiutato, e a ogni connessione il nome deve risolversi in indirizzi privati ([`InternalOnly`]),
//! perché l'app non può ancora chiedere il consenso degli utenti. La spezzatura dei passaggi resta
//! locale, con il tokenizer di EmbeddingGemma 2.
//!
//! Nel log: mai chiavi, testi o vettori.

use std::collections::BTreeMap;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard, TryLockError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use ureq::http::Uri;
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};

use crate::compute::{ComputeBackend, ComputeError, DIM, MODEL_ID, ModelStatus, expected_model};
use crate::config::is_local_or_private;
use crate::llm::{ApiKey, BaseUrl, key_hint, parse_base_url, save_private, trimmed};

/// Il file, nella cartella dei dati, con il modello di embedding scelto dal pannello. Scritto con
/// permessi 0600: può contenere la chiave del servizio.
pub const SETTINGS_FILE: &str = "embedding-settings.json";
/// Il file, accanto, di ciò che si ricorda dei modelli remoti salvati, per nome: quante volte i
/// vettori dietro quel nome sono cambiati e il vettore della frase di prova; e la fine della pausa
/// dopo l'ultimo cambio. Senza segreti, e non si cancella tornando al modello locale: il `#n` di
/// un nome resta anche se nel frattempo si è usato un altro modello.
pub const RECORDS_FILE: &str = "embedding-models.json";
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
    /// Il nome del servizio (l'host) non si risolve.
    NotResolved(String),
    /// Il nome del servizio si risolve (anche) in un indirizzo pubblico: nessuna connessione.
    PublicAddress(String),
}

impl RemoteError {
    /// Per il log: mai un contenuto.
    fn label(&self) -> &'static str {
        match self {
            RemoteError::NotResolved(_) => "not_resolved",
            RemoteError::PublicAddress(_) => "public_address",
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
            RemoteError::NotResolved(host) => write!(f, "Il nome «{host}» non si risolve da questo server."),
            RemoteError::PublicAddress(host) => {
                write!(f, "Il nome «{host}» si risolve in un indirizzo pubblico: il testo della posta uscirebbe dal server.")
            }
        }
    }
}

/// Il resolver del client verso il servizio: ogni connessione (la prova, l'avvio, ogni richiesta
/// dopo) va solo a indirizzi di loopback o privati. Un nome è interno per dove si risolve, non per
/// come è scritto: in un container con un dominio di ricerca `embeddings` può diventare un host
/// pubblico, anche giorni dopo il salvataggio. La ricerca del nome sta nel tempo della chiamata.
#[derive(Debug, Default)]
struct InternalOnly<R = DefaultResolver>(R);

/// Il segno, dentro l'errore del client, di un nome che si risolve in un indirizzo pubblico.
#[derive(Debug)]
struct PublicAddressFound;

impl fmt::Display for PublicAddressFound {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the service name resolves to a public address")
    }
}

impl std::error::Error for PublicAddressFound {}

impl<R: Resolver> Resolver for InternalOnly<R> {
    fn resolve(&self, uri: &Uri, config: &ureq::config::Config, timeout: NextTimeout) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let found = self.0.resolve(uri, config, timeout)?;
        if found.is_empty() {
            return Err(ureq::Error::HostNotFound);
        }
        // Un solo indirizzo pubblico tra quelli del nome basta: il client potrebbe usare quello.
        if found.iter().any(|address| !is_local_or_private(address.ip())) {
            return Err(ureq::Error::Io(io::Error::new(io::ErrorKind::PermissionDenied, PublicAddressFound)));
        }
        Ok(found)
    }
}

/// Perché la richiesta non è partita: il nome, poi tutto il resto (nessun dettaglio di rete).
fn transport_error(error: &ureq::Error, host: &str) -> RemoteError {
    match error {
        ureq::Error::HostNotFound => RemoteError::NotResolved(host.to_owned()),
        ureq::Error::Io(io) if io.get_ref().is_some_and(|inner| inner.is::<PublicAddressFound>()) => RemoteError::PublicAddress(host.to_owned()),
        _ => RemoteError::Unreachable,
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

/// Il servizio compatibile con OpenAI che calcola i vettori (`POST <base>/embeddings`).
pub struct RemoteEmbedder {
    url: String,
    /// L'host dell'indirizzo, per dire quale nome non si risolve.
    host: String,
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
    fn new(base: &BaseUrl, model: String, identity: String, key: Option<ApiKey>, timeout: Duration) -> RemoteEmbedder {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .max_redirects(0)
            .max_redirects_will_error(false)
            .http_status_as_error(false)
            // Mai il proxy dell'ambiente (HTTP_PROXY...): l'indirizzo lo sceglie l'operatore.
            .proxy(None)
            .build();
        let agent = ureq::Agent::with_parts(config, DefaultConnector::new(), InternalOnly::<DefaultResolver>::default());
        RemoteEmbedder { url: base.endpoint("/embeddings"), host: base.host.clone(), model, identity, key, agent, last_error: Mutex::new(None) }
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
        let mut response = request.send(&body[..]).map_err(|error| transport_error(&error, &self.host))?;
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
/// Il servizio e la pausa stanno sotto un solo lucchetto: una richiesta li legge una volta
/// ([`Self::in_use`]) e usa per i vettori lo stesso modello con cui li etichetta.
#[derive(Clone, Default)]
pub struct EmbeddingHolder(Arc<RwLock<Held>>);

#[derive(Default)]
struct Held {
    remote: Option<Arc<RemoteEmbedder>>,
    /// Fino a quando, dopo un cambio di modello dal pannello, non si danno vettori.
    quiet_until: Option<Instant>,
}

impl EmbeddingHolder {
    fn read(&self) -> RwLockReadGuard<'_, Held> {
        self.0.read().unwrap_or_else(|p| p.into_inner())
    }

    fn write(&self) -> RwLockWriteGuard<'_, Held> {
        self.0.write().unwrap_or_else(|p| p.into_inner())
    }

    pub fn get(&self) -> Option<Arc<RemoteEmbedder>> {
        self.read().remote.clone()
    }

    /// Una sola lettura per richiesta: il servizio che fa i vettori (nessuno: il modello locale),
    /// oppure, finché dura la pausa dopo un cambio, `rate_limited` con i secondi che mancano.
    fn in_use(&self) -> Result<Option<Arc<RemoteEmbedder>>, ComputeError> {
        let held = self.read();
        if let Some(left) = held.quiet_until.and_then(|until| until.checked_duration_since(Instant::now())) {
            return Err(ComputeError::RateLimited { retry_after_s: left.as_secs().clamp(1, 600) });
        }
        Ok(held.remote.clone())
    }

    /// Senza pausa: all'avvio, o quando il modello dichiarato resta lo stesso.
    fn replace(&self, remote: Option<Arc<RemoteEmbedder>>) {
        self.write().remote = remote;
    }

    /// Un altro modello, a server acceso. L'app controlla il modello una volta per messaggio e
    /// poi chiede i vettori a lotti: per `pause` nessun vettore, così un messaggio cominciato
    /// prima del cambio si ferma (una pausa che non gli costa tentativi) invece di finire con
    /// vettori di due modelli sotto il nome del primo. Quando riprova, vede il modello nuovo.
    fn switch(&self, remote: Option<Arc<RemoteEmbedder>>, pause: Duration) {
        let mut held = self.write();
        held.quiet_until = (!pause.is_zero()).then(|| Instant::now() + pause);
        held.remote = remote;
    }

    /// All'avvio: il resto della pausa di un cambio fatto prima del riavvio.
    fn quiet_for(&self, left: Duration) {
        self.write().quiet_until = Some(Instant::now() + left);
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

    /// Il nome dichiarato di ciò che [`EmbeddingHolder::in_use`] ha restituito.
    fn identity_of(&self, remote: &Option<Arc<RemoteEmbedder>>) -> String {
        remote.as_ref().map_or_else(|| self.local.embedding_identity().0, |remote| remote.identity().to_owned())
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
        self.embed_as(inputs, None).map(|(vectors, tokens, _)| (vectors, tokens))
    }

    fn embed_as(&self, inputs: &[String], expected: Option<&str>) -> Result<(Vec<Vec<f32>>, u64, String), ComputeError> {
        let remote = self.remote.in_use()?;
        let identity = self.identity_of(&remote);
        expected_model(expected, &identity)?;
        let (vectors, tokens) = match &remote {
            // shortcut: remote calls wait at the local model's gate (one at a time), give the remote provider its own gate when a server needs parallel batches
            Some(remote) => remote.serve(inputs)?,
            None => self.local.embed_counted(inputs)?,
        };
        Ok((vectors, tokens, identity))
    }

    fn embed_image(&self, bytes: &[u8]) -> Result<Vec<f32>, ComputeError> {
        self.embed_image_as(bytes, None).map(|(vector, _)| vector)
    }

    /// Prima la pausa e il modello controllato dal client: un messaggio cominciato prima di un
    /// cambio si ferma, invece di finire senza le sue immagini; poi le immagini rifiutate con un
    /// modello remoto (solo quell'immagine).
    fn check_image(&self, expected: Option<&str>) -> Result<(), ComputeError> {
        let remote = self.remote.in_use()?;
        expected_model(expected, &self.identity_of(&remote))?;
        match remote {
            Some(_) => Err(ComputeError::DocumentUnreadable(IMAGES_UNAVAILABLE.to_owned())),
            None => Ok(()),
        }
    }

    fn embed_image_as(&self, bytes: &[u8], expected: Option<&str>) -> Result<(Vec<f32>, String), ComputeError> {
        let remote = self.remote.in_use()?;
        let identity = self.identity_of(&remote);
        expected_model(expected, &identity)?;
        if remote.is_some() {
            return Err(ComputeError::DocumentUnreadable(IMAGES_UNAVAILABLE.to_owned()));
        }
        Ok((self.local.embed_image(bytes)?, identity))
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
        Ok(RemoteEmbedder::new(&base, model, identity, key, timeout))
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

/// Ciò che si ricorda di un modello remoto salvato ([`RECORDS_FILE`]).
#[derive(Clone, Serialize, Deserialize)]
struct Record {
    /// Quante volte, dietro questo nome, i vettori sono cambiati ([`declared`]).
    generation: u32,
    /// Il vettore della frase di prova all'ultimo salvataggio: il confronto per il prossimo.
    probe: Vec<f32>,
}

/// Il contenuto di [`RECORDS_FILE`].
#[derive(Default, Serialize, Deserialize)]
struct Records {
    #[serde(default)]
    models: BTreeMap<String, Record>,
    /// Fino a quando (secondi Unix) i vettori restano fermi dopo l'ultimo cambio di modello: la
    /// pausa continua dopo un riavvio.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    quiet_until: Option<u64>,
}

fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |since| since.as_secs())
}

impl Records {
    /// Nessun file: vuoto. Un file illeggibile vale come vuoto, con un avviso: i `#n` ripartono da
    /// capo, e le app che vedono un nome senza numero reindicizzano se i vettori non coincidono.
    fn load(path: &Path) -> Records {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|_| {
                tracing::warn!(file = RECORDS_FILE, "file dei modelli di embedding non valido: si riparte da capo");
                Records::default()
            }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Records::default(),
            Err(_) => {
                tracing::warn!(file = RECORDS_FILE, "file dei modelli di embedding illeggibile: si riparte da capo");
                Records::default()
            }
        }
    }

    fn save(&self, path: &Path) -> Result<(), String> {
        let body = serde_json::to_vec_pretty(self).map_err(|_| "Il file dei modelli non si può salvare.".to_owned())?;
        save_private(path, &body).map_err(|_| "Il file dei modelli non si scrive nella cartella dei dati.".to_owned())
    }

    /// Un file di impostazioni di una versione che teneva qui il vettore di prova: il suo nome
    /// passa nel file dei modelli con il suo numero. Vero se è cambiato qualcosa.
    fn adopt(&mut self, settings: &EmbeddingSettings) -> bool {
        let (Some(model), Some(probe)) = (trimmed(settings.model.as_deref()), settings.probe.clone()) else { return false };
        if self.models.contains_key(&model) {
            return false;
        }
        self.models.insert(model, Record { generation: settings.generation, probe });
        true
    }

    /// Il numero del nome `model` con questo vettore di prova: lo stesso di prima se i vettori
    /// coincidono, uno in più se dietro il nome c'è un altro modello, 0 per un nome nuovo.
    fn generation_for(&self, model: &str, probe: &[f32]) -> u32 {
        match self.models.get(model) {
            Some(record) if cosine(&record.probe, probe) < SAME_VECTORS => record.generation.saturating_add(1),
            Some(record) => record.generation,
            None => 0,
        }
    }

    /// Il resto della pausa salvata, mai più lungo di `pause`.
    fn quiet_left(&self, pause: Duration) -> Option<Duration> {
        let left = self.quiet_until?.checked_sub(now_unix()).filter(|left| *left > 0)?;
        Some(Duration::from_secs(left).min(pause))
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
    /// [`RECORDS_FILE`], accanto alle impostazioni.
    records_path: PathBuf,
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
        let records_path = path.with_file_name(RECORDS_FILE);
        let mut records = Records::load(&records_path);
        if let Some(settings) = EmbeddingSettings::load(&path) {
            if records.adopt(&settings) && records.save(&records_path).is_err() {
                tracing::warn!(file = RECORDS_FILE, "file dei modelli di embedding non scritto");
            }
            match settings.embedder(TIMEOUT) {
                Ok(remote) => {
                    tracing::info!(model = remote.identity(), "modello di embedding remoto scelto dal pannello");
                    holder.replace(Some(Arc::new(remote)));
                }
                Err(_) => tracing::error!(file = SETTINGS_FILE, "impostazioni del modello di embedding non valide: vale il modello locale"),
            }
        }
        // Un cambio di modello fatto poco prima di un riavvio: la sua pausa continua.
        if let Some(left) = records.quiet_left(pause) {
            holder.quiet_for(left);
        }
        EmbeddingAdmin { holder, local, path, records_path, pause, lock: Mutex::new(()) }
    }

    /// Un salvataggio alla volta; il secondo non aspetta il primo (la sua prova può durare 20 s).
    fn one_at_a_time(&self) -> Result<MutexGuard<'_, ()>, String> {
        match self.lock.try_lock() {
            Ok(guard) => Ok(guard),
            Err(TryLockError::Poisoned(poisoned)) => Ok(poisoned.into_inner()),
            Err(TryLockError::WouldBlock) => Err(SAVE_IN_PROGRESS.to_owned()),
        }
    }

    /// Il nome che le app vedono con `remote` in uso (`None`: il modello locale).
    fn declared_with(&self, remote: Option<&RemoteEmbedder>) -> String {
        remote.map_or_else(|| self.local.embedding_identity().0, |remote| remote.identity().to_owned())
    }

    /// Vero se mettere in uso `remote` cambia il nome dichiarato: allora i vettori si fermano per la
    /// pausa, e la sua fine si scrive in `records` (che il chiamante salva) per un riavvio.
    fn begins_pause(&self, remote: Option<&RemoteEmbedder>, records: &mut Records) -> bool {
        let changes = self.declared_with(self.holder.get().as_deref()) != self.declared_with(remote);
        if changes && !self.pause.is_zero() {
            records.quiet_until = Some(now_unix().saturating_add(self.pause.as_secs()));
        }
        changes
    }

    /// Mette in uso `remote` (`None`: il modello locale), con la pausa se cambia il nome dichiarato.
    fn put_in_use(&self, remote: Option<Arc<RemoteEmbedder>>, changes: bool) {
        if changes {
            self.holder.switch(remote, self.pause);
        } else {
            self.holder.replace(remote);
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
        let (_, model) = settings.checked()?;
        if !matches!(self.builtin_state().as_str(), "ready" | "loading") {
            return Err(format!("Installa prima il modello locale «{MODEL_ID}» dalla sezione «Modelli»: il suo tokenizer taglia i passaggi anche con un modello remoto."));
        }
        // Un testo fisso nostro, mai la posta di qualcuno. Il client risolve il nome e lo rifiuta
        // se non è interno ([`InternalOnly`]), dentro gli stessi 20 secondi.
        let (vectors, _) = settings.embedder(PROBE_TIMEOUT)?.embed(&[PROBE.to_owned()]).map_err(|error| match error {
            RemoteError::Unreachable => PROBE_UNREACHABLE.to_owned(),
            other => other.to_string(),
        })?;
        let probe = vectors.into_iter().next().ok_or_else(|| RemoteError::BadAnswer.to_string())?;
        // Lo stesso nome di una volta qualunque (anche prima di un ritorno al modello locale o di
        // un altro modello): se i vettori non sono più gli stessi, per le app è un altro modello.
        let mut records = Records::load(&self.records_path);
        if let Some(previous) = &previous {
            records.adopt(previous);
        }
        settings.generation = records.generation_for(&model, &probe);
        records.models.insert(model, Record { generation: settings.generation, probe });
        let remote = settings.embedder(TIMEOUT)?;
        let changes = self.begins_pause(Some(&remote), &mut records);
        // Prima il ricordo del nome, poi le impostazioni: dopo un'interruzione tra i due, il numero
        // è già quello nuovo e il salvataggio seguente non lo perde.
        records.save(&self.records_path)?;
        let body = serde_json::to_vec_pretty(&settings).map_err(|_| "Le impostazioni non si possono salvare.".to_owned())?;
        save_private(&self.path, &body).map_err(|_| "Il file delle impostazioni non si scrive nella cartella dei dati.".to_owned())?;
        tracing::info!(model = remote.identity(), "modello di embedding remoto salvato dal pannello");
        self.put_in_use(Some(Arc::new(remote)), changes);
        Ok(self.view())
    }

    /// Cancella le impostazioni salvate: torna il modello locale.
    pub fn reset(&self) -> Result<serde_json::Value, String> {
        let _one_at_a_time = self.one_at_a_time()?;
        self.back_to_builtin()
    }

    /// Il file dei modelli resta: i `#n` dei nomi servono anche al prossimo modello remoto.
    fn back_to_builtin(&self) -> Result<serde_json::Value, String> {
        let mut records = Records::load(&self.records_path);
        let changes = self.begins_pause(None, &mut records);
        // La pausa che non si scrive dura comunque fino al prossimo riavvio: non blocca il ritorno.
        if changes && !self.pause.is_zero() && records.save(&self.records_path).is_err() {
            tracing::warn!(file = RECORDS_FILE, "fine della pausa non scritta");
        }
        match std::fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("Il file delle impostazioni non si cancella.".to_owned()),
        }
        if changes {
            tracing::info!(model = MODEL_ID, "modello di embedding locale ripristinato dal pannello");
        }
        self.put_in_use(None, changes);
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

    /// Un resolver finto: questi indirizzi per ogni nome, senza rete.
    #[derive(Debug)]
    struct Fixed(Vec<&'static str>);

    impl Resolver for Fixed {
        fn resolve(&self, _: &Uri, _: &ureq::config::Config, _: NextTimeout) -> Result<ResolvedSocketAddrs, ureq::Error> {
            let mut found = self.empty();
            for address in &self.0 {
                found.push(address.parse().unwrap());
            }
            Ok(found)
        }
    }

    fn resolved(addresses: Vec<&'static str>) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let uri: Uri = "http://embeddings:8080/v1/embeddings".parse().unwrap();
        InternalOnly(Fixed(addresses)).resolve(&uri, &ureq::config::Config::default(), NextTimeout::default())
    }

    #[test]
    fn every_connection_goes_only_where_the_name_resolves_internally() {
        assert!(resolved(vec!["10.0.0.5:8080", "127.0.0.1:8080", "[fd00::1]:8080"]).is_ok());
        // Un solo indirizzo pubblico tra quelli del nome basta a rifiutarlo; nessun indirizzo, anche.
        let public = resolved(vec!["10.0.0.5:8080", "93.184.216.34:8080"]).unwrap_err();
        assert_eq!(transport_error(&public, "embeddings"), RemoteError::PublicAddress("embeddings".to_owned()));
        let none = resolved(vec![]).unwrap_err();
        assert_eq!(transport_error(&none, "embeddings"), RemoteError::NotResolved("embeddings".to_owned()));
        // Il pannello dice quale nome e perché.
        assert!(RemoteError::PublicAddress("embeddings".to_owned()).to_string().contains("«embeddings» si risolve in un indirizzo pubblico"));
        // Un altro errore di rete resta «non risponde», senza dettagli.
        assert_eq!(transport_error(&ureq::Error::ConnectionFailed, "embeddings"), RemoteError::Unreachable);
    }

    #[test]
    fn the_pause_after_a_switch_ends_and_a_plain_replace_has_none() {
        let holder = EmbeddingHolder::default();
        holder.replace(None);
        assert!(holder.in_use().is_ok());
        holder.switch(None, Duration::from_secs(660));
        assert!(matches!(holder.in_use(), Err(ComputeError::RateLimited { retry_after_s }) if (1..=600).contains(&retry_after_s)));
        // Una sostituzione senza cambio di nome non tocca la pausa in corso.
        holder.replace(None);
        assert!(holder.in_use().is_err());
        // La pausa finisce davvero, senza che nessuno la tolga.
        holder.switch(None, Duration::from_millis(300));
        assert!(holder.in_use().is_err());
        std::thread::sleep(Duration::from_millis(400));
        assert!(holder.in_use().is_ok());
    }

    #[test]
    fn a_name_keeps_its_number_whatever_was_saved_in_between() {
        let mut a = vec![0.0f32; DIM];
        a[0] = 1.0;
        let mut b = vec![0.0f32; DIM];
        b[1] = 1.0;
        let mut records = Records::default();
        assert_eq!(records.generation_for("x", &a), 0);
        records.models.insert("x".to_owned(), Record { generation: 0, probe: a.clone() });
        assert_eq!(records.generation_for("x", &a), 0);
        assert_eq!(records.generation_for("x", &b), 1);
        records.models.insert("x".to_owned(), Record { generation: 1, probe: b.clone() });
        assert_eq!(records.generation_for("x", &b), 1);
        // Un file di impostazioni di prima, con il vettore dentro: il suo numero non si perde.
        let mut legacy = Records::default();
        let old = EmbeddingSettings { provider: PROVIDER_OPENAI.to_owned(), base_url: None, model: Some("y".to_owned()), api_key: None, generation: 2, probe: Some(a.clone()) };
        assert!(legacy.adopt(&old));
        assert_eq!(legacy.generation_for("y", &a), 2);
        assert!(!legacy.adopt(&old), "una volta sola");
        // La pausa salvata non dura più di quella impostata, e quella passata non c'è più.
        legacy.quiet_until = Some(now_unix() + 10_000);
        assert_eq!(legacy.quiet_left(Duration::from_secs(660)), Some(Duration::from_secs(660)));
        legacy.quiet_until = Some(now_unix().saturating_sub(5));
        assert_eq!(legacy.quiet_left(Duration::from_secs(660)), None);
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
