//! Durable progress contains identifiers and counters, never message contents.
use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 3;

/// Where a folder scan stands. The messages read by the current pass form one
/// contiguous UID range `[before_uid, after_uid]`: new mail extends it upwards from
/// `after_uid`, older mail extends it downwards from `before_uid`, newest first.
/// Everything at or below `floor_uid` was read by the former oldest-first scan, so
/// the downward walk stops there. A checkpoint of that scan (`after_uid` only) is
/// turned into this shape on the first step: the unread part above it is then read
/// newest first as well.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FolderProgress {
    pub name: String,
    pub role: String,
    pub uid_validity: Option<u32>,
    pub after_uid: u32,
    #[serde(default)]
    pub before_uid: u32,
    #[serde(default)]
    pub floor_uid: u32,
    pub total: u32,
    pub complete: bool,
    /// Gmail Inbox next to All Mail: only mail that arrives from now on is read
    /// here; its history is read once, in All Mail.
    #[serde(default)]
    pub arrivals_only: bool,
    /// Gmail label folder (Sent, Drafts, user labels): a view of mail that All
    /// Mail already holds. Never scanned.
    #[serde(default)]
    pub view: bool,
}

/// Role order of a Gmail scan: new Inbox mail, then All Mail, then Spam and Trash.
fn scan_rank(folder: &FolderProgress) -> u8 {
    match folder.role.as_str() {
        "Inbox" => 0,
        "Archive" => 1,
        "Spam" => 2,
        "Trash" => 3,
        _ => 4,
    }
}

/// Prepares the folders of an account for a scan. With a Gmail All Mail folder
/// (role `Archive`), the history is read there once: the Inbox reads only new
/// arrivals and the label folders are views, never scanned. Without it every
/// folder is scanned, Inbox first, as before.
pub fn plan_folders(folders: &mut [FolderProgress]) {
    let all_mail = folders.iter().any(|folder| folder.role == "Archive");
    for folder in folders.iter_mut() {
        folder.arrivals_only = all_mail && folder.role == "Inbox";
        folder.view = all_mail && !matches!(folder.role.as_str(), "Inbox" | "Archive" | "Spam" | "Trash");
        if folder.view {
            folder.complete = true;
        }
    }
}

/// The next folder to scan: the first incomplete one in scan order.
pub fn next_folder(folders: &[FolderProgress]) -> Option<&FolderProgress> {
    let gmail = folders.iter().any(|folder| folder.role == "Archive" && !folder.view);
    let mut candidates: Vec<&FolderProgress> = folders.iter().filter(|f| !f.complete && !f.view).collect();
    if gmail {
        candidates.sort_by_key(|folder| scan_rank(folder));
    }
    candidates.into_iter().next()
}

/// One UID range to search for the next message to read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScanWindow {
    pub start: u32,
    pub end: u32,
    /// Take the highest UID of the range (older mail below the read range) or the
    /// lowest (new mail above it).
    pub descending: bool,
}

impl FolderProgress {
    /// Nothing read yet and no top recorded.
    fn untouched(&self) -> bool {
        self.after_uid == 0 && self.before_uid == 0
    }
    /// A checkpoint of the oldest-first scan: only `after_uid` is set.
    fn legacy(&self) -> bool {
        self.after_uid > 0 && self.before_uid == 0
    }
    /// Forgets the position: a new UID generation makes every UID unrelated.
    pub fn restart(&mut self) {
        self.after_uid = 0;
        self.before_uid = 0;
        self.floor_uid = 0;
        self.complete = false;
    }
    /// Marks an empty mailbox as read up to its last UID, so its next message is new mail.
    pub fn finish_empty(&mut self, upper: u32) {
        self.after_uid = upper;
        self.before_uid = upper.saturating_add(1);
        self.floor_uid = upper;
        self.complete = true;
    }
    /// The lowest UID the downward walk still has to read.
    fn lowest_unread(&self) -> u32 {
        self.floor_uid.saturating_add(1)
    }
    /// Everything up to the mailbox's last UID `upper` is read.
    fn is_done(&self, upper: u32) -> bool {
        !self.untouched() && self.after_uid >= upper && self.before_uid <= self.lowest_unread()
    }
    /// The next range to search, at most `span` UIDs, or `None` when the folder is
    /// read (and then marked complete). New mail above the read range always comes
    /// before older mail below it; a folder never read, or read only by the old
    /// oldest-first scan, starts at its newest UID. Changes `self`: call it on a copy
    /// of the stored progress and keep the copy only once the step has succeeded.
    pub fn plan(&mut self, upper: u32, span: u32) -> Option<ScanWindow> {
        if self.arrivals_only {
            // Everything present now counts as read: only later UIDs are new.
            if self.untouched() {
                self.after_uid = upper;
                self.before_uid = upper.saturating_add(1);
            } else if self.legacy() {
                self.before_uid = self.after_uid.saturating_add(1);
            }
            self.floor_uid = self.before_uid.saturating_sub(1);
        }
        if self.untouched() || (self.legacy() && self.after_uid < upper) {
            // Everything up to the old cursor is read: the walk down stops there.
            self.floor_uid = self.after_uid;
            // The read range starts empty, right above the newest message.
            self.after_uid = upper;
            self.before_uid = upper.saturating_add(1);
        } else if self.legacy() {
            self.floor_uid = self.after_uid.min(upper);
            self.before_uid = self.lowest_unread();
        }
        let span = span.max(1);
        if self.after_uid < upper {
            let start = self.after_uid + 1;
            let end = start.saturating_add(span - 1).min(upper);
            return Some(ScanWindow { start, end, descending: false });
        }
        if self.before_uid > self.lowest_unread() {
            let end = self.before_uid - 1;
            let start = (end + 1).saturating_sub(span).max(self.lowest_unread());
            return Some(ScanWindow { start, end, descending: true });
        }
        self.complete = true;
        None
    }
    /// Searches `window` among the UIDs the server lists for it, takes the next one
    /// to read (the highest below the read range, the lowest above it), moves the
    /// read range over it and marks the folder complete when nothing is left. With
    /// no UID in the window the whole window is crossed: a stretch of deleted mail.
    pub fn advance(
        &mut self,
        window: ScanWindow,
        listed: impl IntoIterator<Item = u32>,
        upper: u32,
    ) -> Option<u32> {
        let inside = listed
            .into_iter()
            .filter(|uid| (window.start..=window.end).contains(uid));
        let found = if window.descending { inside.max() } else { inside.min() };
        if window.descending {
            self.before_uid = found.unwrap_or(window.start);
        } else {
            self.after_uid = found.unwrap_or(window.end);
        }
        self.complete = self.is_done(upper);
        found
    }
    /// Like `advance`, for up to `max` messages of one window, in walk order
    /// (highest first below the read range, lowest first above it). The read
    /// range moves over all of them; a window with fewer than `max` messages is
    /// crossed whole.
    pub fn advance_many(
        &mut self,
        window: ScanWindow,
        listed: impl IntoIterator<Item = u32>,
        upper: u32,
        max: usize,
    ) -> Vec<u32> {
        let mut inside: Vec<u32> = listed
            .into_iter()
            .filter(|uid| (window.start..=window.end).contains(uid))
            .collect();
        inside.sort_unstable();
        inside.dedup();
        if window.descending {
            inside.reverse();
        }
        let exhausted = inside.len() <= max;
        inside.truncate(max.max(1));
        match (window.descending, exhausted, inside.last()) {
            (true, false, Some(last)) => self.before_uid = *last,
            (true, _, _) => self.before_uid = window.start,
            (false, false, Some(last)) => self.after_uid = *last,
            (false, _, _) => self.after_uid = window.end,
        }
        self.complete = self.is_done(upper);
        inside
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct IndexProgress {
    pub enabled: bool,
    pub folders: Vec<FolderProgress>,
    pub state: String,
    pub scanned: u64,
    pub indexed: u64,
    pub partial: u64,
    pub last_check: i64,
    pub last_discovery: i64,
    pub prefer_history: bool,
    pub storage_limited: bool,
    pub notice: Option<String>,
    pub inventory_total: Option<u64>,
    pub inventory_attempted: bool,
}
impl Default for IndexProgress {
    fn default() -> Self {
        Self {
            enabled: true,
            folders: Vec::new(),
            state: "waiting_model".into(),
            scanned: 0,
            indexed: 0,
            partial: 0,
            last_check: 0,
            last_discovery: 0,
            prefer_history: false,
            storage_limited: false,
            notice: None,
            inventory_total: None,
            inventory_attempted: false,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageIndexState {
    pub version: u32,
    pub content_hash: String,
    pub partial: bool,
    pub notice: Option<String>,
    #[serde(default)]
    pub retry_at: i64,
    /// Failed attempts caused by the message itself. Pauses never count.
    #[serde(default)]
    pub attempts: u32,
    /// Indexed for words only: the search model was missing. A later pass adds
    /// the vectors once the model is installed (EX-9).
    #[serde(default)]
    pub semantic_pending: bool,
}

#[derive(Clone, Serialize)]
pub struct AccountIndexStatus {
    pub account_id: u32,
    pub account: String,
    pub progress: IndexProgress,
    pub indexed_present: usize,
    pub total: Option<u64>,
    pub percent: Option<u8>,
    /// Messages of the account in the list (W3.3).
    pub listed: usize,
    /// The history walk of a Gmail account; `None` for other providers.
    pub history: Option<crate::domain::history::HistoryProgress>,
    /// Spam and Trash are listed with the history.
    pub include_spam_trash: bool,
}

#[cfg(test)]
mod scan_tests {
    use super::*;

    /// Walks a folder the way the IMAP source does, one step per message or empty
    /// window, and returns the UIDs in the order they were read.
    fn walk(folder: &mut FolderProgress, uids: &[u32], span: u32) -> Vec<u32> {
        let upper = uids.iter().copied().max().unwrap_or(0);
        let mut visited = Vec::new();
        for _ in 0..10_000 {
            let Some(window) = folder.plan(upper, span) else { break };
            visited.extend(folder.advance(window, uids.iter().copied(), upper));
        }
        visited
    }
    fn folder(after_uid: u32, before_uid: u32) -> FolderProgress {
        FolderProgress { name: "Archive".into(), after_uid, before_uid, ..Default::default() }
    }

    #[test]
    fn a_new_folder_is_read_from_the_newest_message_to_the_oldest() {
        let mut folder = folder(0, 0);
        assert_eq!(walk(&mut folder, &[1, 5, 9, 10], 3), [10, 9, 5, 1]);
        assert!(folder.complete, "the walk ends at the bottom of the mailbox");
    }

    #[test]
    fn deleted_stretches_are_crossed_without_a_message() {
        let mut folder = folder(0, 0);
        assert_eq!(walk(&mut folder, &[2, 7_000], 2048), [7_000, 2]);
        assert!(folder.complete);
    }

    #[test]
    fn a_window_of_one_uid_reads_every_message_once() {
        let mut folder = folder(0, 0);
        assert_eq!(walk(&mut folder, &[1, 2, 4, 5], 1), [5, 4, 2, 1]);
        assert!(folder.complete);
    }

    #[test]
    fn below_the_read_range_the_highest_message_of_a_window_comes_first() {
        let mut folder = folder(0, 0);
        let first = folder.plan(10, 4).unwrap();
        assert_eq!((first.start, first.end, first.descending), (7, 10, true), "a new folder starts at its newest UID");
        assert_eq!(folder.advance(first, [7, 8, 9, 10], 10), Some(10));
        assert!(!folder.complete, "the rest of the window is still unread");
    }

    #[test]
    fn above_the_read_range_the_lowest_message_of_a_window_comes_first() {
        // UIDs 8..=10 are read; 11 and 12 arrived since.
        let mut folder = FolderProgress { after_uid: 10, before_uid: 8, floor_uid: 0, ..folder(0, 0) };
        let up = folder.plan(12, 4).unwrap();
        assert_eq!((up.start, up.end, up.descending), (11, 12, false));
        assert_eq!(folder.advance(up, [11, 12], 12), Some(11));
        assert_eq!(folder.after_uid, 11);
    }
    #[test]
    fn a_checkpoint_of_the_oldest_first_scan_goes_newest_first_down_to_its_cursor() {
        // Old stores: everything up to `after_uid` was read, oldest first.
        let mut folder = folder(5, 0);
        assert_eq!(walk(&mut folder, &[1, 2, 3, 4, 5, 6, 7, 8], 3), [8, 7, 6]);
        assert!(folder.complete, "the old cursor already covered the bottom");
        assert_eq!(folder.floor_uid, 5);
    }

    #[test]
    fn a_checkpoint_of_the_oldest_first_scan_is_migrated_once() {
        let mut folder = folder(5, 0);
        let first = folder.plan(8, 3).unwrap();
        assert_eq!((first.start, first.end, first.descending), (6, 8, true));
        assert_eq!(folder.advance(first, [6, 7, 8], 8), Some(8));
        let second = folder.plan(8, 3).unwrap();
        assert_eq!((folder.floor_uid, folder.after_uid), (5, 8), "the migration did not run twice");
        assert_eq!((second.start, second.end, second.descending), (6, 7, true), "the walk goes on below 8, it does not restart");
    }

    #[test]
    fn a_finished_checkpoint_of_the_oldest_first_scan_stays_finished_until_new_mail_comes() {
        let mut folder = folder(8, 0);
        assert!(walk(&mut folder, &[1, 2, 3, 4, 5, 6, 7, 8], 3).is_empty());
        assert!(folder.complete);
        folder.complete = false;
        assert_eq!(walk(&mut folder, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10], 3), [9, 10]);
        assert!(folder.complete);
    }

    #[test]
    fn a_checkpoint_beyond_the_last_uid_is_read_as_finished() {
        let mut folder = folder(50, 0);
        assert!(walk(&mut folder, &[1, 2, 3], 3).is_empty());
        assert!(folder.complete);
    }

    #[test]
    fn new_mail_is_read_before_the_walk_goes_on_downwards() {
        let mut folder = folder(0, 0);
        let window = folder.plan(10, 2).unwrap();
        folder.advance(window, [8, 10], 10);
        // Two messages arrive while the walk is at 10.
        assert_eq!(walk(&mut folder, &[1, 4, 8, 10, 11, 12], 2), [11, 12, 8, 4, 1]);
        assert!(folder.complete);
    }

    #[test]
    fn an_empty_mailbox_is_complete_and_still_sees_its_first_message() {
        let mut folder = folder(0, 0);
        folder.finish_empty(0);
        assert!(folder.complete);
        folder.complete = false;
        assert_eq!(walk(&mut folder, &[1], 2048), [1]);
        assert!(folder.complete);
    }

    #[test]
    fn an_emptied_mailbox_reads_only_mail_newer_than_what_was_expunged() {
        let mut folder = folder(0, 0);
        folder.finish_empty(5);
        assert_eq!((folder.after_uid, folder.floor_uid, folder.complete), (5, 5, true));
        folder.complete = false;
        assert_eq!(walk(&mut folder, &[6, 7], 2048), [6, 7]);
        assert!(folder.complete);
    }

    #[test]
    fn a_completed_folder_rescanned_later_reads_only_the_new_arrivals() {
        let mut folder = folder(0, 0);
        walk(&mut folder, &[1, 2, 3], 2);
        folder.complete = false;
        assert_eq!(walk(&mut folder, &[1, 2, 3, 4], 2), [4]);
        folder.complete = false;
        assert!(walk(&mut folder, &[1, 2, 3, 4], 2).is_empty());
        assert!(folder.complete);
    }

    #[test]
    fn a_stretch_of_new_deleted_mail_above_the_range_is_crossed_too() {
        let mut folder = folder(0, 0);
        walk(&mut folder, &[1, 2], 2048);
        folder.complete = false;
        assert_eq!(walk(&mut folder, &[1, 2, 9_000], 2048), [9_000]);
        assert!(folder.complete);
    }

    #[test]
    fn restart_forgets_every_cursor() {
        let mut folder = FolderProgress { after_uid: 9, before_uid: 3, floor_uid: 2, complete: true, ..Default::default() };
        folder.restart();
        assert_eq!((folder.after_uid, folder.before_uid, folder.floor_uid, folder.complete), (0, 0, 0, false));
    }

    fn walk_batches(folder: &mut FolderProgress, uids: &[u32], span: u32, max: usize) -> Vec<Vec<u32>> {
        let upper = uids.iter().copied().max().unwrap_or(0);
        let mut batches = Vec::new();
        for _ in 0..10_000 {
            let Some(window) = folder.plan(upper, span) else { break };
            let batch = folder.advance_many(window, uids.iter().copied(), upper, max);
            if !batch.is_empty() { batches.push(batch); }
        }
        batches
    }

    #[test]
    fn batches_read_the_same_order_as_single_steps() {
        let uids = [1, 2, 3, 5, 8, 9, 10, 11, 20, 21];
        let single = walk(&mut folder(0, 0), &uids, 4);
        let batched: Vec<u32> = walk_batches(&mut folder(0, 0), &uids, 4, 3).concat();
        assert_eq!(single, batched);
        let mut whole = folder(0, 0);
        assert_eq!(walk_batches(&mut whole, &uids, 2048, 25), [vec![21, 20, 11, 10, 9, 8, 5, 3, 2, 1]]);
        assert!(whole.complete);
    }

    #[test]
    fn a_full_batch_stops_on_its_last_message_and_resumes_below_it() {
        let mut folder = folder(0, 0);
        let window = folder.plan(10, 2048).unwrap();
        assert_eq!(folder.advance_many(window, 1..=10, 10, 4), [10, 9, 8, 7]);
        assert_eq!(folder.before_uid, 7);
        assert!(!folder.complete);
        let window = folder.plan(10, 2048).unwrap();
        assert_eq!(folder.advance_many(window, 1..=10, 10, 25), [6, 5, 4, 3, 2, 1]);
        assert!(folder.complete);
    }

    #[test]
    fn new_mail_comes_first_in_batches_too() {
        let mut folder = FolderProgress { after_uid: 10, before_uid: 8, floor_uid: 0, ..folder(0, 0) };
        let window = folder.plan(14, 2048).unwrap();
        assert_eq!(folder.advance_many(window, [11, 12, 13, 14], 14, 2), [11, 12]);
        assert_eq!(folder.after_uid, 12);
    }

    #[test]
    fn an_arrivals_only_inbox_skips_its_history_and_reads_new_mail() {
        let mut inbox = FolderProgress { name: "INBOX".into(), role: "Inbox".into(), arrivals_only: true, ..Default::default() };
        assert!(walk(&mut inbox, &[1, 2, 3], 2048).is_empty(), "history is read in All Mail");
        assert!(inbox.complete);
        inbox.complete = false;
        assert_eq!(walk(&mut inbox, &[1, 2, 3, 4, 5], 2048), [4, 5]);
        // An Inbox half way down its history from an older version stops going down.
        let mut started = FolderProgress { after_uid: 50, before_uid: 40, arrivals_only: true, ..Default::default() };
        assert_eq!(walk(&mut started, &[10, 20, 45, 50, 51], 2048), [51]);
    }

    #[test]
    fn gmail_folders_read_history_once_in_all_mail() {
        let folder = |name: &str, role: &str| FolderProgress { name: name.into(), role: role.into(), ..Default::default() };
        let mut folders = vec![folder("INBOX", "Inbox"), folder("[Gmail]/Sent", "Sent"), folder("Clienti", "Clienti"), folder("[Gmail]/Trash", "Trash"), folder("[Gmail]/All", "Archive"), folder("[Gmail]/Spam", "Spam")];
        plan_folders(&mut folders);
        assert!(folders[0].arrivals_only);
        assert!(folders[1].view && folders[1].complete && folders[2].view);
        let order: Vec<_> = std::iter::from_fn(|| {
            let next = next_folder(&folders)?.name.clone();
            folders.iter_mut().find(|f| f.name == next).unwrap().complete = true;
            Some(next)
        }).collect();
        assert_eq!(order, ["INBOX", "[Gmail]/All", "[Gmail]/Spam", "[Gmail]/Trash"]);
        let mut generic = vec![folder("INBOX", "Inbox"), folder("Sent", "Sent")];
        plan_folders(&mut generic);
        assert!(!generic[0].arrivals_only && !generic[1].view, "without All Mail every folder is scanned");
    }

    #[test]
    fn old_stores_without_the_new_cursors_still_load() {
        let json = r#"{"name":"Archive","role":"Archive","uid_validity":7,"after_uid":12,"total":40,"complete":false}"#;
        let folder: FolderProgress = serde_json::from_str(json).unwrap();
        assert_eq!((folder.after_uid, folder.before_uid, folder.floor_uid), (12, 0, 0));
    }
}