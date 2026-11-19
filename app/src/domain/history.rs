//! Where the history backfill of an account stands (W3.1): the walk of Gmail's
//! All Mail from the newest UID to the oldest, in batches of UIDs, plus the
//! messages that arrive above the read range. Identifiers and counters only.
use serde::{Deserialize, Serialize};

/// UIDs per batch (W3.1).
pub const BATCH_UIDS: u32 = 500;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct HistoryProgress {
    /// The All Mail folder walked, by its IMAP name.
    pub folder: String,
    pub uid_validity: Option<u32>,
    /// Lowest UID read so far; 0 before the first batch.
    pub before_uid: u32,
    /// Highest UID read so far: later ones are new arrivals.
    pub highest_seen: u32,
    /// Messages in the folder at the last batch (`EXISTS`).
    pub total: u32,
    /// Messages written by the backfill.
    pub fetched: u32,
    /// The walk reached the oldest UID.
    pub complete: bool,
    pub last_error: Option<String>,
    /// The account quota stopped the walk: the date of the oldest message kept.
    pub stopped_for_space: Option<i64>,
    /// The notice about many old unread Inbox messages was shown.
    pub unread_notice_shown: bool,
    /// Unread Inbox messages the walk brought in.
    pub unread_inbox: u32,
    /// Folders asked from the interface ("Spam", "Trash") read with the next batch.
    pub requested: Vec<String>,
    /// Inbox messages older than this instant are marked read on Gmail with the
    /// next batch (the choice of the unread notice).
    pub mark_read_before: Option<i64>,
}

/// Unread Inbox messages from the history above which the notice is shown once.
pub const UNREAD_NOTICE_THRESHOLD: u32 = 100;

/// One UID range to fetch, inclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UidRange {
    pub start: u32,
    pub end: u32,
    /// Below the read range (history) rather than above it (new arrivals).
    pub older: bool,
}

impl HistoryProgress {
    /// Where the walk stands, without the requests that ride on it.
    pub fn cursor(&self) -> (Option<u32>, u32, u32, bool, Option<i64>) {
        (self.uid_validity, self.before_uid, self.highest_seen, self.complete, self.stopped_for_space)
    }
    /// A new UID generation: every UID is unrelated, the walk starts again from
    /// the top. Rows are never removed; `remote_global_id` finds them again (EX-7).
    pub fn restart(&mut self, uid_validity: u32) {
        self.uid_validity = Some(uid_validity);
        self.before_uid = 0;
        self.highest_seen = 0;
        self.complete = false;
        self.stopped_for_space = None;
    }
    /// The next range for a folder whose next UID is `uid_next`: arrivals above
    /// the read range first, then older mail below it. `None`: nothing to read.
    pub fn plan(&self, uid_next: u32, batch: u32) -> Option<UidRange> {
        let batch = batch.max(1);
        let top = uid_next.saturating_sub(1);
        if self.before_uid == 0 {
            if top == 0 {
                return None;
            }
            return Some(UidRange { start: top.saturating_sub(batch - 1).max(1), end: top, older: true });
        }
        if top > self.highest_seen {
            let start = self.highest_seen + 1;
            return Some(UidRange { start, end: top.min(start.saturating_add(batch - 1)), older: false });
        }
        if self.complete || self.stopped_for_space.is_some() || self.before_uid <= 1 {
            return None;
        }
        let end = self.before_uid - 1;
        Some(UidRange { start: end.saturating_sub(batch - 1).max(1), end, older: true })
    }
    /// Records a written range and the messages it brought.
    pub fn advance(&mut self, range: UidRange, written: u32) {
        if range.older || self.before_uid == 0 {
            self.before_uid = if self.before_uid == 0 { range.start } else { self.before_uid.min(range.start) };
            self.complete = range.start <= 1;
        }
        self.highest_seen = self.highest_seen.max(range.end);
        self.fetched = self.fetched.saturating_add(written);
        self.last_error = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn walk(progress: &mut HistoryProgress, uid_next: u32, batch: u32) -> Vec<(u32, u32)> {
        let mut ranges = Vec::new();
        while let Some(range) = progress.plan(uid_next, batch) {
            ranges.push((range.start, range.end));
            progress.advance(range, 1);
            assert!(ranges.len() < 1000);
        }
        ranges
    }
    #[test]
    fn an_empty_folder_has_nothing_to_read() {
        assert_eq!(HistoryProgress::default().plan(1, 500), None);
    }
    #[test]
    fn the_walk_goes_from_the_newest_batch_to_the_oldest() {
        let mut progress = HistoryProgress::default();
        assert_eq!(walk(&mut progress, 1201, 500), [(701, 1200), (201, 700), (1, 200)], "the last batch is partial");
        assert!(progress.complete);
        assert_eq!(progress.plan(1201, 500), None);
    }
    #[test]
    fn one_page_is_enough_for_a_small_folder() {
        let mut progress = HistoryProgress::default();
        assert_eq!(walk(&mut progress, 41, 500), [(1, 40)]);
        assert!(progress.complete);
    }
    #[test]
    fn new_arrivals_come_before_the_rest_of_the_history() {
        let mut progress = HistoryProgress::default();
        let first = progress.plan(1001, 500).unwrap();
        progress.advance(first, 500);
        // 20 messages arrived meanwhile.
        let next = progress.plan(1021, 500).unwrap();
        assert_eq!((next.start, next.end, next.older), (1001, 1020, false));
        progress.advance(next, 20);
        let older = progress.plan(1021, 500).unwrap();
        assert_eq!((older.start, older.end, older.older), (1, 500, true));
        progress.advance(older, 500);
        assert!(progress.complete);
        // A complete walk still sees later arrivals.
        assert_eq!(progress.plan(1025, 500).map(|r| (r.start, r.end)), Some((1021, 1024)));
    }
    #[test]
    fn a_changed_uid_generation_starts_again_from_the_top() {
        // EX-7
        let mut progress = HistoryProgress::default();
        walk(&mut progress, 300, 100);
        assert!(progress.complete);
        progress.restart(9);
        assert!(!progress.complete);
        assert_eq!(progress.plan(50, 100).map(|r| (r.start, r.end)), Some((1, 49)));
    }
    #[test]
    fn a_walk_stopped_for_space_waits() {
        // EX-8
        let mut progress = HistoryProgress::default();
        let first = progress.plan(1001, 500).unwrap();
        progress.advance(first, 500);
        progress.stopped_for_space = Some(1_500_000_000);
        assert_eq!(progress.plan(1001, 500), None);
    }
    #[test]
    fn old_accounts_without_a_history_load() {
        let progress: HistoryProgress = serde_json::from_str("{}").unwrap();
        assert_eq!(progress, HistoryProgress::default());
    }
}
