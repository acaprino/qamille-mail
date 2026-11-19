use super::{actions, model::Store};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UndoOperation {
    Mail {
        email_id: u64,
        action: String,
        value: Option<String>,
    },
    SendDraft {
        draft_id: u64,
    },
    DiscardDraft {
        draft_id: u64,
    },
    ScheduleDraft {
        draft_id: u64,
        send_at: i64,
    },
}
impl UndoOperation {
    pub fn draft_id(&self) -> Option<u64> {
        match self {
            Self::Mail { .. } => None,
            Self::SendDraft { draft_id }
            | Self::DiscardDraft { draft_id }
            | Self::ScheduleDraft { draft_id, .. } => Some(*draft_id),
        }
    }
    pub fn same_target(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Mail { email_id: a, .. }, Self::Mail { email_id: b, .. }) => a == b,
            _ => self.draft_id().is_some() && self.draft_id() == other.draft_id(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UndoStatus {
    Pending,
    Committing,
    Failed,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct UndoAction {
    pub id: u64,
    pub operation: UndoOperation,
    pub subject: String,
    pub created_at_ms: i64,
    pub expires_at_ms: i64,
    pub status: UndoStatus,
    pub error: Option<String>,
    /// Next attempt of a confirmation that never reached the provider. `None`
    /// until the first retryable failure; the queue skips the action until then.
    /// Defaulted so data files written before the retry queue still load.
    #[serde(default)]
    pub retry_at_ms: Option<i64>,
    /// Retryable failures so far. Grows without a cap while the account exists,
    /// and drives the incremental backoff (ADR-0007 v3, decision 8).
    #[serde(default)]
    pub attempts: u32,
}

/// Pending commands only affect the displayed view. The saved mail and provider
/// are changed after the undo window, so cancellation never needs a reverse move.
/// Whether `project` would change anything: some action is still pending.
pub fn has_pending(store: &Store) -> bool {
    store.undo_actions.iter().any(|pending| pending.status != UndoStatus::Failed)
}

/// The projected store, sharing `store` when nothing is pending (no copy).
pub fn project_shared(store: std::sync::Arc<Store>) -> std::sync::Arc<Store> {
    if has_pending(&store) {
        std::sync::Arc::new(project((*store).clone()))
    } else {
        store
    }
}

pub fn project(mut store: Store) -> Store {
    for pending in store.undo_actions.clone() {
        if pending.status == UndoStatus::Failed {
            continue;
        }
        match pending.operation {
            UndoOperation::Mail {
                email_id,
                action,
                value,
            } => {
                // Absolute queued dates were validated when the action was
                // created. Project the same instant using that clock, rather
                // than a second system-clock reminder-limit check.
                if matches!(action.as_str(), "snooze" | "changeReminder" | "setDue") {
                    if let Some(when) = value.as_deref().and_then(|value|
                        chrono::DateTime::parse_from_rfc3339(value).ok()) {
                        let _ = super::regional::set_email_date(&mut store, email_id, &action,
                            when.timestamp(), pending.created_at_ms / 1000);
                        continue;
                    }
                }
                let _ = actions::apply(&mut store, email_id, &action, value.as_deref());
            }
            operation => store.drafts.retain(|d| Some(d.id) != operation.draft_id()),
        }
    }
    store
}
