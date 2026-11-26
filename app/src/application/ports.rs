use super::{AppError, AppResult};
use crate::domain::activity::{
    ACTIVITY_RETENTION_MS, ActivityEntry, ActivityFilter, ActivityOutcome, NewActivity,
};
use crate::domain::model::{Account, Attachment, Category, Email, Settings, Store};

/// Atomic read-modify-write of the mail aggregate. An adapter must invoke the
/// operation once and commit only on Ok. Network work must stay outside it.
pub trait StoreRepository: Send + Sync {
    fn read(&self) -> AppResult<Store>;
    /// Only a workspace-wide grant includes its shared (unowned) address book.
    fn includes_local_contacts(&self) -> bool { true }
    /// The committed store, shared and read-only. Adapters that can keep it
    /// between writes (SQLite: until `PRAGMA data_version` moves) avoid a full
    /// read per call; the default reads.
    fn read_shared(&self) -> AppResult<std::sync::Arc<Store>> {
        self.read().map(std::sync::Arc::new)
    }
    fn transaction(&self, operation: &mut dyn FnMut(&mut Store) -> AppResult<()>) -> AppResult<()>;
    /// `transaction` for an operation that changes no message except those
    /// whose id is in `emails` (it may change or remove them); everything else
    /// in the store (accounts, undo actions, drafts, settings…) it may change
    /// freely. Adding messages or reordering them stays correct but forfeits
    /// the benefit. An adapter may then skip copying and comparing the whole
    /// mailbox (SQLite: the committed snapshot is changed in place and only the
    /// listed messages are compared), so a change to an unlisted message may
    /// be lost: debug builds check the promise and panic when it is broken.
    /// The default runs `transaction`.
    fn transaction_on_emails(
        &self,
        emails: &[u64],
        operation: &mut dyn FnMut(&mut Store) -> AppResult<()>,
    ) -> AppResult<()> {
        let _ = emails;
        self.transaction(operation)
    }
    /// Inserts or enriches metadata-only rows of one account (history,
    /// indexing, server search): a known message, by `same_message`, keeps its
    /// local state and gains identity and location (`merge_remote`); a new one
    /// gets an id. One transaction; rows that would not fit the quota fail it.
    fn upsert_remote_metadata(&self, account_id: u32, rows: Vec<Email>) -> AppResult<UpsertOutcome> {
        let mut outcome = UpsertOutcome::default();
        self.transaction(&mut |store| {
            let (inserted, enriched) = crate::domain::mail_identity::merge_rows(store, account_id, &rows);
            outcome = UpsertOutcome { inserted, enriched };
            Ok(())
        })?;
        Ok(outcome)
    }
    /// Measured auxiliary bytes owned by messages (e.g. semantic vectors).
    fn message_storage_bytes(&self, _store: &Store) -> AppResult<std::collections::HashMap<u64, u64>> {
        Ok(std::collections::HashMap::new())
    }
    /// Unique workspace-owned draft files, excluding bytes already charged elsewhere.
    fn draft_storage_bytes(&self, _store: &Store) -> AppResult<std::collections::HashMap<u32, crate::domain::storage::DraftStorageBytes>> {
        Ok(std::collections::HashMap::new())
    }
    /// Removes only rebuildable entries whose source no longer exists locally.
    fn reconcile_storage(&self) -> AppResult<()> { Ok(()) }
    /// Committed changes from OTHER processes since `since` (exclusive), in
    /// feed order (Task 4.3, EX-005). Rows of the calling process are omitted:
    /// its writers already announced them. The default is an empty feed — the
    /// JSON-store era, where the interim scheduler cap in the desktop bridges
    /// cross-process deadlines instead.
    fn poll_change_feed(&self, _since: i64) -> AppResult<Vec<ChangeFeedRow>> { Ok(Vec::new()) }
    /// The feed's current tail: a poller starts here instead of replaying a
    /// history it has no context for.
    fn change_feed_head(&self) -> AppResult<i64> { Ok(0) }
    /// Fold the write-ahead log, return free pages and prune the feed. For the
    /// maintenance job only (Task 4.3 wiring of review 4.2 minor 3); stores
    /// without a log do nothing.
    fn storage_checkpoint(&self) -> AppResult<()> { Ok(()) }
    /// The activity log of this store's data directory (centro attività), or `None` when the store
    /// keeps none (a preview, or a test double). The log lives in its own file whatever engine the
    /// mail store runs on; the methods below all go through it.
    fn activity_log(&self) -> Option<&dyn ActivityStore> { None }
    /// Whether this store keeps a history; the UI says so when it does not.
    fn activity_supported(&self) -> bool { self.activity_log().is_some() }
    /// Appends one row at `at_ms` and returns its `seq`; `None` when the store keeps no log.
    fn append_activity(&self, entry: &NewActivity, at_ms: i64) -> AppResult<Option<i64>> {
        match self.activity_log() {
            Some(log) => log.append(entry, at_ms).map(Some),
            None => Ok(None),
        }
    }
    /// Sets the outcome (and the detail, when given) of the row whose `undo_id` matches and
    /// returns its `seq`; `None` when no row matches. The only update the log allows.
    fn set_activity_outcome(
        &self,
        undo_id: u64,
        outcome: ActivityOutcome,
        detail: Option<&str>,
    ) -> AppResult<Option<i64>> {
        match self.activity_log() {
            Some(log) => log.set_outcome(undo_id, outcome, detail),
            None => Ok(None),
        }
    }
    /// Newest first; `before_seq` is exclusive.
    fn activity_page(
        &self,
        before_seq: Option<i64>,
        limit: u32,
        filter: ActivityFilter,
    ) -> AppResult<Vec<ActivityEntry>> {
        match self.activity_log() {
            Some(log) => log.page(before_seq, limit, filter),
            None => Ok(Vec::new()),
        }
    }
    /// The newest row whose code is one of `codes` for this account (`None` matches the rows
    /// without an account).
    fn last_activity(&self, codes: &[&str], account_id: Option<u32>) -> AppResult<Option<ActivityEntry>> {
        match self.activity_log() {
            Some(log) => log.last(codes, account_id),
            None => Ok(None),
        }
    }
    /// When the user last opened the panel or the page; 0 before the first time.
    fn activity_seen_ms(&self) -> AppResult<i64> {
        match self.activity_log() {
            Some(log) => log.seen_ms(),
            None => Ok(0),
        }
    }
    fn set_activity_seen_ms(&self, at_ms: i64) -> AppResult<()> {
        match self.activity_log() {
            Some(log) => log.set_seen_ms(at_ms),
            None => Ok(()),
        }
    }
    /// Error rows and failed actions that changed after the seen marker.
    fn unseen_activity_errors(&self) -> AppResult<u32> {
        match self.activity_log() {
            Some(log) => log.unseen_errors(),
            None => Ok(0),
        }
    }
    /// The activity feed's tail: a poller starts here instead of replaying history.
    fn activity_feed_head(&self) -> AppResult<i64> {
        match self.activity_log() {
            Some(log) => log.feed_head(),
            None => Ok(0),
        }
    }
    /// The rows OTHER processes wrote or changed since `since`, and the new cursor. The
    /// desktop's poller turns them into `ActivityChanged` (the MCP process has no UI).
    fn poll_activity_feed(&self, since: i64) -> AppResult<(i64, Vec<u64>)> {
        match self.activity_log() {
            Some(log) => log.feed_since(since, std::process::id()),
            None => Ok((since, Vec::new())),
        }
    }
    /// Drops the rows older than [`ACTIVITY_RETENTION_MS`]; for the maintenance job.
    fn prune_activity(&self, now_ms: i64) -> AppResult<()> {
        match self.activity_log() {
            Some(log) => log.prune(now_ms - ACTIVITY_RETENTION_MS),
            None => Ok(()),
        }
    }
}

/// The activity log's storage (centro attività), implemented by `infrastructure::activity_db`.
/// Every method is one short operation; see `StoreRepository` for what each is for.
pub trait ActivityStore: Send + Sync {
    fn append(&self, entry: &NewActivity, at_ms: i64) -> AppResult<i64>;
    fn set_outcome(
        &self,
        undo_id: u64,
        outcome: ActivityOutcome,
        detail: Option<&str>,
    ) -> AppResult<Option<i64>>;
    fn page(
        &self,
        before_seq: Option<i64>,
        limit: u32,
        filter: ActivityFilter,
    ) -> AppResult<Vec<ActivityEntry>>;
    fn last(&self, codes: &[&str], account_id: Option<u32>) -> AppResult<Option<ActivityEntry>>;
    fn seen_ms(&self) -> AppResult<i64>;
    fn set_seen_ms(&self, at_ms: i64) -> AppResult<()>;
    fn unseen_errors(&self) -> AppResult<u32>;
    fn feed_head(&self) -> AppResult<i64>;
    fn feed_since(&self, since: i64, exclude_pid: u32) -> AppResult<(i64, Vec<u64>)>;
    fn prune(&self, before_ms: i64) -> AppResult<()>;
}

/// One committed cross-process change (SQLite change feed, Task 4.3).
#[derive(Clone, Debug, PartialEq)]
pub struct ChangeFeedRow {
    /// Feed position; a poller advances its cursor with it.
    pub seq: i64,
    /// What changed, in the `AppEvent` kebab-case vocabulary where one exists
    /// (`email-changed`, `drafts-changed`, `undo-changed`, `outbox-changed`,
    /// `accounts-changed`).
    pub kind: String,
    pub account_id: Option<u32>,
    /// The record ids the transaction touched (changed or deleted).
    pub ids: Vec<u64>,
}
impl ChangeFeedRow {
    /// The local UI events one row from another process announces (EX-005).
    /// Kinds a poller cannot synthesize from ids alone (undo rows carry whole
    /// actions; accounts and settings refresh through their own surfaces) map
    /// to nothing; the poller wakes the scheduler separately.
    pub fn events(&self) -> Vec<super::events::AppEvent> {
        use super::events::AppEvent;
        match (self.kind.as_str(), self.account_id) {
            ("email-changed", Some(account_id)) => vec![
                AppEvent::EmailChanged {
                    account_id,
                    ids: self.ids.clone(),
                },
                AppEvent::CountsChanged { account_id },
            ],
            ("drafts-changed", _) => vec![AppEvent::DraftsChanged {
                ids: self.ids.clone(),
            }],
            _ => Vec::new(),
        }
    }
}

/// Single outlet for UI events. Services build `AppEvent`s inside their writer
/// and `ServiceContext::update_with_events` hands them over after the commit.
pub trait EventSink: Send + Sync {
    fn emit(&self, event: super::events::AppEvent);
}

/// Sink that drops every event: tests, MCP and any context without a UI.
pub struct NullSink;
impl EventSink for NullSink {
    fn emit(&self, _event: super::events::AppEvent) {}
}

pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> AppResult<String>;
    fn set(&self, key: &str, value: &str) -> AppResult<()>;
    /// Removes the value: `get` answers empty afterwards. The vault removes a key set to empty.
    fn remove(&self, key: &str) -> AppResult<()> {
        self.set(key, "")
    }
    /// Before a sign-in that ends in `set`: an error when that `set` would be refused (credentials
    /// that a transfer passphrase opens, damaged files), so no login or device is made for nothing.
    fn check_writable(&self) -> AppResult<()> {
        Ok(())
    }
    /// A value read for a sign-in the person starts. Credentials of another Windows user or
    /// computer, with no passphrase to open them, read as empty: the sign-in goes on, and its
    /// `set` sets the old credentials aside. Every other error stays an error.
    fn get_for_sign_in(&self, key: &str) -> AppResult<String> {
        match self.get(key) {
            Err(error) if error.code == super::error::vault_codes::FOREIGN => Ok(String::new()),
            result => result,
        }
    }
    /// The folder a sign-in set the old credentials aside in, until the person was told.
    fn set_aside_notice(&self) -> Option<String> {
        None
    }
}

#[derive(Clone, serde::Serialize)]
pub struct RegionalSettings { pub time_zone: String, pub locale: String, pub warning: Option<String> }

pub trait Clock: Send + Sync {
    fn regional(&self, settings: &Settings) -> RegionalSettings {
        let requested = if settings.time_zone.is_empty() { "UTC" } else { &settings.time_zone };
        let valid = requested.parse::<chrono_tz::Tz>().is_ok();
        RegionalSettings {
            time_zone: if valid { requested.into() } else { "UTC".into() },
            locale: if settings.locale.is_empty() { "en-US".into() } else { settings.locale.clone() },
            warning: (!valid).then(|| "Fuso non riconosciuto: uso UTC / Unknown time zone: using UTC".into()),
        }
    }
    fn now(&self) -> i64;
    fn now_ms(&self) -> i64 {
        self.now() * 1000
    }
}

pub trait AttachmentGateway: Send + Sync {
    /// Stable workspace-owned attachment identities, independent of a machine's folder path.
    fn persistent_paths(&self, paths: &[String]) -> AppResult<Vec<String>> { Ok(paths.to_vec()) }
    /// Preserve outgoing attachment bytes in managed storage before a delivery is queued.
    fn snapshot_outgoing(&self, paths:&[String], _key:&str)->AppResult<Vec<String>> { Ok(paths.to_vec()) }
    fn cleanup_cache(&self, _store: &Store) -> AppResult<()> { Ok(()) }
    fn total_size(&self, paths: &[String]) -> AppResult<u64>;
    fn read_text(&self, attachment: &Attachment) -> AppResult<String>;
    /// Text of a file that exists only as bytes, such as a part inside an
    /// attached message. Adapters with document extractors override this.
    fn text_from_bytes(&self, _name: &str, bytes: &[u8]) -> AppResult<String> {
        Ok(String::from_utf8_lossy(bytes).trim().to_string())
    }
    fn read_bytes(&self, attachment: &Attachment) -> AppResult<Vec<u8>> {
        attachment
            .inline_text
            .as_ref()
            .map(|text| text.as_bytes().to_vec())
            .ok_or_else(|| {
                super::AppError::validation(format!("Attachment unavailable: {}", attachment.name))
            })
    }
}

#[derive(Clone, Debug)]
pub enum RemoteAction {
    Seen(bool),
    Flag(bool),
    Move(String),
    /// Takes the message out of the inbox; it stays in the mailbox.
    Archive,
    Label(String),
    RemoveLabel(String),
    Spam,
}

pub struct SyncBatch {
    pub emails: Vec<Email>,
    pub archive_folder: Option<String>,
    pub trash_folder: Option<String>,
}

/// Coverage is an explicit provider result; a successful recent-inbox sync is
/// not evidence that the whole account is available without a connection.
pub struct PolicySyncBatch {
    pub batch: SyncBatch,
    pub offline_complete: bool,
    pub notice: Option<String>,
}

pub struct SyncRequest<'a> {
    pub max_bytes: Option<u64>,
    pub known: &'a [Email],
}

pub struct OutgoingMessage {
    pub message_id: Option<String>,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub bcc: Vec<String>,
    pub subject: String,
    pub plain: String,
    pub html: Option<String>,
    pub in_reply_to: Option<String>,
    pub attachments: Vec<String>,
}

/// A dedicated notification connection. A timeout is a quiet heartbeat, not a change.
pub trait MailWatch: Send {
    fn wait(&mut self, timeout: std::time::Duration) -> AppResult<bool>;
}

pub trait MailGateway: Send + Sync {
    fn send_remote_draft(&self,_account:&Account,_sender_name:&str,_message:OutgoingMessage,_id:&str,_revision:&str)->AppResult<()> {
        Err(super::AppError::new("unsupported_capability","Sending synchronized cloud drafts is unavailable for this provider"))
    }
    /// Revalidate a persisted MCP grant immediately before a remote delivery.
    fn authorize_mcp_delivery(&self,profile:Option<&str>,_account_id:u32)->AppResult<()> {
        if profile.is_some(){return Err(super::AppError::new("permission_denied","Named profile delivery authorization is unavailable"));}Ok(())
    }
    fn list_labels(&self,_account:&Account)->AppResult<serde_json::Value>{Err(super::AppError::new("unsupported_capability","Provider label management is unavailable"))}
    fn manage_label(&self,_account:&Account,_action:&str,_name:&str,_new_name:Option<&str>)->AppResult<serde_json::Value>{Err(super::AppError::new("unsupported_capability","Provider label management is unavailable"))}
    fn capabilities(&self, account: &Account) -> crate::domain::outbox::ProviderCapabilities {
        let gmail=account.resolved_provider_id()=="gmail";
        crate::domain::outbox::ProviderCapabilities { provider_id: account.resolved_provider_id().into(), sync:gmail,remote_actions:gmail,remote_search:gmail, send: true, max_attachment_bytes: crate::rules::ATTACHMENT_CAP_BYTES, ..Default::default() }
    }
    fn list_remote_drafts(&self, _account: &Account) -> AppResult<Vec<crate::domain::outbox::RemoteDraft>> {
        Err(super::AppError::new("unsupported_capability", "This provider does not support cloud drafts"))
    }
    /// Return `cloud_save_not_started` only when no remote write began. All other
    /// failures are uncertain outcomes and require reconciliation before retry.
    fn save_remote_draft(&self, _account: &Account, _draft: &crate::domain::outbox::RemoteDraft) -> AppResult<crate::domain::outbox::RemoteDraft> {
        Err(super::AppError::new("unsupported_capability", "This provider does not support cloud drafts"))
    }
    fn delete_remote_draft(&self, _account: &Account, _id: &str) -> AppResult<()> {
        Err(super::AppError::new("unsupported_capability", "This provider does not support cloud drafts"))
    }
    fn sync_with_policy(&self, account: &Account, _request: &SyncRequest<'_>) -> AppResult<PolicySyncBatch> {
        Ok(PolicySyncBatch {
            batch: self.sync(account)?,
            offline_complete: false,
            notice: account.storage_policy.keep_all_offline.then(||
                "This provider has not verified a complete offline copy of the account".into()),
        })
    }
    fn load_body(&self, _account: &Account, _email: &Email) -> AppResult<Email> {
        Err(super::AppError::external("This message is online only. Connect to Gmail to download it."))
    }
    /// Reading the message must not wait for document text extraction.
    fn load_body_preview(&self, account: &Account, email: &Email) -> AppResult<Email> {
        self.load_body(account, email)
    }
    /// Recheck access before serving temporary online content from memory.
    fn authorize_read(&self, _account: &Account) -> AppResult<()> { Ok(()) }
    /// Read-only provider search (`X-GM-RAW` for Gmail): metadata only, one page.
    fn search(&self, _account: &Account, _request: &RemoteSearchRequest, _cancel: &std::sync::atomic::AtomicBool) -> AppResult<RemoteSearchPage> {
        Err(super::AppError::external("Server search is unavailable for this account"))
    }
    /// One batch of the history backfill (W3.1): metadata of `request.range`
    /// in All Mail, or only the folder's state when its generation changed.
    fn history_batch(&self, _account: &Account, _request: &HistoryRequest) -> AppResult<HistoryBatch> {
        Err(super::AppError::new("unsupported_capability", "This provider does not bring older mail into the list"))
    }
    fn apply_message(&self, account: &Account, email: &Email, action: RemoteAction) -> AppResult<()> {
        let uid = email.uid.ok_or_else(|| super::AppError::external("Sync this message before applying a server action"))?;
        self.apply(account, uid, action)
    }
    /// Resolve a verified provider identity before an operation invalidates its
    /// current folder UID. RFC Message-ID headers alone are not identity proof.
    fn resolve_identity(&self, _account: &Account, email: &Email) -> AppResult<Email> {
        Ok(email.clone())
    }
    fn watch(&self, _account: &Account) -> AppResult<Box<dyn MailWatch>> {
        Err(super::AppError::new("idle_unsupported", "Mail notifications are unavailable; periodic sync remains active"))
    }
    fn originals(&self, _account: &Account, _emails: &[Email]) -> AppResult<Vec<Vec<u8>>> {
        Err(super::AppError::external(
            "Original message download is unavailable",
        ))
    }
    /// The original bytes of one attachment of a message that is not
    /// downloaded. `email` locates the message and its attachments are the
    /// list the reader showed, `attachment_id` one of their ids. The bytes stay
    /// in memory: online reads never enlarge the offline cache. Only adapters
    /// that can map an id to its part override this.
    fn attachment_bytes(&self, _account: &Account, _email: &Email, _attachment_id: u64) -> AppResult<Vec<u8>> {
        Err(super::AppError::external(
            "Attachment bytes unavailable for this provider",
        ))
    }
    fn thread(&self, _account: &Account, _email: &Email) -> AppResult<Vec<Email>> {
        Err(super::AppError::external(
            "The complete conversation could not be downloaded",
        ))
    }
    fn sync(&self, account: &Account) -> AppResult<SyncBatch>;
    fn apply(&self, account: &Account, uid: u32, action: RemoteAction) -> AppResult<()>;
    fn send(&self, account: &Account, sender_name: &str, message: OutgoingMessage)
    -> AppResult<()>;
    fn check_login(&self, account: &Account) -> AppResult<()>;
}

#[derive(Clone, serde::Serialize)]
pub struct SearchHit {
    pub email_id: u64,
    pub score: f32,
    pub passage: String,
}
pub trait SearchGateway: Send + Sync {
    fn lexical_search(&self, _store:&Store, _query:&str, _account:Option<u32>, _now:i64, _sort:&str)->AppResult<Option<Vec<u64>>> {Ok(None)}
    fn index_document(&self, store: &Store, content: &super::indexing_ports::IndexContent, cancel: &std::sync::atomic::AtomicBool) -> AppResult<super::indexing_ports::DocumentIndexReport> {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) { return Err(super::AppError::external("Indexing paused")); }
        self.index_with_content(store, std::slice::from_ref(&content.email))?;
        Ok(super::indexing_ports::DocumentIndexReport { indexed: true, warnings: content.warnings.clone() })
    }
    /// Words only (BM25), without vectors: the search model is missing (EX-9).
    /// Sources without a word index have nothing to write.
    fn index_lexical(&self, _store: &Store, content: &super::indexing_ports::IndexContent, cancel: &std::sync::atomic::AtomicBool) -> AppResult<super::indexing_ports::DocumentIndexReport> {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) { return Err(super::AppError::new("index_paused", "Indexing paused")); }
        Ok(super::indexing_ports::DocumentIndexReport { indexed: true, warnings: content.warnings.clone() })
    }
    fn search(&self, store: &Store, query: &str, limit: usize) -> AppResult<Vec<SearchHit>>;
    fn index(&self, store: &Store) -> AppResult<String>;
    /// Content is temporary and must not be charged as persisted mail data.
    fn index_with_content(&self, store: &Store, _content: &[Email]) -> AppResult<String> {
        self.index(store)
    }
    fn installed(&self) -> bool;
    /// There is no server to make the vectors (the gateway says `compute_not_connected`): the
    /// account shows `needs_server` and not «waiting for the model».
    fn needs_server(&self) -> bool {
        false
    }
    /// Whether vectors can be made now: `Ok(false)` means words only for now. An error stops
    /// the account's indexing without touching any message: `compute_unauthorized` (the server
    /// no longer knows this device, A8) or `model_mismatch` (D4).
    fn semantic_readiness(&self) -> AppResult<bool> {
        Ok(self.installed())
    }
    /// Removes every vector, so that the messages are embedded again (D4 «reindex»).
    fn reset_vectors(&self) -> AppResult<()> {
        Ok(())
    }
    fn install_status(&self) -> ModelInstallStatus {
        ModelInstallStatus { installed: self.installed(), ..Default::default() }
    }
}

/// Search model download, as shown in Settings while it installs in the background.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct ModelInstallStatus {
    pub installed: bool,
    pub running: bool,
    pub done_bytes: u64,
    pub total_bytes: u64,
    pub error: Option<String>,
}

pub trait AssistantGateway: Send + Sync {
    fn summarize(&self, settings: &Settings, kind: &str, text: &str) -> AppResult<String>;
    fn reply(&self, settings: &Settings, email: &Email, instruction: &str) -> AppResult<String>;
    fn ask(&self, settings: &Settings, question: &str, context: &str) -> AppResult<String>;
    fn classify(
        &self,
        settings: &Settings,
        email: &Email,
        labels: &[String],
    ) -> AppResult<(Category, Vec<String>)>;
}

pub trait GoogleIdentity: Send + Sync {
    fn client_credentials(&self, client_id: &str, client_secret: &str) -> AppResult<(String, String)> {
        if client_id.trim().is_empty() {
            return Err(AppError::new("google_not_configured", GOOGLE_NOT_CONFIGURED));
        }
        Ok((client_id.trim().into(), client_secret.trim().into()))
    }
    fn sign_in(&self, client_id: &str, client_secret: &str, login_hint: &str) -> AppResult<(String, String)>;
    /// Ends a sign-in that is waiting for the browser; does nothing when none is.
    fn cancel_sign_in(&self);
}

pub const AI_KEY: &str = "ai-key";
pub const GOOGLE_SECRET: &str = "google-client-secret";
pub const GOOGLE_NOT_CONFIGURED: &str = "Google sign-in is not configured in this version of Qamille. Contact the app developer.";
pub fn account_key(email: &str) -> String {
    format!("gmail:{}", email.to_lowercase())
}
pub fn provider_account_key(provider:&str,account_id:u32)->String {format!("provider:{provider}:{account_id}")}

pub use crate::domain::search_remote::RemoteScope;

/// Where a provider search stopped: the folder, its generation and the lowest
/// UID already shown. A changed generation starts again from the first page.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RemoteCursor {
    pub folder: String,
    pub uid_validity: u32,
    pub before_uid: u32,
}

#[derive(Clone, Debug, Default)]
pub struct RemoteSearchRequest {
    /// The provider's search language (see `domain::search_remote`).
    pub raw: String,
    /// Folders searched after All Mail.
    pub scopes: Vec<RemoteScope>,
    pub cursor: Option<RemoteCursor>,
    pub page_size: usize,
}

#[derive(Clone, Debug, Default)]
pub struct RemoteSearchPage {
    /// Metadata-only rows (`remote_only`), newest first.
    pub emails: Vec<Email>,
    /// Matches on the server, across every searched folder.
    pub total: u32,
    /// Messages in All Mail, for the coverage notice.
    pub mailbox_total: u32,
    pub next: Option<RemoteCursor>,
}

/// What `upsert_remote_metadata` did, in the order of the rows given.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UpsertOutcome {
    pub inserted: Vec<u64>,
    pub enriched: Vec<u64>,
}

#[derive(Clone, Debug, Default)]
pub struct HistoryRequest {
    /// The walk so far; the gateway plans the next range from it once it knows
    /// the folder's generation and next UID.
    pub progress: crate::domain::history::HistoryProgress,
    pub batch_uids: u32,
    /// Further folders read with this batch, newest first: "Spam", "Trash".
    pub roles: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct HistoryBatch {
    pub folder: String,
    pub uid_validity: u32,
    pub uid_next: u32,
    pub exists: u32,
    /// The range read, planned on `progress` (restarted on a new generation);
    /// `None`: nothing left to read.
    pub range: Option<crate::domain::history::UidRange>,
    /// Metadata-only rows (`remote_only`), placed by their Gmail labels.
    pub emails: Vec<Email>,
    /// With nothing left to read: the Gmail state of the most recent messages,
    /// so labels changed on the web reach the list (W3.2).
    pub refresh: Vec<LabelState>,
    /// The newest messages of the `roles` asked, placed in their folder.
    pub extra: Vec<Email>,
    /// The roles actually read (a role Gmail does not expose is left out).
    pub roles_read: Vec<String>,
    /// Messages marked read on Gmail for `progress.mark_read_before`.
    pub marked_read: u32,
}

/// Flags and labels of one message of All Mail, by its Gmail identity (W3.2).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LabelState {
    pub global_id: u64,
    pub labels: Vec<String>,
    pub read: bool,
    pub starred: bool,
    pub replied: bool,
}
