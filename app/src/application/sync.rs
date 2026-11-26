//! The mailboxes that follow a person between computers (sync spec S2..S6, INV-35). `SyncServer`
//! is the port to the server's `/v1/sync/items` routes; `SyncService` turns the mailboxes into
//! items, applies the ones that arrive and sends the local changes. The credentials of the mailboxes
//! stay in the vault (`SecretStore`): they leave this computer only inside an item, only to the
//! address of the session, only after the consent, and never reach `Settings`, events, logs,
//! `Debug` or the state file (which keeps HMACs, not values).
use super::{
    AppError, AppResult, ServiceContext,
    events::AppEvent,
    ports::{GOOGLE_SECRET, account_key, provider_account_key},
    server_account::{RemoteEndpoint, ServerAccountService},
};
use crate::domain::{
    model::{Account, AccountKind, Settings, Signature, SignatureBinding, Store},
    sync::{self, SyncAccount, SyncCredential, SyncItem, item_id, merge_changed},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The vault value holding the random key of the HMACs of the state file.
pub const MAC_KEY: &str = "mailbox-sync-key";
/// How often a pass runs on its own, and how long after a local change.
pub const PERIOD: Duration = Duration::from_secs(300);
pub const AFTER_CHANGE: Duration = Duration::from_secs(2);
/// A full listing (and the check for items that vanished) at least this often, in seconds.
pub const FULL_LISTING_SECS: i64 = 3600;
const MAX_ATTEMPTS: usize = 3;

/// One item as the server returns it. `item` is `None` for a tombstone, and for an item this app
/// cannot read (a future schema): it is then ignored, never overwritten blindly.
#[derive(Clone, Debug, PartialEq)]
pub struct RemoteItem {
    pub id: String,
    pub version: u64,
    pub deleted: bool,
    pub item: Option<SyncItem>,
}
/// `GET /v1/sync/items`.
#[derive(Clone, Debug, PartialEq)]
pub struct Listing {
    pub version: u64,
    pub epoch: u64,
    pub items: Vec<RemoteItem>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum PutOutcome {
    Written { version: u64 },
    /// `409 sync_conflict`: the current item, or `None` when the declared version does not exist.
    Conflict(Option<RemoteItem>),
}

/// The synchronization routes of a Qamille server. Errors carry fixed codes: `unreachable`,
/// `unauthorized`, `subscription_required`, `not_found` (a server without the key), `sync_limit`,
/// `payload_too_large`, `rate_limited`, `tls_required`, `invalid_request`, `server_error`.
pub trait SyncServer: Send + Sync {
    fn list(&self, endpoint: &RemoteEndpoint, since: u64) -> AppResult<Listing>;
    fn put(&self, endpoint: &RemoteEndpoint, id: &str, base_version: u64, item: &SyncItem) -> AppResult<PutOutcome>;
    /// A tombstone; an item the server does not know counts as deleted.
    fn delete(&self, endpoint: &RemoteEndpoint, id: &str, base_version: Option<u64>) -> AppResult<()>;
}

/// What this computer remembers of the synchronization (no secret in it).
pub trait SyncStateStore: Send + Sync {
    fn load(&self) -> AppResult<SyncState>;
    fn save(&self, state: &SyncState) -> AppResult<()>;
}

/// Which Google clients belong to this build: their secret never travels in an item.
pub trait GoogleClients: Send + Sync {
    fn is_app_client(&self, client_id: &str) -> bool;
}

/// What the server holds for one mailbox, as far as this computer knows: the content is stored
/// as HMACs of its two parts, so a changed local mailbox is told from an unchanged one.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tracked {
    pub version: u64,
    pub account_mac: String,
    pub credential_mac: String,
    pub updated_at: i64,
    pub credential_updated_at: i64,
}
/// A mailbox removed (or switched off: the server cannot tell them apart) on another computer,
/// waiting for the person.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemovalNotice {
    pub item_id: String,
    pub name: String,
    pub email: String,
}
/// A mailbox this computer does not send as it is: it cannot be shared (it does not pass the
/// check of an item), or the server refused it (`400`, `413`, the limit). It is tried again when
/// its content changes (the two HMACs differ) and at the next full listing; the reason is shown
/// next to the mailbox and the rest of the synchronization goes on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rejected {
    pub account_mac: String,
    pub credential_mac: String,
    pub reason: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingDelete {
    pub item_id: String,
    pub version: u64,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncState {
    /// The session this state belongs to: another server or user starts from nothing.
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub epoch: Option<u64>,
    #[serde(default)]
    pub since: u64,
    #[serde(default)]
    pub last_full: i64,
    #[serde(default)]
    pub last_sync: Option<i64>,
    #[serde(default)]
    pub items: BTreeMap<String, Tracked>,
    /// Mailboxes the person keeps only on this computer («Sincronizza questa casella» off).
    #[serde(default)]
    pub disabled: BTreeSet<String>,
    #[serde(default)]
    pub removals: Vec<RemovalNotice>,
    #[serde(default)]
    pub pending_deletes: Vec<PendingDelete>,
    #[serde(default)]
    pub rejected: BTreeMap<String, Rejected>,
    /// Mailboxes whose token this computer revoked: they wait for a new sign-in, and the old
    /// token that is still on the server must not come back into the vault meanwhile.
    #[serde(default)]
    pub needs_reconnect: BTreeSet<String>,
}

impl SyncState {
    /// The state for another session (or none): what was learned about the server goes, what the
    /// person decided about their mailboxes on this computer stays.
    pub fn restarted(&self, url: String, user_id: String) -> SyncState {
        SyncState { url, user_id, disabled: self.disabled.clone(), needs_reconnect: self.needs_reconnect.clone(), ..Default::default() }
    }
    /// Puts into `self` (the state as saved now) what a pass changed: `start` is the state the
    /// pass began with and `end` the one it ended with. A change made by someone else while the
    /// pass was talking to the server (a switch, a revocation, a removal) is therefore kept.
    fn absorb(&mut self, start: &SyncState, end: &SyncState) {
        self.epoch = end.epoch;
        self.since = end.since;
        self.last_full = end.last_full;
        self.last_sync = end.last_sync;
        self.rejected = end.rejected.clone();
        for (id, tracked) in &end.items {
            // A mailbox removed here meanwhile is not tracked again.
            if start.items.get(id) != Some(tracked) && !self.pending_deletes.iter().any(|d| &d.item_id == id) {
                self.items.insert(id.clone(), tracked.clone());
            }
        }
        for id in start.items.keys().filter(|id| !end.items.contains_key(*id)) {
            self.items.remove(id);
        }
        for notice in end.removals.iter().filter(|r| !start.removals.contains(r)) {
            if !self.removals.iter().any(|r| r.item_id == notice.item_id) {
                self.removals.push(notice.clone());
            }
        }
        self.removals.retain(|r| end.removals.contains(r) || !start.removals.contains(r));
        for delete in end.pending_deletes.iter().filter(|d| !start.pending_deletes.contains(d)) {
            if !self.pending_deletes.iter().any(|d| d.item_id == delete.item_id) {
                self.pending_deletes.push(delete.clone());
            }
        }
        self.pending_deletes.retain(|d| end.pending_deletes.contains(d) || !start.pending_deletes.contains(d));
        for id in start.needs_reconnect.iter().filter(|id| !end.needs_reconnect.contains(*id)) {
            self.needs_reconnect.remove(id);
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SyncReport {
    pub downloaded: usize,
    pub uploaded: usize,
    pub removal_notices: usize,
    /// The server asks for a subscription (`402`): the local mailboxes go on.
    pub paused: bool,
    /// The server has no synchronization (`404`).
    pub unavailable: bool,
    /// A pass was already running, or the conditions are not met: nothing was done.
    pub skipped: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncMode {
    /// The general switch is off, or the consent was never given.
    Off,
    /// No server connected.
    NoServer,
    /// The server does not declare the synchronization.
    Unavailable,
    Idle,
    PausedForSubscription,
    Error,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MailboxSwitch {
    pub account_id: u32,
    pub enabled: bool,
    /// The token was revoked here: the mailbox must be signed in again.
    pub needs_reconnect: bool,
    /// Why this mailbox is not synchronized although it is switched on, if it is not.
    pub problem: Option<String>,
}

/// What asking Google to revoke the access of a mailbox came to (S5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RevokeOutcome {
    /// Google revoked the token: it no longer works on any computer.
    Revoked,
    /// Google did not answer or refused: nothing changed, the person may retry.
    Failed,
    /// An app password: only its provider can revoke it.
    ManualPassword,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MailboxRevoke {
    pub account_id: u32,
    pub email: String,
    pub provider_id: String,
    pub outcome: RevokeOutcome,
}

/// Asks Google to revoke a refresh token. The token is never in an error.
pub trait TokenRevoker: Send + Sync {
    fn revoke(&self, token: &str) -> AppResult<()>;
}
struct NoRevoker;
impl TokenRevoker for NoRevoker {
    fn revoke(&self, _: &str) -> AppResult<()> {
        Err(AppError::new("unavailable", "Google did not revoke the access"))
    }
}
/// What the interface may know: no credential, no key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SyncStatus {
    pub mode: SyncMode,
    pub enabled: bool,
    pub consented: bool,
    pub last_sync: Option<i64>,
    pub error: Option<String>,
    pub removals: Vec<RemovalNotice>,
    pub mailboxes: Vec<MailboxSwitch>,
}

#[derive(Default)]
struct Outcome {
    paused: bool,
    unavailable: bool,
    error: Option<String>,
}
#[derive(Default)]
struct Schedule {
    started: bool,
    next: Option<Instant>,
    nudged: Option<Instant>,
}

pub struct SyncService {
    context: Arc<ServiceContext>,
    accounts: Arc<ServerAccountService>,
    server: Arc<dyn SyncServer>,
    state: Arc<dyn SyncStateStore>,
    clients: Arc<dyn GoogleClients>,
    revoker: Arc<dyn TokenRevoker>,
    run: Mutex<()>,
    /// Held for each load-change-save of the state file, never across a call to the server.
    state_lock: Mutex<()>,
    outcome: Mutex<Outcome>,
    schedule: Mutex<Schedule>,
    /// The server refused this device (`unauthorized`) and `SignInExpired` said so: not again until a
    /// pass works.
    refused: std::sync::atomic::AtomicBool,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The vault key of the credential of a mailbox.
pub fn credential_key(account: &Account) -> String {
    if account.resolved_provider_id() == "gmail" {
        account_key(&account.email)
    } else {
        provider_account_key(account.resolved_provider_id(), account.id)
    }
}

/// A local mailbox as an item, with the parts' HMACs.
struct Local {
    /// The signature of this mailbox is also the one of another: it goes out with the item, but
    /// it is not part of what changed here, and an item from the server never rewrites it.
    shared_signature: bool,
    /// Why the item cannot be sent (it does not pass the check an item must pass).
    invalid: Option<&'static str>,
    account_id: u32,
    id: String,
    item: SyncItem,
    account_mac: String,
    credential_mac: String,
}

impl SyncService {
    pub fn new(
        context: Arc<ServiceContext>,
        accounts: Arc<ServerAccountService>,
        server: Arc<dyn SyncServer>,
        state: Arc<dyn SyncStateStore>,
        clients: Arc<dyn GoogleClients>,
    ) -> Self {
        Self {
            context,
            accounts,
            server,
            state,
            clients,
            revoker: Arc::new(NoRevoker),
            run: Mutex::new(()),
            state_lock: Mutex::new(()),
            outcome: Mutex::new(Outcome::default()),
            schedule: Mutex::new(Schedule::default()),
            refused: std::sync::atomic::AtomicBool::new(false),
        }
    }

    pub fn with_revoker(mut self, revoker: Arc<dyn TokenRevoker>) -> Self {
        self.revoker = revoker;
        self
    }

    /// One load-change-save of the state, alone: two changes made at the same time both stay. A
    /// failed save is an error, and a change that changes nothing writes nothing.
    fn with_state<R>(&self, change: impl FnOnce(&mut SyncState) -> AppResult<R>) -> AppResult<R> {
        let _held = lock(&self.state_lock);
        let mut state = self.state.load()?;
        let before = state.clone();
        let result = change(&mut state)?;
        if state != before {
            self.state.save(&state)?;
        }
        Ok(result)
    }
    fn read_state(&self) -> AppResult<SyncState> {
        let _held = lock(&self.state_lock);
        self.state.load()
    }

    // ---- consent and switches -------------------------------------------------------------

    /// The person agreed to keep the credentials of the mailboxes on this server (S6). It counts
    /// only if the server itself declares the synchronization now.
    pub fn record_consent(&self, url: &str) -> AppResult<()> {
        let Some(address) = self.accounts.sync_declared(url)? else { return Ok(()) };
        self.context.update(|store| {
            store.settings.mailbox_sync_consent = Some(address.clone());
            store.settings.mailbox_sync = true;
            Ok(())
        })
    }
    /// The general switch. Turning it on needs the consent for the address of the session.
    pub fn set_enabled(&self, enabled: bool) -> AppResult<()> {
        if enabled {
            let session = self.accounts.session()?.ok_or_else(|| AppError::new("not_connected", "This computer is not connected to a Qamille server"))?;
            if self.context.read_shared()?.settings.mailbox_sync_consent.as_deref() != Some(session.url.as_str()) {
                return Err(AppError::new("consent_required", "Agree to keep the mailboxes on this server first"));
            }
        }
        self.context.update(|store| {
            store.settings.mailbox_sync = enabled;
            Ok(())
        })?;
        if enabled {
            self.nudge();
        }
        Ok(())
    }
    /// «Sincronizza questa casella».
    pub fn set_mailbox(&self, account_id: u32, enabled: bool) -> AppResult<()> {
        let store = self.context.read_shared()?;
        let account = store.account(account_id).ok_or_else(|| AppError::missing("Account"))?;
        let id = item_id(account.resolved_provider_id(), &account.email);
        self.with_state(|state| {
            if enabled {
                state.disabled.remove(&id);
                // Switched on again before the server was told: nothing is deleted.
                state.pending_deletes.retain(|d| d.item_id != id);
            } else {
                state.disabled.insert(id);
            }
            Ok(())
        })?;
        if enabled {
            self.nudge();
        }
        Ok(())
    }

    // ---- scheduling -----------------------------------------------------------------------

    pub fn nudge(&self) {
        self.nudge_at(Instant::now());
    }
    pub fn nudge_at(&self, at: Instant) {
        lock(&self.schedule).nudged.get_or_insert(at);
    }
    /// Called about once a second by the desktop: runs a pass at startup, every five minutes and
    /// two seconds after a local change.
    pub fn tick(&self, now: Instant) -> AppResult<Option<SyncReport>> {
        {
            let mut schedule = lock(&self.schedule);
            let due = !schedule.started
                || schedule.next.is_some_and(|next| now >= next)
                || schedule.nudged.is_some_and(|at| now >= at + AFTER_CHANGE);
            if !due {
                return Ok(None);
            }
            schedule.started = true;
            schedule.nudged = None;
            schedule.next = Some(now + PERIOD);
        }
        self.sync_now().map(Some)
    }

    // ---- state of the pass ----------------------------------------------------------------

    fn mac(&self, data: &[u8]) -> AppResult<String> {
        let mut key = self.context.secrets.get(MAC_KEY)?;
        if key.trim().is_empty() {
            let mut bytes = [0u8; 32];
            getrandom::fill(&mut bytes).map_err(|_| AppError::storage("No random numbers for the synchronization state"))?;
            key = bytes.iter().map(|b| format!("{b:02x}")).collect();
            self.context.secrets.set(MAC_KEY, &key)?;
        }
        let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key.as_bytes());
        Ok(ring::hmac::sign(&key, data).as_ref().iter().map(|b| format!("{b:02x}")).collect())
    }

    /// The item of a local mailbox, without its times; `None` when the mailbox cannot be shared
    /// (a sample, or a credential that is gone and must be reconnected).
    fn local(&self, store: &Store, settings: &Settings, account: &Account) -> AppResult<Option<Local>> {
        if account.kind == AccountKind::Sample || account.resolved_provider_id() == "sample" {
            return Ok(None);
        }
        let value = self.context.secrets.get(&credential_key(account))?;
        if value.trim().is_empty() {
            return Ok(None);
        }
        let google = !account.oauth_client_id.is_empty();
        let client_secret = (google && !self.clients.is_app_client(&account.oauth_client_id) && settings.google_client_id == account.oauth_client_id)
            .then(|| self.context.secrets.get(GOOGLE_SECRET))
            .transpose()?
            .filter(|secret| !secret.trim().is_empty());
        let signature = match account.signature {
            SignatureBinding::Id(id) => store.signatures.iter().find(|s| s.id == id).map(|s| s.body.clone()).unwrap_or_default(),
            _ => String::new(),
        };
        let shared_signature = matches!(account.signature, SignatureBinding::Id(_)) && store.accounts.iter().filter(|a| a.signature == account.signature).count() > 1;
        let item = SyncItem {
            schema: sync::SCHEMA,
            kind: sync::KIND.into(),
            updated_at: 0,
            credential_updated_at: 0,
            account: SyncAccount {
                provider_id: account.resolved_provider_id().into(),
                name: account.name.clone(),
                email: account.email.trim().to_lowercase(),
                color: account.color,
                imap_host: account.imap_host.clone(),
                imap_port: account.imap_port,
                smtp_host: account.smtp_host.clone(),
                smtp_port: account.smtp_port,
                username: account.username.clone(),
                oauth_client_id: account.oauth_client_id.clone(),
                archive_folder: account.archive_folder.clone(),
                trash_folder: account.trash_folder.clone(),
            },
            credential: SyncCredential {
                kind: if google { sync::CREDENTIAL_GOOGLE } else { sync::CREDENTIAL_PASSWORD }.into(),
                value,
                client_secret,
            },
            mail_signature: signature,
        };
        let (account_mac, credential_mac) = self.macs(&item, shared_signature)?;
        Ok(Some(Local { shared_signature, invalid: item.validate().err(), account_id: account.id, id: item.id(), item, account_mac, credential_mac }))
    }
    /// The HMACs of the two parts of an item: the mailbox's settings with its signature, and the
    /// credential with the client it was issued for.
    fn macs(&self, item: &SyncItem, shared_signature: bool) -> AppResult<(String, String)> {
        let mut account = item.account.clone();
        account.oauth_client_id.clear();
        let signature = if shared_signature { "" } else { item.mail_signature.as_str() };
        let account_part = serde_json::to_vec(&(&account, signature)).map_err(|_| AppError::storage("The synchronization state could not be computed"))?;
        // The client's secret is left out: it follows the client id and only some computers have it.
        let credential_part = serde_json::to_vec(&(&item.credential.kind, &item.credential.value, &item.account.oauth_client_id)).map_err(|_| AppError::storage("The synchronization state could not be computed"))?;
        Ok((self.mac(&account_part)?, self.mac(&credential_part)?))
    }
    /// The two times of a local mailbox: the remembered ones while it is as the server has it,
    /// `now` for a part that changed here, `untracked` for a mailbox the state never saw.
    fn stamp(local: &mut Local, tracked: Option<&Tracked>, now: i64, untracked: i64) {
        local.item.updated_at = match tracked {
            Some(t) if t.account_mac == local.account_mac => t.updated_at,
            Some(_) => now,
            None => untracked,
        };
        local.item.credential_updated_at = match tracked {
            Some(t) if t.credential_mac == local.credential_mac => t.credential_updated_at,
            Some(_) => now,
            None => untracked,
        };
    }
    /// The merge of a local mailbox with the server's copy. What was not changed here since the
    /// server last had it (`tracked`, compared by HMAC) is the server's, whole. A mailbox the
    /// state never saw takes the server's settings and keeps its own credential when that
    /// differs: a credential that works here is not overwritten by one nobody has tried (it goes
    /// up instead, and the other computers follow).
    fn merged(&self, local: &Local, tracked: Option<&Tracked>, remote: &SyncItem, now: i64) -> AppResult<SyncItem> {
        let Some(t) = tracked else {
            let differs = self.macs(remote, local.shared_signature)?.1 != local.credential_mac;
            let mut merged = merge_changed(&local.item, remote, now, false, false);
            if differs {
                merged.credential = local.item.credential.clone();
                merged.account.oauth_client_id = local.item.account.oauth_client_id.clone();
                merged.credential_updated_at = now;
            }
            return Ok(merged);
        };
        // Version zero marks a tracked row in a restored history. Neither side is a
        // common baseline there: compare the known times for both parts once.
        let rebased = t.version == 0;
        Ok(merge_changed(&local.item, remote, now, rebased || t.account_mac != local.account_mac, rebased || t.credential_mac != local.credential_mac))
    }
    fn track(&self, version: u64, item: &SyncItem, shared_signature: bool) -> AppResult<Tracked> {
        let (account_mac, credential_mac) = self.macs(item, shared_signature)?;
        Ok(Tracked { version, account_mac, credential_mac, updated_at: item.updated_at, credential_updated_at: item.credential_updated_at })
    }

    // ---- the pass -------------------------------------------------------------------------

    pub fn sync_now(&self) -> AppResult<SyncReport> {
        let Ok(_running) = self.run.try_lock() else { return Ok(SyncReport { skipped: true, ..Default::default() }) };
        let Some((endpoint, user_id)) = self.conditions()? else { return Ok(SyncReport { skipped: true, ..Default::default() }) };
        let now = self.context.clock.now();
        let start = self.with_state(|state| {
            if state.url.is_empty() && state.user_id.is_empty() {
                // The first pass: what the person switched off before it stays off.
                state.url = endpoint.url.clone();
                state.user_id = user_id.clone();
            } else if state.url != endpoint.url || state.user_id != user_id {
                *state = state.restarted(endpoint.url.clone(), user_id.clone());
            }
            Ok(state.clone())
        })?;
        let mut state = start.clone();
        let mut report = SyncReport::default();
        let result = self.pass(&endpoint, &mut state, now, &mut report);
        if matches!(&result, Err(error) if error.code == "sync_session_changed") {
            return Ok(SyncReport { skipped: true, ..Default::default() });
        }
        // What was learned is kept even when a later step failed, on top of what the state became
        // meanwhile; a state that cannot be saved is not a success.
        let saved = self.with_state(|current| {
            if current.url == start.url && current.user_id == start.user_id {
                current.absorb(&start, &state);
            }
            Ok(())
        });
        let result = result.and(saved);
        self.note_refusal(&result);
        let mut outcome = lock(&self.outcome);
        *outcome = Outcome::default();
        match result {
            Ok(()) => {}
            Err(error) if error.code == "subscription_required" => {
                outcome.paused = true;
                report.paused = true;
            }
            Err(error) if error.code == "not_found" => {
                outcome.unavailable = true;
                report.unavailable = true;
            }
            Err(error) => {
                outcome.error = Some(error.message.clone());
                return Err(error);
            }
        }
        Ok(report)
    }

    /// The server no longer knows this device (`unauthorized`): announced once (a Windows notification,
    /// desktop `notifications.rs`), and again only after a pass that worked. Every other failure is
    /// not an expired sign-in.
    fn note_refusal(&self, result: &AppResult<()>) {
        use std::sync::atomic::Ordering;
        match result {
            Err(error) if error.code == "unauthorized" => {
                if !self.refused.swap(true, Ordering::SeqCst) {
                    self.context.events.emit(AppEvent::SignInExpired { account_id: None });
                }
            }
            Ok(()) => self.refused.store(false, Ordering::SeqCst),
            Err(_) => {}
        }
    }

    /// The endpoint and the user when the synchronization may talk to the server: a session, the
    /// consent for its address, the general switch (INV-35).
    fn conditions(&self) -> AppResult<Option<(RemoteEndpoint, String)>> {
        let Some(session) = self.accounts.session()? else {
            // Signed out: the server's state goes, the switches of the mailboxes stay.
            self.with_state(|state| {
                *state = state.restarted(String::new(), String::new());
                Ok(())
            })?;
            return Ok(None);
        };
        let settings = self.context.read_shared()?.settings.clone();
        if !settings.mailbox_sync || settings.mailbox_sync_consent.as_deref() != Some(session.url.as_str()) {
            return Ok(None);
        }
        let user_id = session.user_id.clone();
        Ok(Some((self.accounts.endpoint_for_session(session)?, user_id)))
    }

    fn check_session(&self, endpoint: &RemoteEndpoint) -> AppResult<()> {
        let session = self.accounts.session()?;
        let store = self.context.read_shared()?;
        if session.is_some_and(|s| s.url == endpoint.url && s.device_key == endpoint.device_key)
            && store.settings.mailbox_sync
            && store.settings.mailbox_sync_consent.as_deref() == Some(endpoint.url.as_str())
        {
            return Ok(());
        }
        Err(AppError::new("sync_session_changed", "Mailbox synchronization settings changed"))
    }

    fn pass(&self, endpoint: &RemoteEndpoint, state: &mut SyncState, now: i64, report: &mut SyncReport) -> AppResult<()> {
        let full = state.epoch.is_none() || now - state.last_full > FULL_LISTING_SECS || now < state.last_full;
        let mut listing = self.server.list(endpoint, if full { 0 } else { state.since })?;
        let mut full = full;
        let last_seen = state.items.values().map(|item| item.version)
            .chain(state.pending_deletes.iter().map(|delete| delete.version))
            .fold(state.since, u64::max);
        if state.epoch.is_some_and(|epoch| epoch != listing.epoch) || listing.version < last_seen {
            if !full {
                listing = self.server.list(endpoint, 0)?;
                full = true;
            }
            // Missing rows can be uploaded again. Keep the baseline of rows that still
            // exist: a tombstone is a real removal even after restore, and a newer local
            // edit must survive an older restored item. Old version numbers cannot be compared.
            let restored: BTreeSet<&str> = listing.items.iter().map(|item| item.id.as_str()).collect();
            state.items.retain(|id, tracked| {
                tracked.version = 0;
                restored.contains(id.as_str())
            });
            state.since = 0;
        }
        self.check_session(endpoint)?;
        let mut applied = false;
        // 1. Download.
        let mut present: BTreeSet<String> = BTreeSet::new();
        for remote in &listing.items {
            present.insert(remote.id.clone());
            report.downloaded += 1;
            self.check_session(endpoint)?;
            applied |= self.apply_remote(state, remote, now, report)?;
        }
        if full {
            // A mailbox this computer synchronized and the server no longer lists was removed.
            let vanished: Vec<String> = state.items.keys().filter(|id| !present.contains(*id)).cloned().collect();
            for id in vanished {
                self.note_removed(state, &id, now, report)?;
            }
            state.last_full = now;
            // What the server refused is asked again at least this often.
            state.rejected.clear();
        }
        state.epoch = Some(listing.epoch);
        state.since = state.since.max(listing.version);
        // 2. Upload.
        let store = self.context.read_shared()?;
        let settings = store.settings.clone();
        let mut first_error = None;
        let mut known_ids = BTreeSet::new();
        for account in &store.accounts {
            let Some(mut local) = self.local(&store, &settings, account)? else { continue };
            known_ids.insert(local.id.clone());
            // A new sign-in after a revocation: the mailbox has a credential again.
            state.needs_reconnect.remove(&local.id);
            if state.disabled.contains(&local.id) || state.removals.iter().any(|r| r.item_id == local.id) {
                continue;
            }
            if local.invalid.is_some() {
                let reason = "This mailbox cannot be shared: its settings or credential are not valid".to_string();
                state.rejected.insert(local.id.clone(), Rejected { account_mac: local.account_mac.clone(), credential_mac: local.credential_mac.clone(), reason });
                continue;
            }
            if let Some(known) = state.rejected.get(&local.id) {
                if known.account_mac == local.account_mac && known.credential_mac == local.credential_mac {
                    continue;
                }
                state.rejected.remove(&local.id);
            }
            let tracked = state.items.get(&local.id).cloned();
            if tracked.as_ref().is_some_and(|t| t.account_mac == local.account_mac && t.credential_mac == local.credential_mac) {
                continue;
            }
            Self::stamp(&mut local, tracked.as_ref(), now, now);
            let (id, account_mac, credential_mac) = (local.id.clone(), local.account_mac.clone(), local.credential_mac.clone());
            match self.upload(endpoint, state, local, tracked, now, report, &mut applied) {
                Ok(()) => {}
                Err(error) if matches!(error.code, "subscription_required" | "unauthorized" | "not_found" | "unreachable" | "sync_session_changed") => return Err(error),
                // This mailbox alone is the problem: it is not sent again until it changes.
                Err(error) if matches!(error.code, "invalid_request" | "payload_too_large" | "sync_limit" | "unsupported_schema") => {
                    let reason = match error.code {
                        "unsupported_schema" => "This mailbox was saved by a newer version of Qamille",
                        "payload_too_large" => "This mailbox is too large for the server",
                        "sync_limit" => "The server does not take more synchronized mailboxes",
                        _ => "The server did not accept this mailbox",
                    };
                    state.rejected.insert(id, Rejected { account_mac, credential_mac, reason: reason.into() });
                }
                Err(error) => first_error = first_error.or(Some(error)),
            }
        }
        state.rejected.retain(|id, _| known_ids.contains(id));
        // 3. A mailbox removed here is removed there, and so is one switched off here: it stays
        // only on this computer, so the server no longer keeps its credential.
        let switched_off: Vec<(String, u64)> = state.items.iter().filter(|(id, _)| state.disabled.contains(*id)).map(|(id, tracked)| (id.clone(), tracked.version)).collect();
        for (item_id, version) in switched_off {
            state.items.remove(&item_id);
            state.pending_deletes.push(PendingDelete { item_id, version });
        }
        let gone: Vec<(String, u64)> = state
            .items
            .iter()
            .filter(|(id, _)| !known_ids.contains(*id) && !state.disabled.contains(*id) && !state.removals.iter().any(|r| &r.item_id == *id))
            .map(|(id, tracked)| (id.clone(), tracked.version))
            .collect();
        for (item_id, version) in gone {
            // A mailbox waiting to be reconnected is not gone: only a mailbox without an account is.
            let still_there = store.accounts.iter().any(|a| self::item_id(a.resolved_provider_id(), &a.email) == item_id);
            if !still_there {
                state.items.remove(&item_id);
                state.pending_deletes.push(PendingDelete { item_id, version });
            }
        }
        let mut pending = std::mem::take(&mut state.pending_deletes);
        // A mailbox that was added again here meanwhile is not deleted behind its back.
        pending.retain(|d| state.disabled.contains(&d.item_id) || !store.accounts.iter().any(|a| self::item_id(a.resolved_provider_id(), &a.email) == d.item_id));
        for delete in pending {
            self.check_session(endpoint)?;
            let sent = match self.server.delete(endpoint, &delete.item_id, Some(delete.version)) {
                // Changed elsewhere since: the person removed it here on purpose, so it goes.
                Err(error) if error.code == "sync_conflict" => {
                    self.check_session(endpoint)?;
                    self.server.delete(endpoint, &delete.item_id, None)
                }
                other => other,
            };
            match sent {
                Ok(()) => {}
                Err(error) => {
                    state.pending_deletes.push(delete);
                    first_error = first_error.or(Some(error));
                }
            }
        }
        state.last_sync = Some(now);
        if applied {
            self.context.events.emit(AppEvent::AccountsChanged);
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Sends one mailbox, merging with the server's copy on a conflict.
    #[allow(clippy::too_many_arguments)]
    fn upload(
        &self,
        endpoint: &RemoteEndpoint,
        state: &mut SyncState,
        mut local: Local,
        tracked: Option<Tracked>,
        now: i64,
        report: &mut SyncReport,
        applied: &mut bool,
    ) -> AppResult<()> {
        let mut base = tracked.as_ref().map_or(0, |t| t.version);
        for _ in 0..MAX_ATTEMPTS {
            self.check_session(endpoint)?;
            match self.server.put(endpoint, &local.id, base, &local.item)? {
                PutOutcome::Written { version } => {
                    state.items.insert(local.id.clone(), self.track(version, &local.item, local.shared_signature)?);
                    report.uploaded += 1;
                    return Ok(());
                }
                PutOutcome::Conflict(None) => base = 0,
                PutOutcome::Conflict(Some(current)) if !current.deleted && current.item.is_none() => {
                    return Err(AppError::new("unsupported_schema", "This mailbox was saved by a newer version of Qamille"));
                }
                PutOutcome::Conflict(Some(current)) => match current.item.filter(|_| !current.deleted) {
                    Some(remote) => {
                        // The copy of another mailbox under this id is not merged into this one.
                        if remote.schema > sync::SCHEMA {
                            return Err(AppError::new("unsupported_schema", "This mailbox was saved by a newer version of Qamille"));
                        }
                        if remote.validate().is_err() || remote.id() != local.id {
                            return Err(AppError::new("server_error", "The server refused the mailbox item"));
                        }
                        let merged = self.merged(&local, state.items.get(&local.id), &remote, now)?;
                        *applied |= self.apply_item(&merged, Some(local.account_id), now)?;
                        // What the server holds now is the other copy; the merged one goes up next.
                        state.items.insert(local.id.clone(), self.track(current.version, &remote, local.shared_signature)?);
                        (local.account_mac, local.credential_mac) = self.macs(&merged, local.shared_signature)?;
                        local.item = merged;
                        base = current.version;
                    }
                    None => {
                        // A tombstone: a mailbox that was synchronized is a removal, a new one is recreated.
                        if tracked.is_some() {
                            self.note_removed(state, &local.id, now, report)?;
                            return Ok(());
                        }
                        base = current.version;
                    }
                },
            }
        }
        Err(AppError::new("sync_conflict", "The mailbox changed on another computer; it will be retried"))
    }

    /// Applies one item of the listing to this computer. Returns whether a mailbox changed.
    fn apply_remote(&self, state: &mut SyncState, remote: &RemoteItem, now: i64, report: &mut SyncReport) -> AppResult<bool> {
        if !sync::valid_item_id(&remote.id) {
            return Ok(false);
        }
        // A mailbox removed (or switched off) here is waiting for its delete: the server's copy
        // is not brought back by the listing that comes before the delete.
        if state.pending_deletes.iter().any(|d| d.item_id == remote.id) {
            return Ok(false);
        }
        if remote.deleted {
            if state.disabled.contains(&remote.id) {
                state.items.remove(&remote.id);
            } else if state.items.contains_key(&remote.id) {
                self.note_removed(state, &remote.id, now, report)?;
            }
            return Ok(false);
        }
        let Some(item) = &remote.item else { return Ok(false) };
        // An item that does not pass, or that says another address than its id, is not applied.
        if item.validate().is_err() || item.id() != remote.id {
            return Ok(false);
        }
        state.removals.retain(|r| r.item_id != remote.id);
        if state.disabled.contains(&remote.id) {
            return Ok(false);
        }
        if state.items.get(&remote.id).is_some_and(|t| t.version >= remote.version) {
            return Ok(false);
        }
        let store = self.context.read_shared()?;
        let settings = store.settings.clone();
        let existing = store.accounts.iter().find(|a| item_id(a.resolved_provider_id(), &a.email) == remote.id).cloned();
        let tracked = state.items.get(&remote.id).cloned();
        let waiting = state.needs_reconnect.contains(&remote.id);
        let mut shared_signature = false;
        let merged = match &existing {
            Some(account) => match self.local(&store, &settings, account)? {
                // A mailbox that cannot be shared as it is takes the server's copy whole.
                Some(local) if local.invalid.is_some() => item.clone(),
                Some(mut local) => {
                    shared_signature = local.shared_signature;
                    // What was not changed here is the server's, whole; what was, is decided by the times.
                    Self::stamp(&mut local, tracked.as_ref(), now, now);
                    self.merged(&local, tracked.as_ref(), item, now)?
                }
                // Revoked here: the token still on the server must not come back.
                None if waiting => return Ok(false),
                None => item.clone(),
            },
            None => item.clone(),
        };
        let changed = self.apply_item(&merged, existing.as_ref().map(|a| a.id), now)?;
        state.items.insert(remote.id.clone(), self.track(remote.version, item, shared_signature)?);
        Ok(changed)
    }

    fn note_removed(&self, state: &mut SyncState, id: &str, _now: i64, report: &mut SyncReport) -> AppResult<()> {
        state.items.remove(id);
        let store = self.context.read_shared()?;
        let Some(account) = store.accounts.iter().find(|a| item_id(a.resolved_provider_id(), &a.email) == id) else { return Ok(()) };
        if state.removals.iter().any(|r| r.item_id == id) {
            return Ok(());
        }
        state.removals.push(RemovalNotice { item_id: id.into(), name: account.name.clone(), email: account.email.clone() });
        report.removal_notices += 1;
        self.context.events.emit(AppEvent::Notice { text: format!("The mailbox {} was removed or switched off on another computer", account.email) });
        Ok(())
    }

    /// Writes `item` into this computer: the credential in the vault first, then the mailbox, its
    /// signature and, for a personal Google client, the client's id and secret if missing.
    fn apply_item(&self, item: &SyncItem, existing: Option<u32>, now: i64) -> AppResult<bool> {
        let provider = item.account.provider_id.as_str();
        let (id, was_present) = match existing {
            Some(id) => (id, true),
            None => self.context.update(|store| {
                if let Some(account) = store.accounts.iter().find(|a| a.resolved_provider_id() == provider && a.email.eq_ignore_ascii_case(&item.account.email)) {
                    return Ok((account.id, true));
                }
                let id = store.allocate_mailbox_id().ok_or_else(|| AppError::storage("Mailbox identifiers exhausted"))?;
                Ok((id, false))
            })?,
        };
        let key = if provider == "gmail" { account_key(&item.account.email) } else { provider_account_key(provider, id) };
        self.context.update(|store| {
            let mut touched = false;
            let position = store.accounts.iter().position(|a| a.resolved_provider_id() == provider && a.email.eq_ignore_ascii_case(&item.account.email));
            let index = match position {
                Some(index) if store.accounts[index].id == id => index,
                Some(_) => return Err(AppError::storage("The mailbox identity changed during synchronization; it will be retried")),
                None => {
                    if was_present || store.account(id).is_some() {
                        return Err(AppError::storage("The mailbox list changed while it was being synchronized; it will be retried"));
                    }
                    store.accounts.push(Account {
                        id,
                        provider_id: provider.into(),
                        email: item.account.email.clone(),
                        kind: AccountKind::Gmail,
                        signature: SignatureBinding::MostRecent,
                        fetch_count: 200,
                        ..Default::default()
                    });
                    touched = true;
                    store.accounts.len() - 1
                }
            };
            // Under the mailbox writer lock, a retirement cannot mistake this credential for its own.
            if self.context.secrets.get(&key)? != item.credential.value {
                self.context.secrets.set(&key, &item.credential.value)?;
                if provider == "gmail" || !item.account.oauth_client_id.is_empty() {
                    forget_access_hook(&item.account.email);
                }
                touched = true;
            }
            if let Some(secret) = item.credential.client_secret.as_deref().filter(|secret| !secret.trim().is_empty())
                .filter(|_| store.settings.google_client_id.trim().is_empty())
                .filter(|_| !item.account.oauth_client_id.is_empty() && !self.clients.is_app_client(&item.account.oauth_client_id))
            {
                self.context.secrets.set(GOOGLE_SECRET, secret)?;
                store.settings.google_client_id = item.account.oauth_client_id.clone();
                touched = true;
            }
            let a = &mut store.accounts[index];
            let a_before = (a.name.clone(), a.color, a.imap_host.clone(), a.imap_port, a.smtp_host.clone(), a.smtp_port, a.username.clone(), a.oauth_client_id.clone(), a.archive_folder.clone(), a.trash_folder.clone());
            a.name = item.account.name.clone();
            a.color = item.account.color;
            a.imap_host = item.account.imap_host.clone();
            a.imap_port = item.account.imap_port;
            a.smtp_host = item.account.smtp_host.clone();
            a.smtp_port = item.account.smtp_port;
            a.username = item.account.username.clone();
            a.oauth_client_id = item.account.oauth_client_id.clone();
            a.archive_folder = item.account.archive_folder.clone();
            a.trash_folder = item.account.trash_folder.clone();
            let a_after = (a.name.clone(), a.color, a.imap_host.clone(), a.imap_port, a.smtp_host.clone(), a.smtp_port, a.username.clone(), a.oauth_client_id.clone(), a.archive_folder.clone(), a.trash_folder.clone());
            touched |= a_before != a_after;
            // The signature: the text travels, the reference is local.
            let binding = store.accounts[index].signature;
            let text = &item.mail_signature;
            match binding {
                SignatureBinding::Id(sig_id) if store.signatures.iter().any(|s| s.id == sig_id) => {
                    // A signature that two mailboxes use is not rewritten for one of them.
                    let shared = store.accounts.iter().filter(|a| a.signature == binding).count() > 1;
                    let signature = store.signatures.iter_mut().find(|s| s.id == sig_id).expect("checked above");
                    if !shared && &signature.body != text && !text.is_empty() {
                        signature.body = text.clone();
                        signature.modified = now;
                        touched = true;
                    }
                }
                _ if !text.is_empty() => {
                    let sig_id = store.signatures.iter().map(|s| s.id).max().unwrap_or(0) + 1;
                    store.signatures.push(Signature { id: sig_id, name: format!("Firma di {}", item.account.email), body: text.clone(), modified: now, html: false });
                    store.accounts[index].signature = SignatureBinding::Id(sig_id);
                    touched = true;
                }
                _ => {}
            }
            Ok(touched)
        })
    }

    // ---- revocation (S5) ---------------------------------------------------------------------

    /// Asks Google to revoke the token of one mailbox. On success the credential leaves the vault
    /// (the mailbox asks to be signed in again) and the old token that the server still holds is
    /// not taken back. A password for apps is reported, not touched.
    pub fn revoke_mailbox(&self, account_id: u32) -> AppResult<MailboxRevoke> {
        let store = self.context.read_shared()?;
        let account = store.account(account_id).cloned().ok_or_else(|| AppError::missing("Account"))?;
        let result = |outcome| MailboxRevoke { account_id, email: account.email.clone(), provider_id: account.resolved_provider_id().to_string(), outcome };
        if account.oauth_client_id.is_empty() {
            return Ok(result(RevokeOutcome::ManualPassword));
        }
        let key = credential_key(&account);
        let token = self.context.secrets.get(&key)?;
        let has_token = !token.trim().is_empty();
        if has_token && self.revoker.revoke(&token).is_err() {
            return Ok(result(RevokeOutcome::Failed));
        }
        // The reminder is saved before the token leaves the vault: if it cannot be saved, the
        // token is still there and the old one on the server cannot come back instead of it.
        self.with_state(|state| {
            state.needs_reconnect.insert(item_id(account.resolved_provider_id(), &account.email));
            Ok(())
        })?;
        if has_token {
            self.context.secrets.remove(&key)?;
            forget_access_hook(&account.email);
        }
        Ok(result(RevokeOutcome::Revoked))
    }
    /// The same for every mailbox that this computer synchronizes: a mailbox that stays only
    /// here keeps working. One that fails does not stop the others: it is reported as `Failed`.
    pub fn revoke_all(&self) -> AppResult<Vec<MailboxRevoke>> {
        let store = self.context.read_shared()?;
        let state = self.read_state()?;
        let synchronized = store
            .accounts
            .iter()
            .filter(|a| a.kind != AccountKind::Sample && a.resolved_provider_id() != "sample")
            .filter(|a| state.items.contains_key(&item_id(a.resolved_provider_id(), &a.email)));
        Ok(synchronized
            .map(|a| {
                self.revoke_mailbox(a.id).unwrap_or_else(|_| MailboxRevoke {
                    account_id: a.id,
                    email: a.email.clone(),
                    provider_id: a.resolved_provider_id().to_string(),
                    outcome: RevokeOutcome::Failed,
                })
            })
            .collect())
    }

    // ---- removals and status --------------------------------------------------------------

    /// «Rimuovi anche qui» (`remove_here`) or «Tienila solo qui».
    pub fn resolve_removal(&self, item_id: &str, remove_here: bool, accounts: &super::accounts::AccountService) -> AppResult<()> {
        if !self.read_state()?.removals.iter().any(|r| r.item_id == item_id) {
            return Ok(());
        }
        if remove_here {
            let store = self.context.read_shared()?;
            let target = store.accounts.iter().find(|a| self::item_id(a.resolved_provider_id(), &a.email) == item_id).map(|a| a.id);
            if let Some(id) = target {
                accounts.remove_account(id)?;
            }
        }
        self.with_state(|state| {
            if !remove_here {
                state.disabled.insert(item_id.into());
            }
            state.removals.retain(|r| r.item_id != item_id);
            state.items.remove(item_id);
            Ok(())
        })
    }
    /// A mailbox was removed on this computer: the server's item is deleted at the next pass.
    pub fn after_local_removal(&self, provider_id: &str, email: &str) -> AppResult<()> {
        let id = item_id(provider_id, email);
        let queued = self.with_state(|state| {
            state.disabled.remove(&id);
            state.removals.retain(|r| r.item_id != id);
            let Some(tracked) = state.items.remove(&id) else { return Ok(false) };
            state.pending_deletes.retain(|d| d.item_id != id);
            state.pending_deletes.push(PendingDelete { item_id: id.clone(), version: tracked.version });
            Ok(true)
        })?;
        if queued {
            self.nudge();
        }
        Ok(())
    }

    pub fn status(&self) -> AppResult<SyncStatus> {
        let store = self.context.read_shared()?;
        let settings = &store.settings;
        let session = self.accounts.session()?;
        let state = self.read_state()?;
        let consented = session.as_ref().is_some_and(|s| settings.mailbox_sync_consent.as_deref() == Some(s.url.as_str()));
        let declared = match &session {
            Some(_) => self.accounts.current_info()?.map(|info| info.sync.is_some_and(|s| s.available)),
            None => None,
        };
        let outcome = lock(&self.outcome);
        let mode = if session.is_none() {
            SyncMode::NoServer
        } else if declared == Some(false) || outcome.unavailable {
            SyncMode::Unavailable
        } else if !settings.mailbox_sync || !consented {
            SyncMode::Off
        } else if outcome.paused {
            SyncMode::PausedForSubscription
        } else if outcome.error.is_some() {
            SyncMode::Error
        } else {
            SyncMode::Idle
        };
        let mailboxes = store
            .accounts
            .iter()
            .filter(|a| a.kind != AccountKind::Sample)
            .map(|a| MailboxSwitch {
                account_id: a.id,
                enabled: !state.disabled.contains(&item_id(a.resolved_provider_id(), &a.email)),
                needs_reconnect: state.needs_reconnect.contains(&item_id(a.resolved_provider_id(), &a.email)),
                problem: state.rejected.get(&item_id(a.resolved_provider_id(), &a.email)).map(|r| r.reason.clone()),
            })
            .collect();
        Ok(SyncStatus {
            mode,
            enabled: settings.mailbox_sync,
            consented,
            last_sync: state.last_sync,
            error: outcome.error.clone(),
            removals: state.removals,
            mailboxes,
        })
    }
}

/// A credential changed on this computer: the cached access token for the address is no longer
/// valid. The application layer cannot name the infrastructure, so the hook is set at startup.
static FORGET_ACCESS: Mutex<Option<Box<dyn Fn(&str) + Send>>> = Mutex::new(None);
pub fn set_forget_access(hook: Box<dyn Fn(&str) + Send>) {
    *lock(&FORGET_ACCESS) = Some(hook);
}
pub(super) fn forget_access_hook(email: &str) {
    if let Some(hook) = lock(&FORGET_ACCESS).as_ref() {
        hook(email);
    }
}
