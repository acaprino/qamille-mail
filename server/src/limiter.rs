//! Limitatori dell'accesso.
//!
//! - [`LoginThrottle`] rallenta i tentativi sbagliati **per identificativo**, senza
//!   mai bloccare: l'attesa ha un tetto di 4 s dall'arrivo della richiesta.
//! - [`FailureLimiter`] conta i fallimenti **per origine** e, oltre la soglia,
//!   risponde `rate_limited` prima di qualsiasi verifica.
//!
//! Gli istanti sono passati dal chiamante, così i test non dipendono dall'orologio.

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, Ipv6Addr};
use std::sync::Mutex;
use std::time::Duration;

use tokio::time::Instant;

use crate::error::ServerError;

/// Tentativi sbagliati senza attesa dentro la finestra.
pub const FREE_ATTEMPTS: usize = 3;
/// Finestra dei tentativi sbagliati per identificativo.
pub const THROTTLE_WINDOW: Duration = Duration::from_secs(15 * 60);
/// Prima attesa dopo i tentativi liberi; poi raddoppia.
pub const FIRST_WAIT: Duration = Duration::from_millis(250);
/// Tetto dell'attesa, contato dall'arrivo della richiesta.
pub const MAX_WAIT: Duration = Duration::from_secs(4);
/// Verifiche in corso o in attesa per lo stesso identificativo, oltre le quali un tentativo in
/// più riceve `rate_limited` senza argon2: un account solo non occupa più di due dei quattro
/// posti di argon2, da quante origini arrivino i tentativi.
pub const MAX_PENDING_PER_KEY: usize = 2;
/// Voci massime del rallentamento per identificativo.
pub const THROTTLE_CAPACITY: usize = 10_000;
/// Origini massime del limite per origine.
pub const ORIGIN_CAPACITY: usize = 4096;
/// Prefisso delle chiavi del rallentamento che appartengono a un utente esistente:
/// a tabella piena non si tolgono mai per far posto ad altre.
pub const USER_KEY_PREFIX: &str = "u:";

/// Normalizza un identificativo: spazi ai lati e maiuscole non contano.
pub fn normalize_identifier(identifier: &str) -> String {
    identifier.trim().to_lowercase()
}

/// Attesa prima del prossimo tentativo, dati i fallimenti nella finestra.
pub fn wait_for(failures: usize) -> Duration {
    if failures < FREE_ATTEMPTS {
        return Duration::ZERO;
    }
    let doublings = (failures - FREE_ATTEMPTS).min(8) as u32;
    (FIRST_WAIT * 2u32.pow(doublings)).min(MAX_WAIT)
}

struct ThrottleEntry {
    failures: VecDeque<Instant>,
    /// Tentativi prenotati e non ancora conclusi: contano nell'attesa dei successivi,
    /// così tentativi mandati insieme si mettono in fila.
    pending: usize,
    next_free: Instant,
    last_touch: Instant,
}

impl ThrottleEntry {
    fn new(now: Instant) -> Self {
        ThrottleEntry {
            failures: VecDeque::new(),
            pending: 0,
            next_free: now,
            last_touch: now,
        }
    }

    fn prune(&mut self, now: Instant) {
        while let Some(&first) = self.failures.front() {
            if now.saturating_duration_since(first) >= THROTTLE_WINDOW {
                self.failures.pop_front();
            } else {
                break;
            }
        }
    }

    /// Una voce con tentativi nella finestra, in corso o con un turno prenotato
    /// non si toglie senza motivo.
    fn active(&self, now: Instant) -> bool {
        self.pending > 0
            || self.next_free > now
            || self
                .failures
                .iter()
                .any(|&t| now.saturating_duration_since(t) < THROTTLE_WINDOW)
    }

    fn push_failure(&mut self, now: Instant) {
        self.prune(now);
        self.failures.push_back(now);
        // Oltre FREE_ATTEMPTS + 8 l'attesa è già al tetto.
        while self.failures.len() > FREE_ATTEMPTS + 8 {
            self.failures.pop_front();
        }
    }
}

/// Dove sta il conteggio di una chiave: la sua voce o la voce comune di riserva.
#[derive(Clone, PartialEq, Eq)]
enum Slot {
    Key(String),
    Overflow,
}

struct ThrottleTable {
    entries: HashMap<String, ThrottleEntry>,
    /// Voce comune per le chiavi che non trovano posto: conta tentativi in corso e
    /// sbagliati come le altre, così una tabella piena non rende più economici i
    /// tentativi in parallelo (al più li mette tutti nella stessa fila, sempre con il
    /// tetto di 4 s).
    overflow: ThrottleEntry,
}

impl ThrottleTable {
    fn get_mut(&mut self, slot: &Slot) -> Option<&mut ThrottleEntry> {
        match slot {
            Slot::Key(key) => self.entries.get_mut(key),
            Slot::Overflow => Some(&mut self.overflow),
        }
    }
}

/// Rallentamento per identificativo, mai blocco.
///
/// La chiave la sceglie il chiamante: [`crate::accounts::Accounts`] usa l'id
/// dell'utente (`u:<id>`) quando l'identificativo corrisponde a un utente (username
/// ed email finiscono nella stessa voce) e l'identificativo normalizzato altrimenti.
///
/// A tabella piena (10 000 voci): prima si tolgono le voci non più attive; poi, se
/// serve, la voce di un identificativo **sconosciuto** (non `u:`) senza tentativi in
/// corso, quella con meno tentativi sbagliati e toccata da più tempo; le voci degli
/// utenti esistenti non si tolgono mai. Se non c'è nulla da togliere, la chiave usa
/// la voce comune di riserva, che conta come le altre.
pub struct LoginThrottle {
    capacity: usize,
    table: Mutex<ThrottleTable>,
}

impl Default for LoginThrottle {
    fn default() -> Self {
        Self::new()
    }
}

/// Un tentativo prenotato. Va concluso con [`ThrottleTicket::failed`] o
/// [`ThrottleTicket::succeeded`]; se viene lasciato cadere (richiesta interrotta,
/// errore del server) esce dai tentativi in corso senza contare come sbagliato.
pub struct ThrottleTicket<'a> {
    throttle: &'a LoginThrottle,
    slot: Option<Slot>,
    start: Instant,
}

impl ThrottleTicket<'_> {
    /// Istante da cui può partire la verifica.
    pub fn start(&self) -> Instant {
        self.start
    }

    /// Il tentativo usa la voce comune di riserva (tabella piena).
    pub fn in_overflow(&self) -> bool {
        self.slot == Some(Slot::Overflow)
    }

    /// Tentativo sbagliato: esce dai tentativi in corso e conta nella finestra.
    pub fn failed(mut self, now: Instant) {
        if let Some(slot) = self.slot.take() {
            self.throttle.finish(&slot, Some(now), false);
        }
    }

    /// Password giusta: esce dai tentativi in corso e azzera i tentativi sbagliati
    /// (non quelli della voce comune, che è di tutti).
    pub fn succeeded(mut self) {
        if let Some(slot) = self.slot.take() {
            self.throttle.finish(&slot, None, true);
        }
    }
}

impl Drop for ThrottleTicket<'_> {
    fn drop(&mut self) {
        if let Some(slot) = self.slot.take() {
            self.throttle.finish(&slot, None, false);
        }
    }
}

impl LoginThrottle {
    pub fn new() -> Self {
        Self::with_capacity(THROTTLE_CAPACITY)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        let now = Instant::now();
        LoginThrottle {
            capacity,
            table: Mutex::new(ThrottleTable {
                entries: HashMap::new(),
                overflow: ThrottleEntry::new(now),
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ThrottleTable> {
        self.table.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// La voce di `key`, creata se c'è (o si fa) posto; altrimenti la voce comune.
    fn locate(&self, table: &mut ThrottleTable, key: &str, now: Instant) -> Slot {
        if table.entries.contains_key(key) {
            return Slot::Key(key.to_owned());
        }
        if table.entries.len() >= self.capacity {
            table.entries.retain(|_, e| e.active(now));
        }
        if table.entries.len() >= self.capacity {
            let victim = table
                .entries
                .iter()
                .filter(|(k, e)| !k.starts_with(USER_KEY_PREFIX) && e.pending == 0)
                .min_by_key(|(_, e)| (e.failures.len(), e.last_touch))
                .map(|(k, _)| k.clone());
            match victim {
                Some(victim) => {
                    table.entries.remove(&victim);
                }
                None => return Slot::Overflow,
            }
        }
        table
            .entries
            .insert(key.to_owned(), ThrottleEntry::new(now));
        Slot::Key(key.to_owned())
    }

    /// Prenota il turno del prossimo tentativo: la verifica può partire da
    /// `min(max(next_free, now) + attesa, now + 4 s)`, dove l'attesa conta i
    /// tentativi sbagliati nella finestra **e** quelli ancora in corso.
    pub fn begin(&self, key: &str, now: Instant) -> ThrottleTicket<'_> {
        self.begin_with_cap(key, now, usize::MAX)
            .expect("senza tetto la prenotazione riesce sempre")
    }

    /// Come [`LoginThrottle::begin`], ma con al più [`MAX_PENDING_PER_KEY`] tentativi in corso
    /// per la stessa chiave: oltre, `rate_limited` subito (non si passa da argon2). La voce
    /// comune di riserva non ha tetto: è di tutti.
    pub fn try_begin(&self, key: &str, now: Instant) -> Result<ThrottleTicket<'_>, ServerError> {
        self.begin_with_cap(key, now, MAX_PENDING_PER_KEY)
            .ok_or(ServerError::RateLimited {
                retry_after_s: MAX_WAIT.as_secs(),
            })
    }

    fn begin_with_cap(&self, key: &str, now: Instant, cap: usize) -> Option<ThrottleTicket<'_>> {
        let key = normalize_identifier(key);
        let mut table = self.lock();
        let slot = self.locate(&mut table, &key, now);
        let entry = table.get_mut(&slot).expect("voce appena trovata");
        if matches!(slot, Slot::Key(_)) && entry.pending >= cap {
            return None;
        }
        entry.prune(now);
        entry.last_touch = now;
        let wait = wait_for(entry.failures.len() + entry.pending);
        let start = (entry.next_free.max(now) + wait).min(now + MAX_WAIT);
        entry.next_free = start;
        entry.pending += 1;
        Some(ThrottleTicket {
            throttle: self,
            slot: Some(slot),
            start,
        })
    }

    fn finish(&self, slot: &Slot, failed_at: Option<Instant>, succeeded: bool) {
        let mut table = self.lock();
        if let Some(entry) = table.get_mut(slot) {
            entry.pending = entry.pending.saturating_sub(1);
            if let Some(at) = failed_at {
                entry.push_failure(at);
            }
            if succeeded && *slot != Slot::Overflow {
                entry.failures.clear();
            }
        }
    }

    /// Registra un tentativo sbagliato fuori da una prenotazione.
    pub fn failed(&self, key: &str, now: Instant) {
        let key = normalize_identifier(key);
        let mut table = self.lock();
        let slot = self.locate(&mut table, &key, now);
        if let Some(entry) = table.get_mut(&slot) {
            entry.last_touch = now;
            entry.push_failure(now);
        }
    }

    /// Azzera i tentativi sbagliati (password giusta, password reimpostata).
    pub fn succeeded(&self, key: &str) {
        let key = normalize_identifier(key);
        if let Some(entry) = self.lock().entries.get_mut(&key) {
            entry.failures.clear();
        }
    }

    /// Tentativi sbagliati nella finestra (per i test e la diagnostica).
    pub fn failures(&self, key: &str, now: Instant) -> usize {
        let key = normalize_identifier(key);
        self.lock()
            .entries
            .get_mut(&key)
            .map(|e| {
                e.prune(now);
                e.failures.len()
            })
            .unwrap_or(0)
    }

    /// Tentativi in corso (per i test e la diagnostica).
    pub fn pending(&self, key: &str) -> usize {
        let key = normalize_identifier(key);
        self.lock()
            .entries
            .get(&key)
            .map(|e| e.pending)
            .unwrap_or(0)
    }

    /// Tentativi sbagliati e in corso della voce comune di riserva.
    pub fn overflow_counts(&self, now: Instant) -> (usize, usize) {
        let mut table = self.lock();
        table.overflow.prune(now);
        (table.overflow.failures.len(), table.overflow.pending)
    }

    pub fn len(&self) -> usize {
        self.lock().entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Voce del limite per origine: un IPv4 è un'origine; un IPv6 conta per la sua /64,
/// che di solito appartiene a un solo cliente (cambiare indirizzo dentro la /64 non
/// azzera il conteggio).
pub fn origin_key(ip: IpAddr) -> IpAddr {
    match ip.to_canonical() {
        IpAddr::V4(v4) => IpAddr::V4(v4),
        IpAddr::V6(v6) => {
            let bits = u128::from(v6) & !((1u128 << 64) - 1);
            IpAddr::V6(Ipv6Addr::from(bits))
        }
    }
}

struct OriginEntry {
    failures: VecDeque<Instant>,
    /// Tentativi iniziati e non ancora conclusi: contano come fallimenti finché
    /// non finiscono, così una raffica in parallelo non supera la soglia.
    pending: usize,
    last_touch: Instant,
}

impl OriginEntry {
    fn new(now: Instant) -> Self {
        OriginEntry {
            failures: VecDeque::new(),
            pending: 0,
            last_touch: now,
        }
    }
}

/// Soglia di fallimenti per origine dentro una finestra.
///
/// **Tabella piena** (4096 origini). Un'origine nuova non viene mai rifiutata né
/// lasciata passare senza conteggio: per farle posto si tolgono prima le voci senza
/// fallimenti nella finestra e senza tentativi in corso, poi, se non basta, la voce
/// con **meno** fallimenti più tentativi in corso, a parità quella toccata da più
/// tempo. Così:
/// - chi riempie la tabella con molte origini da un fallimento ciascuna non chiude
///   fuori le altre origini (al più toglie voci che pesano quanto le sue);
/// - un'origine vicina alla soglia o già oltre resta contata finché qualcun altro
///   non ha almeno altrettanti fallimenti in ognuna delle 4096 voci, cioè finché
///   l'attaccante non spende più tentativi di quanti ne recupererebbe.
pub struct FailureLimiter {
    max_failures: usize,
    window: Duration,
    capacity: usize,
    entries: Mutex<HashMap<IpAddr, OriginEntry>>,
}

/// Un tentativo iniziato con [`FailureLimiter::begin`]. Lasciato cadere senza
/// esito esce dai tentativi in corso senza contare.
pub struct OriginAttempt<'a> {
    limiter: &'a FailureLimiter,
    key: IpAddr,
    open: bool,
}

impl OriginAttempt<'_> {
    pub fn failed(mut self, now: Instant) {
        self.open = false;
        self.limiter.finish(self.key, Some(now));
    }

    pub fn succeeded(mut self) {
        self.open = false;
        self.limiter.finish(self.key, None);
    }
}

impl Drop for OriginAttempt<'_> {
    fn drop(&mut self) {
        if self.open {
            self.limiter.finish(self.key, None);
        }
    }
}

impl FailureLimiter {
    pub fn new(max_failures: u32, window: Duration) -> Self {
        Self::with_capacity(max_failures, window, ORIGIN_CAPACITY)
    }

    pub fn with_capacity(max_failures: u32, window: Duration, capacity: usize) -> Self {
        FailureLimiter {
            max_failures: max_failures.max(1) as usize,
            window,
            capacity: capacity.max(1),
            entries: Mutex::new(HashMap::new()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<IpAddr, OriginEntry>> {
        self.entries.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn prune(window: Duration, failures: &mut VecDeque<Instant>, now: Instant) {
        while let Some(&first) = failures.front() {
            if now.saturating_duration_since(first) >= window {
                failures.pop_front();
            } else {
                break;
            }
        }
    }

    fn retry_after(remaining: Duration) -> u64 {
        let secs = remaining.as_secs() + u64::from(remaining.subsec_nanos() > 0);
        secs.max(1)
    }

    /// Soglia raggiunta (fallimenti più tentativi in corso) per un'origine già contata.
    fn over_threshold(
        &self,
        entries: &mut HashMap<IpAddr, OriginEntry>,
        key: IpAddr,
        now: Instant,
    ) -> Result<(), ServerError> {
        let window = self.window;
        let Some(entry) = entries.get_mut(&key) else {
            return Ok(());
        };
        Self::prune(window, &mut entry.failures, now);
        if entry.failures.len() + entry.pending < self.max_failures {
            return Ok(());
        }
        let retry_after_s = if entry.failures.len() >= self.max_failures {
            let oldest = *entry.failures.front().expect("almeno max_failures voci");
            Self::retry_after((oldest + window).saturating_duration_since(now))
        } else {
            // Al limite per tentativi ancora in corso: finiscono in pochi secondi.
            1
        };
        Err(ServerError::RateLimited { retry_after_s })
    }

    /// La voce di `key`, creata se manca: a tabella piena fa posto secondo la regola
    /// descritta su [`FailureLimiter`]. Non rifiuta mai.
    fn entry<'m>(
        &self,
        entries: &'m mut HashMap<IpAddr, OriginEntry>,
        key: IpAddr,
        now: Instant,
    ) -> &'m mut OriginEntry {
        if !entries.contains_key(&key) && entries.len() >= self.capacity {
            let window = self.window;
            entries.retain(|_, e| {
                Self::prune(window, &mut e.failures, now);
                e.pending > 0 || !e.failures.is_empty()
            });
            if entries.len() >= self.capacity {
                let victim = entries
                    .iter()
                    .min_by_key(|(_, e)| (e.failures.len() + e.pending, e.last_touch))
                    .map(|(k, _)| *k);
                if let Some(victim) = victim {
                    entries.remove(&victim);
                }
            }
        }
        let entry = entries.entry(key).or_insert_with(|| OriginEntry::new(now));
        entry.last_touch = now;
        entry
    }

    /// `RateLimited` se l'origine ha raggiunto la soglia nella finestra (contando i
    /// tentativi in corso).
    pub fn check(&self, origin: IpAddr, now: Instant) -> Result<(), ServerError> {
        let mut entries = self.lock();
        self.over_threshold(&mut entries, origin_key(origin), now)
    }

    /// Inizia un tentativo: controllo e prenotazione insieme, sotto lo stesso blocco.
    pub fn begin(&self, origin: IpAddr, now: Instant) -> Result<OriginAttempt<'_>, ServerError> {
        let key = origin_key(origin);
        let mut entries = self.lock();
        self.over_threshold(&mut entries, key, now)?;
        self.entry(&mut entries, key, now).pending += 1;
        Ok(OriginAttempt {
            limiter: self,
            key,
            open: true,
        })
    }

    /// Conta una richiesta come un fallimento, se l'origine è sotto la soglia: per
    /// i limiti che valgono su ogni richiesta, anche riuscita.
    pub fn hit(&self, origin: IpAddr, now: Instant) -> Result<(), ServerError> {
        let key = origin_key(origin);
        let mut entries = self.lock();
        self.over_threshold(&mut entries, key, now)?;
        self.entry(&mut entries, key, now).failures.push_back(now);
        Ok(())
    }

    fn finish(&self, key: IpAddr, failed_at: Option<Instant>) {
        let mut entries = self.lock();
        let window = self.window;
        let max = self.max_failures;
        if let Some(entry) = entries.get_mut(&key) {
            entry.pending = entry.pending.saturating_sub(1);
            if let Some(at) = failed_at {
                Self::prune(window, &mut entry.failures, at);
                entry.failures.push_back(at);
                entry.last_touch = at;
                while entry.failures.len() > max {
                    entry.failures.pop_front();
                }
            }
        } else if let Some(at) = failed_at {
            // La voce è stata tolta per far posto mentre il tentativo era in corso:
            // il fallimento si conta comunque.
            self.entry(&mut entries, key, at).failures.push_back(at);
        }
    }

    /// Registra un fallimento fuori da un tentativo iniziato.
    pub fn record_failure(&self, origin: IpAddr, now: Instant) {
        let key = origin_key(origin);
        let mut entries = self.lock();
        let window = self.window;
        let max = self.max_failures;
        let entry = self.entry(&mut entries, key, now);
        Self::prune(window, &mut entry.failures, now);
        entry.failures.push_back(now);
        while entry.failures.len() > max {
            entry.failures.pop_front();
        }
    }

    /// Fallimenti nella finestra (per i test e la diagnostica).
    pub fn failures(&self, origin: IpAddr, now: Instant) -> usize {
        let mut entries = self.lock();
        entries
            .get_mut(&origin_key(origin))
            .map(|e| {
                Self::prune(self.window, &mut e.failures, now);
                e.failures.len()
            })
            .unwrap_or(0)
    }

    /// Tentativi in corso (per i test e la diagnostica).
    pub fn pending(&self, origin: IpAddr) -> usize {
        self.lock()
            .get(&origin_key(origin))
            .map(|e| e.pending)
            .unwrap_or(0)
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
