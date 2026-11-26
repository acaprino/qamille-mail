//! Google synchronization: incremental reads, queued writes, conflicts and consent.
use super::{AppError, AppResult, activity, contact_ports::*, contacts::*, events::AppEvent};
use crate::domain::{
    activity::{ActivityKind, NewActivity, code},
    contact_merge::{Merge3, three_way},
    contacts::*,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct BookSync {
    pub book: String,
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub full: bool,
    pub skipped: bool,
    pub error: Option<String>,
    /// True when the book cannot be read until the user allows access to Google Contacts.
    pub consent_required: bool,
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct SyncReport {
    pub books: Vec<BookSync>,
    pub writes: Vec<BookOutcome>,
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Keep {
    Mine,
    Theirs,
}

/// How one queued write ended.
enum Step {
    Done,
    Conflict,
}

fn has_local_edits(state: &ContactsState, card: &SourceContact) -> bool {
    card.sync == SyncState::Conflict || state.queue.iter().any(|q| q.key == card.key && q.op != WriteOp::Delete)
}

/// Takes the server copy into a card, merging with unsent local edits when there are some.
fn absorb(state: &mut ContactsState, position: usize, remote: &RemoteContact, now: i64) {
    let local_edits = has_local_edits(state, &state.sources[position]);
    let card = &mut state.sources[position];
    if !local_edits {
        if card.fields != remote.fields {
            card.modified = now;
        }
        card.fields = remote.fields.clone();
        card.base = Some(remote.fields.clone());
        card.etag = Some(remote.etag.clone());
        card.unmanaged = remote.unmanaged.clone();
        card.sync = SyncState::Synced;
        return;
    }
    let base = card.base.clone().unwrap_or_else(|| remote.fields.clone());
    card.etag = Some(remote.etag.clone());
    card.unmanaged = remote.unmanaged.clone();
    match three_way(&base, &card.fields, &remote.fields) {
        Merge3::Clean(merged) => {
            card.fields = merged;
            card.base = Some(remote.fields.clone());
        }
        Merge3::Conflict(_) => {
            card.sync = SyncState::Conflict;
            card.theirs = Some(remote.fields.clone());
        }
    }
}

/// The server lost a card that still has unsent local edits: keep it and create it again on Google.
fn recreate(state: &mut ContactsState, at: usize) {
    let key = state.sources[at].key.clone();
    let card = &mut state.sources[at];
    card.remote_id = None;
    card.etag = None;
    card.base = None;
    card.theirs = None;
    card.unmanaged = serde_json::Value::Null;
    card.sync = SyncState::Pending;
    state.queue.retain(|q| q.key != key);
    state.queue.push(QueuedWrite { key, op: WriteOp::Create });
}

/// Accept only the operation that was actually sent. A concurrent deletion replaces
/// its queued update with a tombstone even when all contact fields are unchanged.
fn acknowledge_update(state: &mut ContactsState, sent: &SourceContact, fields: &ContactFields, remote: &RemoteContact) {
    if !state.queue.iter().any(|q| q.key == sent.key && q.op == WriteOp::Update) {
        return;
    }
    let Some(card) = state.sources.iter_mut().find(|c| c.key == sent.key && !c.deleted) else { return };
    let unchanged = card.fields == sent.fields;
    card.etag = Some(remote.etag.clone());
    card.base = Some(remote.fields.clone());
    if !remote.unmanaged.is_null() {
        card.unmanaged = remote.unmanaged.clone();
    }
    if unchanged {
        card.fields = fields.clone();
        card.sync = SyncState::Synced;
        state.queue.retain(|q| q.key != sent.key || q.op != WriteOp::Update);
    } else {
        card.sync = SyncState::Pending;
    }
}

impl ContactService {
    fn fetch_all(&self, account: &crate::domain::model::Account, other: bool, token: Option<&str>) -> AppResult<(Vec<RemoteContact>, Option<String>)> {
        let (mut items, mut next_token, mut page) = (Vec::new(), None, None::<String>);
        loop {
            let reply = self.gateway.list(account, other, page.as_deref(), token)?;
            items.extend(reply.items);
            if reply.next_sync_token.is_some() {
                next_token = reply.next_sync_token;
            }
            match reply.next_page {
                Some(next) => page = Some(next),
                None => return Ok((items, next_token)),
            }
        }
    }

    /// Reads one Google book. A token that expired is replaced by a full read, not reported as an error.
    /// Brackets the read with `ContactsSync` events so the single activity indicator shows it
    /// (ADR-0007 decision 7, INV-009); `sync_all` announces the run itself and calls the inner read.
    pub fn sync_book(&self, book: &str) -> AppResult<BookSync> {
        let account_id = BookId(book.to_string()).account_id();
        self.context
            .events
            .emit(AppEvent::ContactsSync { account_id, running: true });
        let result = self.sync_book_inner(book);
        self.context
            .events
            .emit(AppEvent::ContactsSync { account_id, running: false });
        result
    }

    fn sync_book_inner(&self, book: &str) -> AppResult<BookSync> {
        let id = BookId(book.to_string());
        let other = match id.kind() {
            BookKind::Google => false,
            BookKind::GoogleOther => true,
            _ => return Err(AppError::validation("This address book is not synchronized with Google")),
        };
        let state = self.store.read()?;
        if state.settings.sync_off.iter().any(|b| b == book) {
            return Ok(BookSync { book: book.into(), skipped: true, ..Default::default() });
        }
        let account = self.gmail_account(&id)?;
        let mut token = state.sync_tokens.get(book).cloned();
        let mut full = token.is_none();
        let (items, next_token) = loop {
            match self.fetch_all(&account, other, token.as_deref()) {
                Ok(read) => break read,
                Err(e) if e.code == "sync_token_expired" && token.is_some() => {
                    token = None;
                    full = true;
                }
                Err(e) => return Err(e),
            }
        };
        let now = self.context.clock.now();
        self.update_state(|state| {
            let mut report = BookSync { book: book.into(), full, ..Default::default() };
            let mut seen = HashSet::new();
            for item in &items {
                seen.insert(item.remote_id.clone());
                let position = state.sources.iter().position(|s| s.book == id && s.remote_id.as_deref() == Some(item.remote_id.as_str()));
                match (position, item.deleted) {
                    (Some(at), true) => {
                        if has_local_edits(state, &state.sources[at]) {
                            recreate(state, at);
                            report.updated += 1;
                        } else {
                            let key = state.sources[at].key.clone();
                            state.queue.retain(|q| q.key != key);
                            state.sources.remove(at);
                            state.hidden.retain(|h| h != &key);
                            report.removed += 1;
                        }
                    }
                    (None, true) => {}
                    (Some(at), false) => {
                        absorb(state, at, item, now);
                        report.updated += 1;
                    }
                    (None, false) => {
                        let (_, key) = new_source_key(state);
                        state.sources.push(SourceContact {
                            key,
                            book: id.clone(),
                            remote_id: Some(item.remote_id.clone()),
                            etag: Some(item.etag.clone()),
                            fields: item.fields.clone(),
                            base: Some(item.fields.clone()),
                            modified: now,
                            unmanaged: item.unmanaged.clone(),
                            ..Default::default()
                        });
                        report.added += 1;
                    }
                }
            }
            if full {
                let gone: Vec<String> = state
                    .sources
                    .iter()
                    .filter(|s| s.book == id && s.remote_id.as_ref().is_some_and(|r| !seen.contains(r)))
                    .map(|s| s.key.clone())
                    .collect();
                for key in &gone {
                    let Some(at) = state.sources.iter().position(|s| &s.key == key) else { continue };
                    if has_local_edits(state, &state.sources[at]) {
                        recreate(state, at);
                    } else {
                        state.queue.retain(|q| &q.key != key);
                        state.sources.remove(at);
                        state.hidden.retain(|h| h != key);
                        report.removed += 1;
                    }
                }
            }
            match next_token.clone() {
                Some(t) => state.sync_tokens.insert(book.to_string(), t),
                None => state.sync_tokens.remove(book),
            };
            state.synced_at.insert(book.to_string(), now);
            Ok(report)
        })
    }

    /// Sends waiting writes, then reads every enabled Google book. The whole run is one
    /// `ContactsSync` activity on the indicator: the per-book events only move it to the
    /// account being read, and the end of a book is never the end of the run.
    pub fn sync_all(&self) -> AppResult<SyncReport> {
        self.context
            .events
            .emit(AppEvent::ContactsSync { account_id: None, running: true });
        let result = self.sync_all_inner();
        self.context
            .events
            .emit(AppEvent::ContactsSync { account_id: None, running: false });
        // The activity log (centro attività): the run ends here, success or not.
        let logged = match &result {
            Ok(report) => contacts_activity(report),
            Err(error) => Some((
                NewActivity {
                    kind: ActivityKind::Error,
                    code: code::CONTACTS_FAILED.into(),
                    detail: Some(error.message.clone()),
                    ..Default::default()
                },
                true,
            )),
        };
        if let Some((entry, failure)) = logged {
            if failure {
                activity::record_failure(&self.context, entry, code::CONTACTS_COMPLETE);
            } else {
                activity::record(&self.context, entry);
            }
        }
        result
    }

    fn sync_all_inner(&self) -> AppResult<SyncReport> {
        let writes = self.flush_queue()?;
        let mut books = Vec::new();
        for view in self.books()?.into_iter().filter(|b| matches!(b.kind, BookKind::Google | BookKind::GoogleOther)) {
            self.context.events.emit(AppEvent::ContactsSync {
                account_id: BookId(view.id.clone()).account_id(),
                running: true,
            });
            books.push(match self.sync_book_inner(&view.id) {
                Ok(done) => done,
                Err(e) => BookSync { book: view.id, consent_required: e.code == "consent_required", error: Some(e.message), ..Default::default() },
            });
        }
        Ok(SyncReport { books, writes })
    }

    pub fn set_sync(&self, book: &str, enabled: bool) -> AppResult<()> {
        self.update_state(|state| {
            state.settings.sync_off.retain(|b| b != book);
            if !enabled {
                state.settings.sync_off.push(book.to_string());
            }
            Ok(())
        })
    }

    /// Runs the Google consent flow for reading or writing one book.
    pub fn authorize(&self, book: &str, write: bool) -> AppResult<()> {
        let id = BookId(book.to_string());
        if id.kind() != BookKind::Google {
            return Err(AppError::validation("Only Google address books need consent"));
        }
        let account = self.gmail_account(&id)?;
        self.gateway.authorize(&account, write)?;
        if write {
            self.update_state(|state| {
                if !state.settings.write_granted.contains(&id.0) {
                    state.settings.write_granted.push(id.0.clone());
                }
                Ok(())
            })?;
        }
        Ok(())
    }

    pub fn resolve_conflict(&self, key: &str, keep: Keep) -> AppResult<()> {
        let now = self.context.clock.now();
        self.update_state(|state| {
            let card = state.sources.iter_mut().find(|s| s.key == key).ok_or_else(|| AppError::missing("Contact"))?;
            let theirs = card.theirs.take().filter(|_| card.sync == SyncState::Conflict).ok_or_else(|| AppError::validation("This card has no conflict to resolve"))?;
            match keep {
                Keep::Mine => {
                    card.base = Some(theirs);
                    card.sync = SyncState::Pending;
                    card.modified = now;
                    queue_write(state, key, WriteOp::Update);
                }
                Keep::Theirs => {
                    card.fields = theirs.clone();
                    card.base = Some(theirs);
                    card.sync = SyncState::Synced;
                    card.modified = now;
                    state.queue.retain(|q| q.key != key);
                }
            }
            Ok(())
        })
    }

    /// Sends every waiting write, one at a time. A failure stays queued and is reported by book.
    pub fn flush_queue(&self) -> AppResult<Vec<BookOutcome>> {
        self.flush(None)
    }

    /// Sends only the waiting writes of these cards (used right after a merge).
    pub(crate) fn flush_keys(&self, keys: &[String]) -> AppResult<Vec<BookOutcome>> {
        self.flush(Some(keys))
    }

    fn flush(&self, only: Option<&[String]>) -> AppResult<Vec<BookOutcome>> {
        let _in_process = self.flush_lock.lock().map_err(|_| AppError::storage("Contacts write lock failed"))?;
        // Another process (the MCP server, a second window) may be sending the same queue: wait for it,
        // then read the queue, so a write it already sent is not sent again.
        let _shared = self.store.lock_flush()?;
        let accounts: Vec<u32> = self.context.read_shared()?.accounts.iter().map(|a| a.id).collect();
        let mut results: BTreeMap<String, (bool, Option<String>)> = BTreeMap::new();
        let mut note = |book: &str, ok: bool, error: Option<String>| {
            let entry = results.entry(book.to_string()).or_insert((true, None));
            if !ok && entry.0 {
                *entry = (false, error);
            }
        };
        for entry in self.store.read()?.queue {
            if only.is_some_and(|keys| !keys.contains(&entry.key)) {
                continue;
            }
            let state = self.store.read()?;
            if !state.queue.contains(&entry) {
                continue;
            }
            let Some(card) = state.sources.iter().find(|s| s.key == entry.key).cloned() else {
                self.update_state(|s| {
                    s.queue.retain(|q| q.key != entry.key);
                    Ok(())
                })?;
                continue;
            };
            if card.book.account_id().is_some_and(|id| !accounts.contains(&id)) {
                continue; // the account was removed: the write waits in case it comes back
            }
            if card.sync == SyncState::Conflict {
                note(&card.book.0, false, Some("Conflict: choose which version to keep".into()));
                continue;
            }
            let outcome = self.gmail_account(&card.book).and_then(|account| self.flush_one(&account, &card, entry.op));
            match outcome {
                Ok(Step::Done) => note(&card.book.0, true, None),
                Ok(Step::Conflict) => note(&card.book.0, false, Some("Conflict: choose which version to keep".into())),
                Err(e) => {
                    let message = e.message.clone();
                    let consent = e.code == "consent_required";
                    note(&card.book.0, false, Some(message.clone()));
                    self.update_state(|s| {
                        if !s.queue.contains(&entry) {
                            return Ok(());
                        }
                        if consent {
                            // Permission is missing or was revoked: forget the grant so the next edit asks again,
                            // and keep the card waiting in the queue instead of marking it broken.
                            s.settings.write_granted.retain(|b| b != &card.book.0);
                        } else if let Some(c) = s.sources.iter_mut().find(|c| c.key == card.key) {
                            c.sync = SyncState::Error(message.clone());
                        }
                        Ok(())
                    })?;
                }
            }
        }
        Ok(results.into_iter().map(|(book, (ok, error))| BookOutcome { book, ok, error }).collect())
    }

    fn flush_one(&self, account: &crate::domain::model::Account, card: &SourceContact, op: WriteOp) -> AppResult<Step> {
        let key = card.key.clone();
        match (op, card.remote_id.clone()) {
            (WriteOp::Create, _) | (WriteOp::Update, None) => {
                let remote = self.gateway.create(account, &card.fields)?;
                self.update_state(|s| {
                    let Some(at) = s.sources.iter().position(|c| c.key == key) else { return Ok(()) };
                    let unchanged = s.sources[at].fields == card.fields;
                    let c = &mut s.sources[at];
                    c.remote_id = Some(remote.remote_id.clone());
                    c.etag = Some(remote.etag.clone());
                    c.base = Some(remote.fields.clone());
                    c.unmanaged = remote.unmanaged.clone();
                    s.queue.retain(|q| q.key != key);
                    if unchanged {
                        s.sources[at].sync = SyncState::Synced;
                    } else {
                        // Edited while the request was in flight: it exists now, so the rest is an update.
                        s.sources[at].sync = SyncState::Pending;
                        s.queue.push(QueuedWrite { key: key.clone(), op: WriteOp::Update });
                    }
                    Ok(())
                })?;
                Ok(Step::Done)
            }
            (WriteOp::Delete, remote_id) => {
                if let Some(remote_id) = remote_id {
                    match self.gateway.delete(account, &remote_id) {
                        Ok(()) => {}
                        Err(e) if e.code == "not_found" => {}
                        Err(e) => return Err(e),
                    }
                }
                self.update_state(|s| {
                    s.sources.retain(|c| c.key != key);
                    s.queue.retain(|q| q.key != key);
                    Ok(())
                })?;
                Ok(Step::Done)
            }
            (WriteOp::Update, Some(remote_id)) => {
                let (mut fields, mut etag, mut base) = (card.fields.clone(), card.etag.clone().unwrap_or_default(), card.base.clone());
                for attempt in 0..3 {
                    match self.gateway.update(account, &remote_id, &etag, &fields, &card.unmanaged) {
                        Ok(remote) => {
                            self.update_state(|s| {
                                acknowledge_update(s, card, &fields, &remote);
                                Ok(())
                            })?;
                            return Ok(Step::Done);
                        }
                        Err(e) if e.code == "etag_conflict" && attempt < 2 => {
                            let remote = self.gateway.get(account, &remote_id)?;
                            let reference = base.clone().unwrap_or_else(|| remote.fields.clone());
                            match three_way(&reference, &fields, &remote.fields) {
                                Merge3::Clean(merged) => {
                                    fields = merged;
                                    etag = remote.etag.clone();
                                    base = Some(remote.fields.clone());
                                }
                                Merge3::Conflict(_) => {
                                    let conflicted = self.update_state(|s| {
                                        if let Some(c) = s.sources.iter_mut().find(|c| c.key == key && !c.deleted) {
                                            c.sync = SyncState::Conflict;
                                            c.theirs = Some(remote.fields.clone());
                                            c.etag = Some(remote.etag.clone());
                                            c.unmanaged = remote.unmanaged.clone();
                                            return Ok(true);
                                        }
                                        Ok(false)
                                    })?;
                                    return Ok(if conflicted { Step::Conflict } else { Step::Done });
                                }
                            }
                        }
                        Err(e) => return Err(e),
                    }
                }
                Err(AppError::new("etag_conflict", "The contact kept changing on the server; try again"))
            }
        }
    }

    /// Copies a Google "other contact" into the real contacts of the same account.
    pub fn promote_other(&self, id: &str) -> AppResult<ContactView> {
        let view = self.get(id)?;
        let origin = view.origins.iter().find(|o| BookId(o.book.clone()).kind() == BookKind::GoogleOther).ok_or_else(|| AppError::validation("This contact is not one of Google's other contacts"))?;
        let other = self.store.read()?.sources.into_iter().find(|s| s.key == origin.key).ok_or_else(|| AppError::missing("Contact"))?;
        let target = BookId::google(other.book.account_id().unwrap_or_default());
        self.ensure_write(&[target.clone()])?;
        let account = self.gmail_account(&target)?;
        let remote = self.gateway.promote_other(&account, other.remote_id.as_deref().unwrap_or_default())?;
        let now = self.context.clock.now();
        let key = self.update_state(|state| {
            drop_source(state, &other.key);
            let (_, key) = new_source_key(state);
            state.sources.push(SourceContact {
                key: key.clone(),
                book: target.clone(),
                remote_id: Some(remote.remote_id.clone()),
                etag: Some(remote.etag.clone()),
                fields: remote.fields.clone(),
                base: Some(remote.fields.clone()),
                modified: now,
                unmanaged: remote.unmanaged.clone(),
                ..Default::default()
            });
            Ok(key)
        })?;
        self.get(&key)
    }

    pub fn settings(&self) -> AppResult<ContactSettings> {
        Ok(self.store.read()?.settings)
    }

    pub fn save_settings(&self, priority: Vec<String>, roles: Vec<String>) -> AppResult<()> {
        let mut clean: Vec<String> = Vec::new();
        for role in roles.iter().map(|r| r.trim().to_lowercase()).filter(|r| !r.is_empty()) {
            if !clean.contains(&role) {
                clean.push(role);
            }
        }
        self.update_state(|state| {
            state.settings.priority = priority.iter().map(|b| BookId(b.clone())).collect();
            state.settings.roles = clean.clone();
            Ok(())
        })
    }
}

/// What a whole contacts run adds to the activity log: the first book error
/// that is not a pending consent is the failure; otherwise the number of
/// contacts that changed, or nothing when the run found none (it repeats every
/// ten minutes). The `bool` tells a failure.
fn contacts_activity(report: &SyncReport) -> Option<(NewActivity, bool)> {
    if let Some(error) = report
        .books
        .iter()
        .filter(|book| !book.consent_required)
        .find_map(|book| book.error.clone())
    {
        return Some((
            NewActivity {
                kind: ActivityKind::Error,
                code: code::CONTACTS_FAILED.into(),
                detail: Some(error),
                ..Default::default()
            },
            true,
        ));
    }
    let changed: usize = report
        .books
        .iter()
        .map(|book| book.added + book.updated + book.removed)
        .sum();
    (changed > 0).then(|| {
        (
            NewActivity {
                kind: ActivityKind::Event,
                code: code::CONTACTS_COMPLETE.into(),
                count: Some(u32::try_from(changed).unwrap_or(u32::MAX)),
                ..Default::default()
            },
            false,
        )
    })
}

#[cfg(test)]
mod activity_tests {
    use super::*;
    use crate::domain::activity::{ActivityKind, code};

    #[test]
    fn a_late_success_for_an_update_preserves_a_queued_deletion() {
        let sent = SourceContact {
            key: "c1".into(), book: BookId::google(1), remote_id: Some("people/1".into()),
            sync: SyncState::Pending, ..Default::default()
        };
        let mut state = ContactsState::default();
        state.sources.push(sent.clone());
        queue_write(&mut state, &sent.key, WriteOp::Update);
        drop_source(&mut state, &sent.key);
        let remote = RemoteContact {
            remote_id: "people/1".into(), etag: "next".into(), fields: sent.fields.clone(),
            unmanaged: serde_json::Value::Null, deleted: false,
        };
        acknowledge_update(&mut state, &sent, &sent.fields, &remote);
        assert!(state.sources[0].deleted);
        assert_eq!(state.sources[0].sync, SyncState::Pending);
        assert_eq!(state.queue, vec![QueuedWrite { key: sent.key, op: WriteOp::Delete }]);
    }

    fn book(added: usize, updated: usize, removed: usize) -> BookSync {
        BookSync { book: "google:1".into(), added, updated, removed, ..Default::default() }
    }

    #[test]
    fn contacts_run_without_changes_is_not_logged() {
        let report = SyncReport { books: vec![book(0, 0, 0), book(0, 0, 0)], writes: vec![] };
        assert!(contacts_activity(&report).is_none());
        assert!(contacts_activity(&SyncReport::default()).is_none());
    }

    #[test]
    fn contacts_changes_are_counted() {
        let report = SyncReport { books: vec![book(2, 0, 0), book(0, 0, 1)], writes: vec![] };
        let (entry, failure) = contacts_activity(&report).unwrap();
        assert!(!failure);
        assert_eq!((entry.code.as_str(), entry.kind, entry.count), (code::CONTACTS_COMPLETE, ActivityKind::Event, Some(3)));
        assert_eq!(entry.account_id, None);
    }

    #[test]
    fn the_first_book_error_is_the_failure() {
        let failing = BookSync { error: Some("Google: quota".into()), ..book(5, 0, 0) };
        let (entry, failure) = contacts_activity(&SyncReport { books: vec![book(1, 0, 0), failing], writes: vec![] }).unwrap();
        assert!(failure, "an error wins over the changes of the other books");
        assert_eq!((entry.code.as_str(), entry.kind), (code::CONTACTS_FAILED, ActivityKind::Error));
        assert_eq!(entry.detail.as_deref(), Some("Google: quota"));
    }

    #[test]
    fn consent_required_books_are_not_failures() {
        let waiting = BookSync { consent_required: true, error: Some("Allow access".into()), ..book(0, 0, 0) };
        assert!(contacts_activity(&SyncReport { books: vec![waiting], writes: vec![] }).is_none());
    }
}
