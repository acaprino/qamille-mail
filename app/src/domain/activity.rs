//! The activity log: what the app did, for the user or on their behalf, kept
//! locally so the sidebar panel and the Activity page can show it later. Only
//! concluded facts live here, never live state (ADR-0007 INV-009): progress of a
//! running job stays in the single live indicator.

/// What a row is: something the user (or the assistant for them) did, something
/// the app finished by itself, or a failure.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActivityKind {
    Action,
    #[default]
    Event,
    Error,
}

/// Who caused the row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActivitySource {
    User,
    Assistant,
    #[default]
    System,
}

/// How an action ended. `Done` is the value at the time of the action: the
/// undo window can still turn it into `Undone`, and a definitive commit failure
/// into `Failed`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActivityOutcome {
    #[default]
    Done,
    Undone,
    Failed,
}

/// The stored text of the three enums. Reading is lossy on purpose: a row
/// written by a newer release with a value this one does not know is shown as
/// a plain event instead of failing the whole page.
macro_rules! stored_text {
    ($name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        impl $name {
            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $text),+ }
            }
            pub fn parse(text: &str) -> Self {
                match text { $($text => Self::$variant,)+ _ => Self::default() }
            }
        }
    };
}
stored_text!(ActivityKind { Action => "action", Event => "event", Error => "error" });
stored_text!(ActivitySource { User => "user", Assistant => "assistant", System => "system" });
stored_text!(ActivityOutcome { Done => "done", Undone => "undone", Failed => "failed" });

/// Which rows the Activity page lists. `Error` also lists the actions that
/// failed, as the red tone and the unseen count do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActivityFilter {
    All,
    Action,
    Event,
    Error,
}

/// A row about to be written; the repository assigns `seq` and the caller
/// gives the time.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NewActivity {
    pub kind: ActivityKind,
    /// One of [`code`]; an unknown value from a newer release is shown generically.
    pub code: String,
    pub source: ActivitySource,
    /// `None` for what belongs to no account (the search model).
    pub account_id: Option<u32>,
    /// The message subject or draft title.
    pub subject: Option<String>,
    /// A short free text: the error message (English, translated by the UI).
    pub detail: Option<String>,
    /// How many messages or contacts: the UI words it, the backend never does.
    pub count: Option<u32>,
    pub ref_kind: Option<String>,
    pub ref_id: Option<u64>,
    /// The `UndoAction` id of a user action, so a later undo or failure finds
    /// its row, also after a restart.
    pub undo_id: Option<u64>,
    pub outcome: ActivityOutcome,
}

/// A stored row, as the UI receives it.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ActivityEntry {
    pub seq: i64,
    pub at_ms: i64,
    pub kind: ActivityKind,
    pub code: String,
    pub source: ActivitySource,
    pub account_id: Option<u32>,
    pub subject: Option<String>,
    pub detail: Option<String>,
    pub count: Option<u32>,
    pub ref_kind: Option<String>,
    pub ref_id: Option<u64>,
    pub undo_id: Option<u64>,
    pub outcome: ActivityOutcome,
}
impl ActivityEntry {
    pub fn new(seq: i64, at_ms: i64, entry: &NewActivity) -> Self {
        Self {
            seq,
            at_ms,
            kind: entry.kind,
            code: entry.code.clone(),
            source: entry.source,
            account_id: entry.account_id,
            subject: entry.subject.clone(),
            detail: entry.detail.clone(),
            count: entry.count,
            ref_kind: entry.ref_kind.clone(),
            ref_id: entry.ref_id,
            undo_id: entry.undo_id,
            outcome: entry.outcome,
        }
    }
    /// A failure the user should notice: an error row, or an action that failed.
    pub fn is_error(&self) -> bool {
        self.kind == ActivityKind::Error || self.outcome == ActivityOutcome::Failed
    }
}
impl ActivityFilter {
    pub fn matches(self, entry: &ActivityEntry) -> bool {
        match self {
            Self::All => true,
            Self::Action => entry.kind == ActivityKind::Action,
            Self::Event => entry.kind == ActivityKind::Event,
            Self::Error => entry.is_error(),
        }
    }
}

/// Rows older than this are pruned by the maintenance job.
pub const ACTIVITY_RETENTION_MS: i64 = 90 * 86_400_000;

/// A failure that keeps repeating (a sync every minute, a model install with backoff) is one row
/// while it stays inside this window; a quiet recovery writes no success row, so without a bound a
/// later outage with the same message would be dropped for good.
pub const FAILURE_REPEAT_MS: i64 = 6 * 3_600_000;

/// The row codes. Mail action codes are the app's own action names.
pub mod code {
    pub const MARK_AS_DONE: &str = "markAsDone";
    pub const ARCHIVE: &str = "archive";
    pub const SNOOZE: &str = "snooze";
    pub const MOVE_TO_TRASH: &str = "moveToTrash";
    pub const SEND: &str = "send";
    pub const DISCARD_DRAFT: &str = "discardDraft";
    pub const SCHEDULE_DRAFT: &str = "scheduleDraft";
    pub const SYNC_COMPLETE: &str = "syncComplete";
    pub const SYNC_FAILED: &str = "syncFailed";
    pub const INDEX_COMPLETE: &str = "indexComplete";
    pub const INDEX_FAILED: &str = "indexFailed";
    pub const MODEL_INSTALLED: &str = "modelInstalled";
    pub const MODEL_FAILED: &str = "modelFailed";
    pub const CONTACTS_COMPLETE: &str = "contactsComplete";
    pub const CONTACTS_FAILED: &str = "contactsFailed";
}

/// The mail actions that enter the log; every other action string (read, pin,
/// labels…) is ignored.
pub const LOGGED_MAIL_ACTIONS: [&str; 4] =
    [code::MARK_AS_DONE, code::ARCHIVE, code::SNOOZE, code::MOVE_TO_TRASH];
