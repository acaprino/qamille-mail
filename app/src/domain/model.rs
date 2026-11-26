//! Data kept by Qamille: accounts, mail, drafts, calendar, meeting notes and settings.
//! Everything lives in one JSON file so the app and the MCP server can share it.

use chrono::{Datelike, Duration, Local, TimeZone, Timelike};
use serde::{Deserialize, Serialize};

pub fn now_ts() -> i64 {
    Local::now().timestamp()
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Category {
    #[default]
    Personal,
    Priority,
    Notification,
    Newsletter,
    Invitation,
    InvitationResponse,
}

impl Category {
    pub const ALL: [Category; 6] = [
        Category::Personal,
        Category::Priority,
        Category::Notification,
        Category::Newsletter,
        Category::Invitation,
        Category::InvitationResponse,
    ];
    pub fn key(self) -> &'static str {
        match self {
            Category::Personal => "personal",
            Category::Priority => "priority",
            Category::Notification => "notification",
            Category::Newsletter => "newsletter",
            Category::Invitation => "invitation",
            Category::InvitationResponse => "invitation_response",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Category::Personal => "Personal",
            Category::Priority => "Priority",
            Category::Notification => "Notification",
            Category::Newsletter => "Newsletter",
            Category::Invitation => "Invitation",
            Category::InvitationResponse => "Invitation reply",
        }
    }
    pub fn from_key(s: &str) -> Option<Category> {
        Category::ALL.iter().copied().find(|c| c.key() == s.to_lowercase())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Attachment {
    pub id: u64,
    pub name: String,
    pub size: u64,
    pub mime: String,
    /// File saved on disk (real mail).
    pub path: Option<String>,
    /// Inline text for sample mail.
    pub inline_text: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct Email {
    #[serde(default)]
    pub search_index: super::indexing::MessageIndexState,
    pub id: u64,
    pub account_id: u32,
    pub folder: String,
    pub thread_id: String,
    pub message_id: String,
    /// Locally submitted copy awaiting an unambiguous, content-verified Sent echo.
    #[serde(default)]
    pub local_sent: bool,
    /// Complete provider membership reconciliation confirmed permanent absence.
    /// Cached content and local task state are retained as a local Trash record.
    #[serde(default)]
    pub remote_deleted: bool,
    /// Hash of the submitted semantic content (including attachment bytes).
    /// Provider adapters only set this after verifying an outgoing echo.
    #[serde(default)]
    pub outgoing_fingerprint: String,
    pub uid: Option<u32>,
    /// IMAP UID namespace for messages loaded outside INBOX.
    #[serde(default)]
    pub remote_folder: Option<String>,
    #[serde(default)]
    pub remote_uid_validity: Option<u32>,
    /// Hash of the complete MIME, used only to verify identity across Gmail labels.
    #[serde(default)]
    pub remote_content_hash: String,
    #[serde(default)]
    pub remote_global_id: Option<u64>,
    /// Other locations verified to contain the same complete MIME.
    #[serde(default)]
    pub remote_locations: Vec<RemoteLocation>,
    pub from_name: String,
    pub from_addr: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reply_to_addresses: Vec<String>,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub subject: String,
    pub body: String,
    /// Original HTML alternative, kept only in memory for the reader.
    /// The offline repository strips it before writing.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body_html: String,
    /// Headers and local task state remain when offline content is evicted.
    #[serde(default)]
    pub remote_only: bool,
    /// A durable task reference whose replaceable data was evicted for quota.
    #[serde(default)]
    pub storage_evicted: bool,
    pub date: i64,
    /// The source message had no usable date. Older snapshots default to false:
    /// their historical fallback timestamps cannot be distinguished reliably.
    #[serde(default)]
    pub date_missing: bool,
    /// When the provider received the message (IMAP INTERNALDATE), set by a sync download only. It
    /// decides whether new mail is an arrival (`MailService::sync_selected`), which the Date header
    /// cannot: a delayed delivery keeps the date it was written. Never stored.
    #[serde(skip)]
    pub received_at: Option<i64>,
    pub read: bool,
    pub starred: bool,
    pub pinned: bool,
    pub done: bool,
    pub muted: bool,
    pub set_aside: bool,
    pub replied: bool,
    pub priority_thread: bool,
    pub category: Category,
    pub labels: Vec<String>,
    pub snoozed_until: Option<i64>,
    pub reminder_at: Option<i64>,
    pub due: Option<i64>,
    pub assigned_to: Option<String>,
    pub attachments: Vec<Attachment>,
    #[serde(default)]
    pub classified: bool,
    /// When a snoozed mail came back: it then sorts as of that moment (inboxOrSnoozeDate, 0x181cdf410).
    #[serde(default)]
    pub returned_at: Option<i64>,
    /// Carries List-Unsubscribe or Precedence: bulk/list.
    #[serde(default)]
    pub bulk: bool,
    /// Auto-Submitted header says a program wrote it.
    #[serde(default)]
    pub auto_submitted: bool,
    /// Carries a calendar part (.ics or text/calendar).
    #[serde(default)]
    pub calendar: bool,
    /// Message-ID of the direct parent, without the angle brackets; empty when unknown
    /// (ADR-0010). `thread_id` keeps its meaning: it is not always the direct parent.
    #[serde(default)]
    pub in_reply_to: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct RemoteLocation {
    pub folder: String,
    pub uid: u32,
    pub uid_validity: Option<u32>,
}

impl Email {
    pub fn reply_recipients(&self) -> &[String] {
        if self.reply_to_addresses.is_empty() { std::slice::from_ref(&self.from_addr) } else { &self.reply_to_addresses }
    }
    /// A provider copy that replaces a local one keeps the parent the local copy recorded
    /// when the provider did not supply its own (ADR-0010: a sent reply's echo).
    pub fn keep_parent_of(&mut self, previous: &Email) {
        if self.in_reply_to.is_empty() {
            self.in_reply_to = previous.in_reply_to.clone();
        }
    }
    pub fn snippet(&self, n: usize) -> String {
        let flat: String = self.body.split_whitespace().collect::<Vec<_>>().join(" ");
        if flat.chars().count() > n {
            format!("{}…", flat.chars().take(n).collect::<String>())
        } else {
            flat
        }
    }
    pub fn is_task(&self) -> bool {
        !self.done
            && self.folder != "Trash"
            && (self.pinned || self.due.is_some() || self.reminder_at.is_some() || self.set_aside)
    }
    pub fn inbox_date(&self) -> i64 {
        self.returned_at.unwrap_or(self.date)
    }
    pub fn is_snoozed(&self, now: i64) -> bool {
        self.snoozed_until.map(|t| t > now).unwrap_or(false)
    }
}

/// How the editor holds the message: plain text, the formatted editor's text, or raw HTML.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum BodyFormat {
    #[default]
    Plain,
    Rich,
    Html,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Signature {
    pub id: u64,
    pub name: String,
    /// In the formatted editor's text.
    pub body: String,
    #[serde(default)]
    pub modified: i64,
    /// True once saved from the visual editor; older ones hold the previous markup.
    #[serde(default)]
    pub html: bool,
}

impl Signature {
    pub fn html_body(&self) -> String {
        if self.html { self.body.clone() } else { crate::richtext::markup_to_html(&self.body) }
    }
    pub fn plain_body(&self) -> String {
        if self.html { crate::richdoc::RichDoc::from_html(&self.body).text } else { crate::richtext::markup_to_plain(&self.body) }
    }
}

/// Which signature a mailbox uses by default ("Most Recent" and "No Signature" in the original).
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SignatureBinding {
    #[default]
    MostRecent,
    None,
    Id(u64),
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Draft {
    pub id: u64,
    pub account_id: u32,
    pub to: String,
    pub cc: String,
    #[serde(default)]
    pub bcc: String,
    #[serde(default)]
    pub request_id: String,
    #[serde(default)]
    pub remote_id: Option<String>,
    #[serde(default)]
    pub remote_revision: Option<String>,
    #[serde(default)]
    pub cloud_content_hash: String,
    #[serde(default)]
    pub cloud_sync_key: String,
    #[serde(default)]
    pub cloud_sync_pending: bool,
    pub subject: String,
    pub body: String,
    pub reply_to: Option<u64>,
    pub send_at: Option<i64>,
    pub attachments: Vec<String>,
    pub updated: i64,
    #[serde(default)]
    pub format: BodyFormat,
    #[serde(default)]
    pub signature_id: Option<u64>,
    /// Quoted reply or forwarded text, kept apart so the signature goes between it and the message.
    #[serde(default)]
    pub quote: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Rsvp {
    #[default]
    None,
    Accepted,
    Maybe,
    Declined,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Event {
    pub id: u64,
    pub title: String,
    pub start: i64,
    pub end: i64,
    pub location: String,
    pub attendees: Vec<String>,
    pub meeting_link: Option<String>,
    pub conference: String,
    pub rsvp: Rsvp,
    pub notes: String,
    pub color: [u8; 3],
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Meeting {
    pub id: u64,
    pub title: String,
    pub date: i64,
    pub duration_secs: u64,
    pub audio_path: Option<String>,
    pub transcript: String,
    pub notes: String,
    pub summary: String,
    pub key_points: Vec<String>,
    pub action_items: Vec<String>,
    pub event_id: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum AccountKind {
    #[default]
    Sample,
    #[serde(alias = "Imap")]
    Gmail,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct Account {
    /// Stable adapter identity. Empty legacy values resolve from `kind`.
    #[serde(default)]
    pub provider_id: String,
    #[serde(default)]
    pub search_index: super::indexing::IndexProgress,
    /// The history backfill of a Gmail account (W3.1).
    #[serde(default)]
    pub history: super::history::HistoryProgress,
    pub id: u32,
    pub name: String,
    pub email: String,
    pub color: [u8; 3],
    pub kind: AccountKind,
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub username: String,
    /// Refresh token (OAuth) or app password. Kept in the portable credential vault, never in the mail data file.
    #[serde(skip)]
    pub password: String,
    /// Empty means an app password account.
    #[serde(default)]
    pub oauth_client_id: String,
    #[serde(default)]
    pub signature: SignatureBinding,
    #[serde(default)]
    pub last_signature: Option<u64>,
    pub archive_folder: String,
    pub trash_folder: String,
    pub last_sync: Option<i64>,
    pub fetch_count: u32,
    /// Existing archives keep their contents until their owner chooses a limit.
    #[serde(default = "super::storage::AccountStoragePolicy::legacy")]
    pub storage_policy: super::storage::AccountStoragePolicy,
    /// Set only after the provider has verified the entire offline scope.
    #[serde(default)]
    pub offline_sync_complete: bool,
    #[serde(default)]
    pub offline_sync_error: Option<String>,
}

/// `password` holds the refresh token or the app password: `Debug` never shows it.
impl std::fmt::Debug for Account {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Account")
            .field("provider_id", &self.provider_id)
            .field("id", &self.id)
            .field("name", &self.name)
            .field("email", &self.email)
            .field("color", &self.color)
            .field("kind", &self.kind)
            .field("imap_host", &self.imap_host)
            .field("imap_port", &self.imap_port)
            .field("smtp_host", &self.smtp_host)
            .field("smtp_port", &self.smtp_port)
            .field("username", &self.username)
            .field("password", &if self.password.is_empty() { "" } else { "<hidden>" })
            .field("oauth_client_id", &self.oauth_client_id)
            .field("signature", &self.signature)
            .field("last_signature", &self.last_signature)
            .field("archive_folder", &self.archive_folder)
            .field("trash_folder", &self.trash_folder)
            .field("last_sync", &self.last_sync)
            .field("fetch_count", &self.fetch_count)
            .field("storage_policy", &self.storage_policy)
            .field("offline_sync_complete", &self.offline_sync_complete)
            .field("offline_sync_error", &self.offline_sync_error)
            .field("search_index", &self.search_index)
            .field("history", &self.history)
            .finish()
    }
}

impl Account {
    pub fn resolved_provider_id(&self)->&str {
        if self.provider_id.trim().is_empty(){match self.kind{AccountKind::Sample=>"sample",AccountKind::Gmail=>"gmail"}}else{self.provider_id.trim()}
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum AiProvider {
    #[default]
    Offline,
    Anthropic,
    OpenAiCompatible,
    Ollama,
    /// The assistant of the connected Qamille server (gateway LLM, wave 5): no key of its own.
    QamilleServer,
}

/// What the person agreed to for the assistant of the server: the provider the server declared
/// in `GET /v1/info` at that address. Sent in every request (`X-Qamille-Llm-Consent`).
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct LlmConsent {
    pub url: String,
    pub label: String,
    pub external: bool,
}

impl LlmConsent {
    /// The header value the gateway compares byte by byte: `<external>;<label>`.
    pub fn header(&self) -> String {
        format!("{};{}", self.external, self.label)
    }
}

impl AiProvider {
    pub fn label(self) -> &'static str {
        match self {
            AiProvider::Offline => "Built-in rules (no internet)",
            AiProvider::Anthropic => "Claude (Anthropic)",
            AiProvider::OpenAiCompatible => "OpenAI-compatible",
            AiProvider::Ollama => "Ollama (local)",
            AiProvider::QamilleServer => "Qamille server",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Access {
    ReadOnly,
    #[default]
    Triage,
    Send,
}

impl Access {
    pub fn label(self) -> &'static str {
        match self {
            Access::ReadOnly => "read-only",
            Access::Triage => "triage",
            Access::Send => "send",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Settings {
    /// IANA identifier, or empty to follow the operating system.
    #[serde(default)]
    pub time_zone: String,
    /// BCP 47 regional locale, or empty to follow the operating system.
    #[serde(default)]
    pub locale: String,
    #[serde(default = "default_undo_seconds")]
    pub undo_seconds: u32,
    pub user_name: String,
    pub signature: String,
    pub ai_provider: AiProvider,
    pub ai_base_url: String,
    /// Kept in the portable credential vault, never in the mail data file.
    #[serde(skip)]
    pub ai_key: String,
    pub ai_model: String,
    pub auto_classify: bool,
    pub auto_tag: bool,
    pub mcp_access: Access,
    pub dark: bool,
    /// Also apply AI labels to the mail in Gmail. Off unless the person turns it on.
    #[serde(default)]
    pub push_labels: bool,
    /// The person's own Google project, for OAuth sign-in.
    #[serde(default)]
    pub google_client_id: String,
    /// Kept in the portable credential vault, never in the mail data file.
    #[serde(skip)]
    pub google_client_secret: String,
    /// Show a join chip before online meetings. On for existing workspaces too.
    #[serde(default = "default_true")]
    pub meeting_reminders: bool,
    /// Minutes before the start: one of `reminders::LEAD_MINUTES`.
    #[serde(default = "default_reminder_lead_minutes")]
    pub reminder_lead_minutes: u32,
    /// Spoken language of the recordings: a language code such as `it`, or `auto`.
    #[serde(default = "default_meeting_language")]
    pub meeting_language: String,
    /// `compute` (whisper.cpp in the compute server of Settings > Compute) or `external` (the
    /// OpenAI-compatible endpoint of the assistant). Files with the old `local`/`remote` read as
    /// these (`meeting::canonical_engine`).
    #[serde(default = "default_meeting_engine", deserialize_with = "super::meeting::deserialize_engine")]
    pub meeting_engine: String,
    /// Id of the local model: one of `meeting::LOCAL_SPEECH_MODELS`.
    #[serde(default = "default_meeting_model")]
    pub meeting_model: String,
    /// Model name sent to the remote endpoint: letters, digits and `-_./:` up to 200 characters
    /// (`meeting::valid_remote_model_name`). Empty means `whisper-1` (see speech.rs `remote_model`).
    #[serde(default)]
    pub meeting_remote_model: String,
    /// Capture device id for the microphone; empty follows the system default.
    #[serde(default)]
    pub meeting_microphone: String,
    /// Render device id for the system audio loopback; empty follows the system default.
    #[serde(default)]
    pub meeting_system_audio: String,
    /// Where the computation runs: on this computer, or on the server of the session (spec A1).
    #[serde(default)]
    pub compute_mode: ComputeMode,
    /// A PEM certificate authority trusted for the remote server, besides the system ones.
    #[serde(default)]
    pub compute_ca_pem: Option<String>,
    /// The server address `compute_ca_pem` belongs to: the authority is used only for it, so a
    /// new session never meets the authority of the old one (security review of compute wave 3).
    #[serde(default)]
    pub compute_ca_url: Option<String>,
    /// Server addresses the person agreed to send content to (INV-10).
    #[serde(default)]
    pub compute_consented_urls: Vec<String>,
    /// The provider of the server's assistant the person agreed to (gateway LLM, L5).
    #[serde(default)]
    pub llm_consent: Option<LlmConsent>,
    /// The first start of a Professional build is behind the person (spec A8).
    #[serde(default)]
    pub onboarding_done: bool,
    /// The person dismissed the notice about the old local compute mode (wave 2).
    #[serde(default)]
    pub legacy_local_notice_dismissed: bool,
    /// Mailboxes sync between this person's computers through the connected server (wave 7).
    /// Written only by the sync commands, like the consent of the compute server.
    #[serde(default)]
    pub mailbox_sync: bool,
    /// The server address the person agreed to store the mailboxes' credentials on (INV-35).
    #[serde(default)]
    pub mailbox_sync_consent: Option<String>,
    /// Allocated once per workspace, including reservations for interrupted syncs.
    #[serde(default)]
    pub mailbox_id_high_water: Option<u32>,
}

/// Where indexing, semantic search and transcription compute (spec A1, D7, INV-41): on a
/// server, or nowhere. Files saved as `local` or `remote` still load (`Off`, `Server`).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ComputeMode {
    #[default]
    #[serde(alias = "local")]
    Off,
    #[serde(alias = "remote")]
    Server,
}

impl Settings {
    /// How a meeting is transcribed unless the person picks otherwise for one run.
    pub fn meeting_options(&self) -> super::meeting::TranscribeOptions {
        let engine = super::meeting::canonical_engine(&self.meeting_engine);
        let remote = engine == super::meeting::ENGINE_EXTERNAL;
        super::meeting::TranscribeOptions {
            provider: engine.to_owned(),
            model: if remote { self.meeting_remote_model.clone() } else { self.meeting_model.clone() },
            language: self.meeting_language.clone(),
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            time_zone: String::new(),
            locale: String::new(),
            undo_seconds: default_undo_seconds(),
            user_name: "You".into(),
            signature: String::new(),
            ai_provider: AiProvider::Offline,
            ai_base_url: String::new(),
            ai_key: String::new(),
            ai_model: String::new(),
            auto_classify: true,
            auto_tag: true,
            mcp_access: Access::Triage,
            dark: false,
            push_labels: false,
            google_client_id: String::new(),
            google_client_secret: String::new(),
            meeting_reminders: true,
            reminder_lead_minutes: default_reminder_lead_minutes(),
            meeting_language: default_meeting_language(),
            meeting_engine: default_meeting_engine(),
            meeting_model: default_meeting_model(),
            meeting_remote_model: String::new(),
            meeting_microphone: String::new(),
            meeting_system_audio: String::new(),
            compute_mode: ComputeMode::Off,
            compute_ca_pem: None,
            compute_ca_url: None,
            compute_consented_urls: Vec::new(),
            llm_consent: None,
            onboarding_done: false,
            legacy_local_notice_dismissed: false,
            mailbox_sync: false,
            mailbox_sync_consent: None,
            mailbox_id_high_water: Some(0),
        }
    }
}

pub fn default_undo_seconds() -> u32 { 5 }
fn default_true() -> bool { true }
pub fn default_reminder_lead_minutes() -> u32 { crate::domain::reminders::DEFAULT_LEAD_MINUTES }
fn default_meeting_language() -> String { "it".into() }
fn default_meeting_engine() -> String { super::meeting::ENGINE_COMPUTE.into() }
fn default_meeting_model() -> String { "base".into() }

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Store {
    #[serde(default)]
    pub outbox: Vec<super::outbox::Delivery>,
    #[serde(default)]
    pub cache_removals: Vec<String>,
    #[serde(default)]
    pub storage_maintenance_pending: bool,
    #[serde(default)]
    pub storage_index_rebuild_required: bool,
    #[serde(default)]
    pub undo_actions: Vec<super::undo::UndoAction>,
    pub accounts: Vec<Account>,
    pub emails: Vec<Email>,
    pub drafts: Vec<Draft>,
    pub events: Vec<Event>,
    pub meetings: Vec<Meeting>,
    pub settings: Settings,
    pub next_id: u64,
    #[serde(default)]
    pub signatures: Vec<Signature>,
}

impl Store {
    fn mailbox_id_floor(&self) -> u32 {
        // Old versions issued small sequential IDs and never retained a counter. Leave their
        // entire lower namespace alone, including historical grants outside the mail store.
        let floor = self.settings.mailbox_id_high_water.unwrap_or(i32::MAX as u32);
        self.accounts.iter().map(|a| a.id)
            .chain(self.outbox.iter().map(|delivery| delivery.draft.account_id))
            .chain(self.drafts.iter().map(|draft| draft.account_id))
            .fold(floor, u32::max)
    }

    pub fn allocate_mailbox_id(&mut self) -> Option<u32> {
        let id = self.mailbox_id_floor().checked_add(1)?;
        self.settings.mailbox_id_high_water = Some(id);
        Some(id)
    }

    pub fn detach_mailbox(&mut self, id: u32, now: i64) {
        use super::{outbox::DeliveryStatus, undo::UndoOperation};
        // Record the number before dropping its last reference. In-flight deliveries retain
        // their uncertain status for reconciliation; pending ones can no longer be sent.
        self.settings.mailbox_id_high_water = Some(self.mailbox_id_floor().max(id));
        for delivery in self.outbox.iter_mut().filter(|d| d.draft.account_id == id) {
            if matches!(delivery.status, DeliveryStatus::Queued | DeliveryStatus::Scheduled | DeliveryStatus::Failed) {
                delivery.status = DeliveryStatus::Cancelled;
                delivery.updated_at = now;
                delivery.error = Some("Mailbox removed".into());
            }
        }
        let email_ids: std::collections::HashSet<_> = self.emails.iter().filter(|e| e.account_id == id).map(|e| e.id).collect();
        let draft_ids: std::collections::HashSet<_> = self.drafts.iter().filter(|d| d.account_id == id).map(|d| d.id).collect();
        self.undo_actions.retain(|action| match &action.operation {
            UndoOperation::Mail { email_id, .. } => !email_ids.contains(email_id),
            other => !other.draft_id().is_some_and(|draft| draft_ids.contains(&draft)),
        });
        self.accounts.retain(|a| a.id != id);
        self.emails.retain(|e| e.account_id != id);
        self.drafts.retain(|d| d.account_id != id);
    }

    /// An older single signature becomes the first entry of the manager.
    pub(crate) fn adopt_old_signature(&mut self) {
        if self.settings.signature.trim().is_empty() || !self.signatures.is_empty() {
            return;
        }
        let id = self.new_id();
        let body = std::mem::take(&mut self.settings.signature);
        self.signatures.push(Signature { id, name: "Signature".into(), body, modified: now_ts(), html: false });
    }
    pub fn signature(&self, id: u64) -> Option<&Signature> {
        self.signatures.iter().find(|s| s.id == id)
    }
    /// The signature a new message from this mailbox starts with.
    pub fn signature_for_new(&self, account_id: u32) -> Option<u64> {
        let a = self.account(account_id)?;
        match a.signature {
            SignatureBinding::None => None,
            SignatureBinding::Id(id) => self.signature(id).map(|s| s.id),
            SignatureBinding::MostRecent => a
                .last_signature
                .and_then(|id| self.signature(id))
                .or_else(|| self.signatures.iter().max_by_key(|s| s.modified))
                .map(|s| s.id),
        }
    }
    /// For a reply: the signature used earlier in this conversation, else the mailbox default.
    /// It searches your other messages in the thread; the longest known signature found wins
    /// (SignatureForReplyQuery 0x1803c8890, helper 0x1803c93b0).
    pub fn signature_for_reply(&self, e: &Email) -> Option<u64> {
        let flat = |t: &str| t.split_whitespace().collect::<Vec<_>>().join(" ");
        let own = self.account(e.account_id).map(|a| a.email.to_lowercase()).unwrap_or_default();
        let mine: Vec<String> = self
            .emails
            .iter()
            .filter(|m| m.id != e.id && m.account_id == e.account_id && !m.thread_id.is_empty() && m.thread_id == e.thread_id)
            .filter(|m| m.folder == "Sent" || m.from_addr.to_lowercase() == own)
            .map(|m| flat(&m.body))
            .collect();
        for body in &mine {
            let best = self
                .signatures
                .iter()
                .map(|s| (s.id, flat(&s.plain_body())))
                .filter(|(_, text)| !text.is_empty() && body.contains(text.as_str()))
                .max_by_key(|(_, text)| text.chars().count());
            if let Some((id, _)) = best {
                return Some(id);
            }
        }
        self.signature_for_new(e.account_id)
    }
    pub fn remove_signature(&mut self, id: u64) {
        self.signatures.retain(|s| s.id != id);
        for a in &mut self.accounts {
            if a.signature == SignatureBinding::Id(id) {
                a.signature = SignatureBinding::MostRecent;
            }
            if a.last_signature == Some(id) {
                a.last_signature = None;
            }
        }
        for d in &mut self.drafts {
            if d.signature_id == Some(id) {
                d.signature_id = None;
            }
        }
    }
    pub fn new_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }
    pub fn account(&self, id: u32) -> Option<&Account> {
        self.accounts.iter().find(|a| a.id == id)
    }
    pub fn email(&self, id: u64) -> Option<&Email> {
        self.emails.iter().find(|e| e.id == id)
    }
    pub fn email_mut(&mut self, id: u64) -> Option<&mut Email> {
        self.emails.iter_mut().find(|e| e.id == id)
    }
    pub fn all_labels(&self) -> Vec<String> {
        let unique: std::collections::BTreeSet<&str> =
            self.emails.iter().flat_map(|e| e.labels.iter().map(String::as_str)).collect();
        unique.into_iter().map(str::to_string).collect()
    }
    /// Snoozed mail whose time has come returns to the Inbox.
    pub fn wake_snoozed(&mut self, now: i64) -> usize {
        let mut n = 0;
        for e in &mut self.emails {
            if let Some(t) = e.snoozed_until {
                if t <= now {
                    e.snoozed_until = None;
                    e.returned_at = Some(t);
                    e.read = false;
                    e.folder = "Inbox".into();
                    n += 1;
                }
            }
        }
        n
    }
}

fn at(days: i64, hour: u32, min: u32) -> i64 {
    let d = Local::now() + Duration::days(days);
    Local
        .with_ymd_and_hms(d.year(), d.month(), d.day(), hour, min, 0)
        .single()
        .map(|t| t.timestamp())
        .unwrap_or_else(now_ts)
}

fn ago_minutes(m: i64) -> i64 {
    now_ts() - m * 60
}

#[allow(clippy::too_many_arguments)]
fn mail(
    s: &mut Store,
    acc: u32,
    from: (&str, &str),
    subject: &str,
    body: &str,
    mins_ago: i64,
    cat: Category,
    read: bool,
) -> u64 {
    let id = s.new_id();
    s.emails.push(Email {
        id,
        account_id: acc,
        folder: "Inbox".into(),
        thread_id: format!("t{id}"),
        message_id: format!("<sample-{id}@qamille.local>"),
        from_name: from.0.into(),
        from_addr: from.1.into(),
        to: vec![s.account(acc).map(|a| a.email.clone()).unwrap_or_default()],
        subject: subject.into(),
        body: body.into(),
        date: ago_minutes(mins_ago),
        read,
        category: cat,
        classified: true,
        ..Default::default()
    });
    id
}

pub fn sample_store() -> Store {
    let mut s = Store {
        settings: Settings { user_name: "Alex".into(), ..Default::default() },
        ..Default::default()
    };
    s.accounts.push(Account {
        id: 1,
        name: "Work".into(),
        email: "alex@studio-example.com".into(),
        color: [47, 111, 237],
        kind: AccountKind::Sample,
        fetch_count: 50,
        ..Default::default()
    });
    s.accounts.push(Account {
        id: 2,
        name: "Personal".into(),
        email: "alex.home@example.org".into(),
        color: [236, 112, 99],
        kind: AccountKind::Sample,
        fetch_count: 50,
        ..Default::default()
    });

    s.signatures.push(Signature { id: s.next_id + 1, name: "Work".into(), body: "**Alex Example**\nStudio Example · [studio-example.com](https://studio-example.com)".into(), modified: 1, html: false });
    s.signatures.push(Signature { id: s.next_id + 2, name: "Personal".into(), body: "Alex".into(), modified: 0, html: false });
    s.next_id += 2;
    let (work_sig, home_sig) = (s.signatures[0].id, s.signatures[1].id);
    s.accounts[0].signature = SignatureBinding::Id(work_sig);
    s.accounts[1].signature = SignatureBinding::Id(home_sig);

    let a = mail(&mut s, 1, ("Maria Rossi", "maria@client-example.com"),
        "Q4 proposal: can you review by Thursday?",
        "Hi Alex,\n\nI attached the Q4 proposal draft. Could you review the budget section and send me your comments by Thursday? We also need to decide the launch date before the board meeting.\n\nThanks!\nMaria",
        12, Category::Priority, false);
    if let Some(e) = s.email_mut(a) {
        e.labels = vec!["Projects".into()];
        e.attachments.push(Attachment {
            id: 9001,
            name: "Q4-proposal.txt".into(),
            size: 2_400,
            mime: "text/plain".into(),
            path: None,
            inline_text: Some("Q4 PROPOSAL\n\nBudget: 48,000 EUR (design 18k, development 22k, marketing 8k).\nTimeline: kickoff 3 Nov, beta 1 Dec, launch 15 Jan.\nRisks: hiring of one front-end developer; holiday slowdown.\nDecision needed: launch date and marketing split.".into()),
        });
        e.due = Some(at(3, 17, 0));
    }
    let b = mail(&mut s, 1, ("Luca Bianchi", "luca@studio-example.com"),
        "Design review moved to 3 pm",
        "Hey, quick heads-up: the design review is now at 3 pm in the small room. I'll share the Figma link before.\n\nLuca",
        45, Category::Personal, false);
    if let Some(e) = s.email_mut(b) {
        e.labels = vec!["Team".into()];
    }
    let c = mail(&mut s, 2, ("Sofia", "sofia@example.net"),
        "Dinner on Saturday?",
        "Ciao! Are you free for dinner on Saturday around 8? Thinking of the new place near the station. Let me know :)",
        90, Category::Personal, false);
    let _ = c;
    let d = mail(&mut s, 1, ("GitHub", "notifications@github-example.com"),
        "[qamille] New issue: Search crashes on empty query",
        "A new issue was opened by @dev-sam: Search crashes on empty query. Steps to reproduce: open search, press Enter with no text.",
        130, Category::Notification, false);
    if let Some(e) = s.email_mut(d) {
        e.labels = vec!["Dev".into()];
    }
    mail(&mut s, 1, ("Bank Alerts", "no-reply@bank-example.com"),
        "Your monthly statement is ready",
        "Your statement for October is now available in online banking. Card ending 4821. This is an automated message.",
        300, Category::Notification, true);
    let n = mail(&mut s, 2, ("The Weekly Brief", "news@brief-example.com"),
        "This week: 5 tools for focused work",
        "In this issue: deep work timers, inbox zero habits, calendar blocking, and more. Unsubscribe at any time.",
        400, Category::Newsletter, false);
    if let Some(e) = s.email_mut(n) {
        e.labels = vec!["Reading".into()];
    }
    mail(&mut s, 2, ("Design Digest", "hello@digest-example.com"),
        "Typography trends for 2027",
        "Variable fonts, softer grotesques and the return of serif headlines. Read the full digest online. Unsubscribe.",
        900, Category::Newsletter, true);
    let i = mail(&mut s, 1, ("Paolo Verdi", "paolo@partner-example.com"),
        "Invitation: Partnership kickoff @ Tue 10:00",
        "Paolo Verdi invited you to Partnership kickoff. When: Tuesday 10:00-11:00. Join online with the link in the event.",
        600, Category::Invitation, true);
    if let Some(e) = s.email_mut(i) {
        e.pinned = true;
    }
    let r = mail(&mut s, 1, ("Accounting", "invoices@vendor-example.com"),
        "Invoice #2291 for October",
        "Please find attached invoice #2291 (1,250.00 EUR) for October services. Payment due within 30 days.",
        1500, Category::Personal, true);
    if let Some(e) = s.email_mut(r) {
        e.labels = vec!["Finance".into()];
        e.reminder_at = Some(at(2, 9, 0));
        e.attachments.push(Attachment {
            id: 9002,
            name: "invoice-2291.txt".into(),
            size: 900,
            mime: "text/plain".into(),
            path: None,
            inline_text: Some("INVOICE 2291\nVendor Example srl\nServices October: 1,250.00 EUR\nIBAN IT00 X000 0000 0000 0000 0000 000\nDue: 30 days".into()),
        });
    }
    mail(&mut s, 2, ("Anna", "anna@example.net"),
        "Photos from the trip",
        "Here are the photos from the weekend in the mountains. The lake ones came out great!",
        2900, Category::Personal, true);
    let f = mail(&mut s, 1, ("Travel Desk", "booking@air-example.com"),
        "Your flight to Berlin is confirmed",
        "Booking ref K7Q2LM. Flight departs Monday 07:40 from Milan Linate, arrives 09:30 Berlin. Check-in opens 24h before.",
        3600, Category::Notification, true);
    if let Some(e) = s.email_mut(f) {
        e.labels = vec!["Travel".into()];
    }

    // A draft and a sent message.
    let did = s.new_id();
    s.drafts.push(Draft {
        id: did,
        account_id: 1,
        to: "luca@studio-example.com".into(),
        subject: "Notes for the design review".into(),
        body: "Hi Luca,\n\nHere are my notes:\n- simplify the onboarding\n- larger tap targets".into(),
        updated: now_ts(),
        ..Default::default()
    });

    // Calendar for this week.
    let eid = s.new_id();
    s.events.push(Event {
        id: eid,
        title: "Design review".into(),
        start: at(0, 15, 0),
        end: at(0, 16, 0),
        location: "Small room".into(),
        attendees: vec!["luca@studio-example.com".into()],
        meeting_link: Some("https://meet.jit.si/qamille-design-review".into()),
        conference: "jitsi".into(),
        rsvp: Rsvp::Accepted,
        color: [47, 111, 237],
        ..Default::default()
    });
    let eid2 = s.new_id();
    s.events.push(Event {
        id: eid2,
        title: "Partnership kickoff".into(),
        start: at(1, 10, 0),
        end: at(1, 11, 0),
        location: "Online".into(),
        attendees: vec!["paolo@partner-example.com".into()],
        meeting_link: Some("https://meet.jit.si/qamille-partnership-kickoff".into()),
        conference: "jitsi".into(),
        color: [124, 92, 230],
        ..Default::default()
    });
    let eid3 = s.new_id();
    s.events.push(Event {
        id: eid3,
        title: "Focus: Q4 proposal".into(),
        start: at(2, 9, 0),
        end: at(2, 11, 30),
        color: [46, 160, 110],
        ..Default::default()
    });
    let eid4 = s.new_id();
    s.events.push(Event {
        id: eid4,
        title: "Dinner with Sofia".into(),
        start: at(4, 20, 0),
        end: at(4, 22, 0),
        location: "Near the station".into(),
        color: [236, 112, 99],
        ..Default::default()
    });

    let mid = s.new_id();
    let now = Local::now();
    s.meetings.push(Meeting {
        id: mid,
        title: "Weekly sync".into(),
        date: now.timestamp() - 86_400 - (now.hour() as i64) * 60,
        duration_secs: 1_740,
        transcript: "Alex: Let's go through the launch plan.\nLuca: Designs are 80% done, onboarding still open.\nMaria: The client wants the proposal by Thursday.\nAlex: OK, I'll review the budget tomorrow morning.".into(),
        notes: "Remember to book the room for the review.".into(),
        summary: "Team aligned on the launch plan; proposal due Thursday.".into(),
        key_points: vec![
            "Designs are about 80% complete".into(),
            "Client expects the Q4 proposal by Thursday".into(),
        ],
        action_items: vec![
            "Alex: review the budget section".into(),
            "Luca: finish onboarding screens".into(),
        ],
        event_id: None,
        audio_path: None,
    });
    s
}

#[cfg(test)]
mod signature_tests {
    use super::*;

    fn store() -> Store {
        let mut s = Store::default();
        s.accounts.push(Account { id: 1, ..Default::default() });
        s.accounts.push(Account { id: 2, ..Default::default() });
        s.signatures.push(Signature { id: 10, name: "A".into(), body: "Saluti da Alfio".into(), modified: 5, html: false });
        s.signatures.push(Signature { id: 11, name: "B".into(), body: "**Team**".into(), modified: 9, html: false });
        s
    }

    #[test]
    fn each_mailbox_has_its_own_signature() {
        let mut s = store();
        s.accounts[0].signature = SignatureBinding::Id(10);
        s.accounts[1].signature = SignatureBinding::None;
        assert_eq!(s.signature_for_new(1), Some(10));
        assert_eq!(s.signature_for_new(2), None);
    }

    #[test]
    fn most_recent_prefers_the_last_used() {
        let mut s = store();
        assert_eq!(s.signature_for_new(1), Some(11), "newest when none was used yet");
        s.accounts[0].last_signature = Some(10);
        assert_eq!(s.signature_for_new(1), Some(10));
    }

    #[test]
    fn reply_keeps_the_signature_used_earlier_in_the_conversation() {
        let mut s = store();
        s.accounts[0].email = "me@gmail.com".into();
        s.accounts[0].signature = SignatureBinding::Id(11);
        s.signatures.push(Signature { id: 12, name: "Short".into(), body: "Saluti".into(), modified: 0, html: false });
        let incoming = Email { id: 1, account_id: 1, thread_id: "t".into(), body: "Grazie".into(), ..Default::default() };
        // Nothing of mine in this thread yet: the mailbox default.
        s.emails.push(incoming.clone());
        assert_eq!(s.signature_for_reply(&incoming), Some(11));
        // My earlier message carries two known signatures: the longest wins.
        s.emails.push(Email { id: 2, account_id: 1, thread_id: "t".into(), folder: "Sent".into(), from_addr: "me@gmail.com".into(), body: "Ok\n\nSaluti   da\nAlfio\n".into(), ..Default::default() });
        assert_eq!(s.signature_for_reply(&incoming), Some(10));
    }

    #[test]
    fn someone_elses_words_never_pick_a_signature() {
        let mut s = store();
        s.accounts[0].email = "me@gmail.com".into();
        s.accounts[0].signature = SignatureBinding::Id(11);
        s.signatures.push(Signature { id: 12, name: "Short".into(), body: "Alex".into(), modified: 0, html: false });
        let incoming = Email { id: 1, account_id: 1, thread_id: "t".into(), from_addr: "maria@x.org".into(), body: "Hi Alex".into(), ..Default::default() };
        s.emails.push(incoming.clone());
        s.emails.push(Email { id: 2, account_id: 1, thread_id: "t".into(), from_addr: "luca@x.org".into(), body: "Ciao Alex".into(), ..Default::default() });
        assert_eq!(s.signature_for_reply(&incoming), Some(11));
    }

    #[test]
    fn deleting_a_signature_cleans_every_link() {
        let mut s = store();
        s.accounts[0].signature = SignatureBinding::Id(10);
        s.accounts[0].last_signature = Some(10);
        s.drafts.push(Draft { signature_id: Some(10), ..Default::default() });
        s.remove_signature(10);
        assert_eq!(s.accounts[0].signature, SignatureBinding::MostRecent);
        assert_eq!(s.accounts[0].last_signature, None);
        assert_eq!(s.drafts[0].signature_id, None);
    }

    #[test]
    fn settings_without_mailbox_sync_load_as_off() {
        let old = r#"{"user_name":"u","signature":"","ai_provider":"Offline","ai_base_url":"","ai_model":"","auto_classify":true,"auto_tag":true,"mcp_access":"Triage","dark":false}"#;
        let s: Settings = serde_json::from_str(old).unwrap();
        assert!(!s.mailbox_sync);
        assert_eq!(s.mailbox_sync_consent, None);
    }

    #[test]
    fn settings_without_llm_consent_load_as_none() {
        let old = r#"{"user_name":"u","signature":"","ai_provider":"Offline","ai_base_url":"","ai_model":"","auto_classify":true,"auto_tag":true,"mcp_access":"Triage","dark":false}"#;
        let s: Settings = serde_json::from_str(old).unwrap();
        assert_eq!(s.llm_consent, None);
    }

    #[test]
    fn qamille_server_provider_round_trips() {
        assert_eq!(serde_json::to_string(&AiProvider::QamilleServer).unwrap(), "\"QamilleServer\"");
        assert!(!AiProvider::QamilleServer.label().is_empty());
        let consent = LlmConsent { url: "https://s".into(), label: "OpenAI".into(), external: true };
        assert_eq!(consent.header(), "true;OpenAI");
    }

    #[test]
    fn older_data_files_still_load() {
        let old = r#"{"accounts":[{"id":1,"name":"n","email":"e","color":[0,0,0],"kind":"Gmail","imap_host":"","imap_port":0,"smtp_host":"","smtp_port":0,"username":"","archive_folder":"","trash_folder":"","last_sync":null,"fetch_count":1}],"emails":[],"drafts":[{"id":1,"account_id":1,"to":"","cc":"","subject":"","body":"x","reply_to":null,"send_at":null,"attachments":[],"updated":0}],"events":[],"meetings":[],"settings":{"user_name":"u","signature":"Ciao","ai_provider":"Offline","ai_base_url":"","ai_model":"","auto_classify":true,"auto_tag":true,"mcp_access":"Triage","dark":false},"next_id":3}"#;
        let mut s: Store = serde_json::from_str(old).unwrap();
        s.adopt_old_signature();
        assert_eq!(s.signatures.len(), 1);
        assert_eq!(s.signatures[0].body, "Ciao");
        assert_eq!(s.accounts[0].signature, SignatureBinding::MostRecent);
        assert_eq!(s.drafts[0].format, BodyFormat::Plain);
    }
}

#[cfg(test)]
mod compute_settings_tests {
    use super::*;
    #[test]
    fn settings_saved_before_the_server_link_load_with_local_defaults() {
        let mut old = serde_json::to_value(Settings::default()).unwrap();
        for key in ["compute_mode", "compute_ca_pem", "compute_consented_urls", "onboarding_done"] {
            old.as_object_mut().unwrap().remove(key);
        }
        let loaded: Settings = serde_json::from_value(old).unwrap();
        assert_eq!(loaded.compute_mode, ComputeMode::Off);
        assert!(loaded.compute_ca_pem.is_none() && loaded.compute_consented_urls.is_empty() && !loaded.onboarding_done);
        assert_eq!(serde_json::to_value(ComputeMode::Server).unwrap(), "server");
    }
    #[test]
    fn settings_saved_with_local_or_remote_still_load() {
        for (saved, expected) in [("local", ComputeMode::Off), ("remote", ComputeMode::Server), ("off", ComputeMode::Off), ("server", ComputeMode::Server)] {
            assert_eq!(serde_json::from_value::<ComputeMode>(serde_json::json!(saved)).unwrap(), expected);
        }
        assert_eq!(serde_json::to_value(ComputeMode::Server).unwrap(), "server");
        assert_eq!(serde_json::to_value(ComputeMode::Off).unwrap(), "off");
        assert_eq!(Settings::default().compute_mode, ComputeMode::Off);
    }
}
