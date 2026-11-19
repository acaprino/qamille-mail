//! Contact entities. No I/O.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Labeled {
    pub label: String,
    pub value: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct ContactFields {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub org: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub birthday: String,
    #[serde(default)]
    pub emails: Vec<Labeled>,
    #[serde(default)]
    pub phones: Vec<Labeled>,
    #[serde(default)]
    pub addresses: Vec<Labeled>,
    #[serde(default)]
    pub notes: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum BookKind {
    Local,
    Google,
    GoogleOther,
    Mail,
}

/// `local`, `google:{account}`, `google-other:{account}` or `mail:{account}`.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(transparent)]
pub struct BookId(pub String);

impl BookId {
    pub fn local() -> Self {
        Self("local".into())
    }
    pub fn google(account: u32) -> Self {
        Self(format!("google:{account}"))
    }
    pub fn google_other(account: u32) -> Self {
        Self(format!("google-other:{account}"))
    }
    pub fn mail(account: u32) -> Self {
        Self(format!("mail:{account}"))
    }
    pub fn kind(&self) -> BookKind {
        if self.0.starts_with("google-other:") {
            BookKind::GoogleOther
        } else if self.0.starts_with("google:") {
            BookKind::Google
        } else if self.0.starts_with("mail:") {
            BookKind::Mail
        } else {
            BookKind::Local
        }
    }
    pub fn account_id(&self) -> Option<u32> {
        self.0.split_once(':').and_then(|(_, id)| id.parse().ok())
    }
    /// Only the local book and Google address books accept writes.
    pub fn writable(&self) -> bool {
        match self.kind() {
            BookKind::Local => self.0 == "local",
            BookKind::Google => self.account_id().is_some(),
            BookKind::GoogleOther | BookKind::Mail => false,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub enum SyncState {
    #[default]
    Synced,
    Pending,
    Conflict,
    Error(String),
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct SourceContact {
    pub key: String,
    pub book: BookId,
    #[serde(default)]
    pub remote_id: Option<String>,
    #[serde(default)]
    pub etag: Option<String>,
    #[serde(default)]
    pub uid: Option<String>,
    pub fields: ContactFields,
    /// Last copy known to match the server: the base of a three-way comparison.
    #[serde(default)]
    pub base: Option<ContactFields>,
    /// The server copy that collided with a local edit, kept while in conflict.
    #[serde(default)]
    pub theirs: Option<ContactFields>,
    #[serde(default)]
    pub modified: i64,
    /// Provider data this app does not manage, written back unchanged.
    #[serde(default)]
    pub unmanaged: serde_json::Value,
    #[serde(default)]
    pub sync: SyncState,
    /// Removed by the user; kept until the provider confirms the deletion.
    #[serde(default)]
    pub deleted: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Link {
    pub keys: Vec<String>,
    /// Field name to the value the user fixed by hand.
    #[serde(default)]
    pub pins: BTreeMap<String, String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Certain,
    Probable,
    Possible,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct DuplicateGroup {
    pub keys: Vec<String>,
    pub level: Level,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteOp {
    Create,
    Update,
    Delete,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct QueuedWrite {
    pub key: String,
    pub op: WriteOp,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct BookOutcome {
    pub book: String,
    pub ok: bool,
    pub error: Option<String>,
}

/// Cards as they were before a merge, kept so it can be undone.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct MergeJournal {
    pub id: u64,
    pub at: i64,
    pub before: Vec<SourceContact>,
    /// Cards the merge created, removed again by an undo.
    #[serde(default)]
    pub created: Vec<String>,
    /// Keys the merge joined with a link, separated again by an undo.
    #[serde(default)]
    pub link_keys: Vec<String>,
    /// Links that touched the merged cards before the merge, put back by an undo.
    #[serde(default)]
    pub links_before: Vec<Link>,
    #[serde(default)]
    pub outcome: Vec<BookOutcome>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct ContactSettings {
    /// Books in the order whose value wins when a single-value field differs.
    #[serde(default)]
    pub priority: Vec<BookId>,
    /// Extra role-address local parts (`info`, ...) that never group people.
    #[serde(default)]
    pub roles: Vec<String>,
    #[serde(default)]
    pub last_book: Option<String>,
    /// Books whose synchronization the user switched off.
    #[serde(default)]
    pub sync_off: Vec<String>,
    /// Books for which the write permission was granted.
    #[serde(default)]
    pub write_granted: Vec<String>,
}

/// Everything stored in `contacts.json`. The unified book itself is derived, never stored.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct ContactsState {
    #[serde(default)]
    pub sources: Vec<SourceContact>,
    #[serde(default)]
    pub links: Vec<Link>,
    #[serde(default)]
    pub rejected: Vec<(String, String)>,
    #[serde(default)]
    pub hidden: Vec<String>,
    #[serde(default)]
    pub queue: Vec<QueuedWrite>,
    #[serde(default)]
    pub journal: Vec<MergeJournal>,
    #[serde(default)]
    pub settings: ContactSettings,
    #[serde(default)]
    pub sync_tokens: BTreeMap<String, String>,
    /// Unix time of the last successful synchronization per book.
    #[serde(default)]
    pub synced_at: BTreeMap<String, i64>,
    #[serde(default)]
    pub next_id: u64,
}

impl ContactsState {
    pub fn new_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn book_ids_round_trip_their_kind_and_account() {
        assert_eq!(BookId::local().0, "local");
        assert_eq!(BookId::google(7).0, "google:7");
        assert_eq!(BookId::google_other(7).kind(), BookKind::GoogleOther);
        assert_eq!(BookId::mail(3).kind(), BookKind::Mail);
        assert_eq!(BookId::local().kind(), BookKind::Local);
        assert_eq!(BookId::google(7).account_id(), Some(7));
        assert_eq!(BookId::mail(3).account_id(), Some(3));
        assert_eq!(BookId::local().account_id(), None);
    }

    #[test]
    fn only_local_and_google_books_are_writable() {
        assert!(BookId::local().writable());
        assert!(BookId::google(1).writable());
        assert!(!BookId::google_other(1).writable());
        assert!(!BookId::mail(1).writable());
    }

    #[test]
    fn a_malformed_book_id_is_read_only_and_local_free() {
        let odd = BookId("google:abc".into());
        assert_eq!(odd.account_id(), None);
        assert!(!odd.writable());
    }

    #[test]
    fn source_contact_deserializes_from_minimal_json() {
        let json = r#"{"key":"k","book":"local","fields":{"name":"Ada"}}"#;
        let source: SourceContact = serde_json::from_str(json).unwrap();
        assert_eq!(source.book, BookId::local());
        assert_eq!(source.sync, SyncState::Synced);
        assert!(source.fields.emails.is_empty());
    }
}
