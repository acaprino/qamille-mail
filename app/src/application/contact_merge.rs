//! Duplicate answers: Stessa persona, Persone diverse and Unisci…, with undo.
use super::{AppError, AppResult, contacts::*};
use crate::domain::{
    contact_match::{MatchContext, find_duplicates},
    contact_merge::merge_fields,
    contacts::*,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

const JOURNAL_DAYS: i64 = 30;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct MemberView {
    pub key: String,
    pub book: String,
    pub label: String,
    pub writable: bool,
    pub fields: ContactFields,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct DuplicateView {
    pub id: String,
    pub level: Level,
    pub members: Vec<MemberView>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub enum MergeAction {
    Create,
    Update,
    Delete,
    Unavailable(String),
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct BookAction {
    pub book: String,
    pub label: String,
    pub key: Option<String>,
    pub action: MergeAction,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct MergePreview {
    pub result: ContactFields,
    pub actions: Vec<BookAction>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct MergeOutcome {
    pub journal_id: u64,
    pub per_book: Vec<BookOutcome>,
    /// After an undo: names of the Google cards that came back as new copies (new remote id).
    pub recreated: Vec<String>,
}

fn sorted_pair(a: &str, b: &str) -> (String, String) {
    if a <= b { (a.into(), b.into()) } else { (b.into(), a.into()) }
}

/// Joins `keys` into one link, absorbing every link they touch.
fn join_link(state: &mut ContactsState, keys: &[String]) {
    let mut joined: BTreeSet<String> = keys.iter().cloned().collect();
    let mut pins = BTreeMap::new();
    state.links.retain(|link| {
        if link.keys.iter().any(|k| joined.contains(k)) {
            joined.extend(link.keys.iter().cloned());
            pins.extend(link.pins.clone());
            false
        } else {
            true
        }
    });
    if joined.len() >= 2 {
        state.links.push(Link { keys: joined.into_iter().collect(), pins });
    }
}

fn leave_links(state: &mut ContactsState, keys: &[String]) {
    for link in &mut state.links {
        link.keys.retain(|k| !keys.contains(k));
    }
    state.links.retain(|l| l.keys.len() >= 2);
}

impl ContactService {
    pub fn duplicates(&self, include_possible: bool) -> AppResult<Vec<DuplicateView>> {
        let snapshot = self.snapshot()?;
        let state = &snapshot.state;
        let cx = MatchContext { region: &snapshot.region, roles: &state.settings.roles, rejected: &state.rejected, links: &state.links };
        let groups = find_duplicates(&snapshot.sources, &cx);
        Ok(groups
            .into_iter()
            .filter(|g| include_possible || g.level != Level::Possible)
            .map(|g| DuplicateView {
                id: g.keys.join("|"),
                level: g.level,
                members: g
                    .keys
                    .iter()
                    .filter_map(|key| snapshot.sources.iter().find(|s| &s.key == key))
                    .map(|s| MemberView {
                        key: s.key.clone(),
                        book: s.book.0.clone(),
                        label: book_label(&s.book, &snapshot.mail.accounts),
                        writable: s.book.writable(),
                        fields: s.fields.clone(),
                    })
                    .collect(),
            })
            .collect())
    }

    fn require_cards(&self, keys: &[String]) -> AppResult<()> {
        let snapshot = self.snapshot()?;
        if keys.len() < 2 {
            return Err(AppError::validation("Select at least two cards"));
        }
        if keys.iter().any(|k| !snapshot.sources.iter().any(|s| &s.key == k)) {
            return Err(AppError::missing("Contact"));
        }
        Ok(())
    }

    /// The cards are one person from now on. No address book is written.
    pub fn same_person(&self, keys: Vec<String>) -> AppResult<()> {
        self.require_cards(&keys)?;
        self.update_state(|state| {
            state.rejected.retain(|(a, b)| !(keys.contains(a) && keys.contains(b)));
            join_link(state, &keys);
            Ok(())
        })
    }

    pub fn unlink(&self, keys: Vec<String>) -> AppResult<()> {
        self.update_state(|state| {
            leave_links(state, &keys);
            Ok(())
        })
    }

    /// The cards are different people and are never proposed together again.
    pub fn different_people(&self, keys: Vec<String>) -> AppResult<()> {
        self.require_cards(&keys)?;
        self.update_state(|state| {
            leave_links(state, &keys);
            for (i, a) in keys.iter().enumerate() {
                for b in &keys[i + 1..] {
                    let pair = sorted_pair(a, b);
                    if !state.rejected.contains(&pair) {
                        state.rejected.push(pair);
                    }
                }
            }
            Ok(())
        })
    }

    fn plan(&self, snapshot: &Snapshot, keys: &[String], keep: &[String]) -> AppResult<(Vec<SourceContact>, Vec<BookAction>)> {
        if keys.len() < 2 {
            return Err(AppError::validation("Select at least two cards"));
        }
        let mut members = Vec::new();
        for key in keys {
            members.push(snapshot.sources.iter().find(|s| &s.key == key).cloned().ok_or_else(|| AppError::missing("Contact"))?);
        }
        if keep.is_empty() {
            return Err(AppError::validation("Choose at least one address book"));
        }
        for book in keep {
            let id = BookId(book.clone());
            let usable = match id.kind() {
                BookKind::Local => id.writable(),
                BookKind::Google => id.account_id().and_then(|a| snapshot.mail.account(a)).is_some_and(|a| a.resolved_provider_id() == "gmail"),
                _ => false,
            };
            if !usable {
                return Err(AppError::validation("This address book cannot receive contacts"));
            }
        }
        let mut involved: BTreeSet<String> = keep.iter().cloned().collect();
        involved.extend(members.iter().filter(|m| m.book.writable()).map(|m| m.book.0.clone()));
        let mut actions = Vec::new();
        for book in involved {
            let id = BookId(book.clone());
            let label = book_label(&id, &snapshot.mail.accounts);
            let copies: Vec<&SourceContact> = members.iter().filter(|m| m.book == id && m.book.writable()).collect();
            let consented = id.kind() == BookKind::Local || snapshot.state.settings.write_granted.contains(&book);
            let action = |key: Option<&str>, action: MergeAction| BookAction { book: book.clone(), label: label.clone(), key: key.map(String::from), action };
            if !consented {
                actions.push(action(None, MergeAction::Unavailable("Write permission not granted for this address book".into())));
            } else if keep.contains(&book) {
                match copies.split_first() {
                    None => actions.push(action(None, MergeAction::Create)),
                    Some(_) if copies.iter().any(|c| c.sync == SyncState::Conflict) => {
                        actions.push(action(None, MergeAction::Unavailable("Resolve the conflict on this contact first".into())));
                    }
                    Some((first, rest)) => {
                        actions.push(action(Some(&first.key), MergeAction::Update));
                        actions.extend(rest.iter().map(|c| action(Some(&c.key), MergeAction::Delete)));
                    }
                }
            } else {
                actions.extend(copies.iter().map(|c| action(Some(&c.key), MergeAction::Delete)));
            }
        }
        Ok((members, actions))
    }

    pub fn merge_preview(&self, keys: Vec<String>, keep_books: Vec<String>) -> AppResult<MergePreview> {
        let snapshot = self.snapshot()?;
        let (members, actions) = self.plan(&snapshot, &keys, &keep_books)?;
        let mut pins = BTreeMap::new();
        for link in snapshot.state.links.iter().filter(|l| l.keys.iter().any(|k| keys.contains(k))) {
            pins.extend(link.pins.clone());
        }
        let refs: Vec<&SourceContact> = members.iter().collect();
        Ok(MergePreview { result: merge_fields(&refs, &snapshot.state.settings.priority, &pins), actions })
    }

    pub fn merge_apply(&self, keys: Vec<String>, keep_books: Vec<String>, fields: ContactFields) -> AppResult<MergeOutcome> {
        let snapshot = self.snapshot()?;
        let (members, actions) = self.plan(&snapshot, &keys, &keep_books)?;
        let now = self.context.clock.now();
        let (mut outcome, touched) = self.update_state(|state| {
            let journal_id = state.new_id();
            let links_before: Vec<Link> = state.links.iter().filter(|l| l.keys.iter().any(|k| keys.contains(k))).cloned().collect();
            let touched: Vec<&str> = actions.iter().filter(|a| matches!(a.action, MergeAction::Update | MergeAction::Delete)).filter_map(|a| a.key.as_deref()).collect();
            let before: Vec<SourceContact> = state.sources.iter().filter(|s| touched.contains(&s.key.as_str())).cloned().collect();
            let mut created = Vec::new();
            let mut kept: Vec<String> = members.iter().filter(|m| !m.book.writable()).map(|m| m.key.clone()).collect();
            let mut results: BTreeMap<String, (bool, Option<String>)> = BTreeMap::new();
            for action in &actions {
                let remote = BookId(action.book.clone()).kind() == BookKind::Google;
                let outcome = results.entry(action.book.clone()).or_insert((true, None));
                match &action.action {
                    MergeAction::Unavailable(reason) => *outcome = (false, Some(reason.clone())),
                    MergeAction::Create => {
                        let (id, key) = new_source_key(state);
                        state.sources.push(SourceContact {
                            key: key.clone(),
                            book: BookId(action.book.clone()),
                            uid: Some(format!("qamille-{id}-{now}")),
                            fields: fields.clone(),
                            modified: now,
                            sync: if remote { SyncState::Pending } else { SyncState::Synced },
                            ..Default::default()
                        });
                        if remote {
                            queue_write(state, &key, WriteOp::Create);
                        }
                        created.push(key.clone());
                        kept.push(key);
                    }
                    MergeAction::Update => {
                        let key = action.key.clone().unwrap_or_default();
                        if let Some(card) = state.sources.iter_mut().find(|s| s.key == key) {
                            card.fields = fields.clone();
                            card.modified = now;
                            if remote {
                                card.sync = SyncState::Pending;
                            }
                        }
                        if remote {
                            queue_write(state, &key, WriteOp::Update);
                        }
                        kept.push(key);
                    }
                    MergeAction::Delete => drop_source(state, action.key.as_deref().unwrap_or_default()),
                }
            }
            state.rejected.retain(|(a, b)| !(keys.contains(a) && keys.contains(b)));
            join_link(state, &kept);
            let outcome: Vec<BookOutcome> = results.into_iter().map(|(book, (ok, error))| BookOutcome { book, ok, error }).collect();
            state.journal.retain(|j| j.at >= now - JOURNAL_DAYS * 86_400);
            let touched: Vec<String> = kept
                .iter()
                .cloned()
                .chain(actions.iter().filter(|a| a.action == MergeAction::Delete).filter_map(|a| a.key.clone()))
                .collect();
            state.journal.push(MergeJournal { id: journal_id, at: now, before, created, link_keys: kept, links_before, outcome: outcome.clone() });
            Ok((MergeOutcome { journal_id, per_book: outcome, recreated: Vec::new() }, touched))
        })?;
        // Google writes are sent now so each book reports its own result; failures stay queued.
        let writes_google = actions.iter().any(|a| BookId(a.book.clone()).kind() == BookKind::Google && !matches!(a.action, MergeAction::Unavailable(_)));
        if writes_google {
            for write in self.flush_keys(&touched)?.into_iter().filter(|w| !w.ok) {
                match outcome.per_book.iter_mut().find(|o| o.book == write.book) {
                    Some(existing) => {
                        existing.ok = false;
                        existing.error = write.error;
                    }
                    None => outcome.per_book.push(write),
                }
            }
        }
        Ok(outcome)
    }

    pub fn merge_undo(&self, journal_id: u64) -> AppResult<MergeOutcome> {
        self.update_state(|state| {
            let position = state.journal.iter().position(|j| j.id == journal_id).ok_or_else(|| AppError::missing("Merge history"))?;
            let journal = state.journal.remove(position);
            if let Some(card) = journal.before.iter().find(|b| state.sources.iter().any(|s| s.key == b.key && s.sync == SyncState::Conflict)) {
                return Err(AppError::new("conflict_pending", format!("Resolve the conflict on {} before undoing this merge", card.fields.name)));
            }
            for key in &journal.created {
                drop_source(state, key);
            }
            let mut recreated = Vec::new();
            for card in &journal.before {
                let remote = card.book.kind() == BookKind::Google;
                match state.sources.iter().position(|s| s.key == card.key) {
                    Some(at) if state.sources[at].deleted => {
                        state.queue.retain(|q| !(q.key == card.key && q.op == WriteOp::Delete));
                        state.sources[at].deleted = false;
                        state.sources[at].sync = SyncState::Synced;
                    }
                    Some(at) => {
                        state.sources[at].fields = card.fields.clone();
                        state.sources[at].modified = card.modified;
                        if remote {
                            state.sources[at].sync = SyncState::Pending;
                            queue_write(state, &card.key, WriteOp::Update);
                        }
                    }
                    None => {
                        let mut restored = card.clone();
                        if remote {
                            // The server already lost it: it comes back as a new card.
                            restored.remote_id = None;
                            restored.etag = None;
                            restored.base = None;
                            restored.sync = SyncState::Pending;
                            queue_write(state, &card.key, WriteOp::Create);
                            recreated.push(card.fields.name.clone());
                        }
                        state.sources.push(restored);
                    }
                }
            }
            leave_links(state, &journal.link_keys);
            for link in &journal.links_before {
                let live: Vec<String> = link.keys.iter().filter(|k| state.sources.iter().any(|s| &s.key == *k)).cloned().collect();
                if live.len() >= 2 {
                    join_link(state, &live);
                    if let Some(restored) = state.links.iter_mut().find(|l| l.keys.contains(&live[0])) {
                        for (field, value) in &link.pins {
                            restored.pins.entry(field.clone()).or_insert_with(|| value.clone());
                        }
                    }
                }
            }
            let mut books: Vec<String> = journal.before.iter().map(|c| c.book.0.clone()).chain(journal.created.iter().filter_map(|k| journal.outcome.iter().find(|o| &o.book == k).map(|o| o.book.clone()))).collect();
            books.extend(journal.outcome.iter().map(|o| o.book.clone()));
            books.sort();
            books.dedup();
            Ok(MergeOutcome { journal_id, per_book: books.into_iter().map(|book| BookOutcome { book, ok: true, error: None }).collect(), recreated })
        })
    }
}
