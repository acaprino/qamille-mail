//! Contacts use cases: the unified book, creation, edits, deletion and suggestions.
use super::{AppError, AppResult, ServiceContext, contact_ports::*};
use crate::domain::{
    contact_match::{MatchContext, certain_groups},
    contact_norm::{normalize_email, normalize_name, phone_tail, region_of},
    contact_view::{ObservedPerson, UnifiedContact, observed_people, unify},
    vcard::{VCardVersion, parse_vcards, write_vcard},
    contacts::*,
    model::{Account, Store},
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct OriginView {
    pub key: String,
    pub book: String,
    pub label: String,
    pub writable: bool,
    pub sync: SyncState,
    pub modified: i64,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ContactView {
    pub id: String,
    pub fields: ContactFields,
    pub origins: Vec<OriginView>,
    pub hidden: bool,
}

#[derive(Deserialize, Clone, Debug, Default)]
pub struct ContactQuery {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub book: Option<String>,
    #[serde(default)]
    pub include_hidden: bool,
    #[serde(default)]
    pub offset: usize,
    #[serde(default)]
    pub limit: usize,
}

#[derive(Serialize, Clone, Debug)]
pub struct ContactPage {
    pub items: Vec<ContactView>,
    pub total: usize,
    pub next_offset: Option<usize>,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct BookView {
    pub id: String,
    pub label: String,
    pub kind: BookKind,
    pub writable: bool,
    pub sync_enabled: bool,
    pub last_sync: Option<i64>,
    pub write_granted: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub name: String,
    pub address: String,
    pub books: Vec<String>,
}

#[derive(Clone)]
pub struct ContactService {
    pub(crate) context: Arc<ServiceContext>,
    pub(crate) store: Arc<dyn ContactStore>,
    pub(crate) gateway: Arc<dyn ContactGateway>,
    /// Remote writes of one user are sent one after another.
    pub(crate) flush_lock: Arc<Mutex<()>>,
    /// False for callers that must not see data that belongs to no account (restricted MCP profiles).
    pub(crate) include_local: bool,
}

/// Everything a read needs, taken once so one call sees one consistent book.
pub(crate) struct Snapshot {
    pub state: ContactsState,
    pub mail: Store,
    pub sources: Vec<SourceContact>,
    pub unified: Vec<UnifiedContact>,
    pub region: String,
}

pub(crate) fn book_label(book: &BookId, accounts: &[Account]) -> String {
    let email = |id: Option<u32>| {
        id.and_then(|id| accounts.iter().find(|a| a.id == id)).map(|a| a.email.clone()).unwrap_or_else(|| "?".into())
    };
    match book.kind() {
        BookKind::Local => "Locale".into(),
        BookKind::Google => format!("Google · {}", email(book.account_id())),
        BookKind::GoogleOther => format!("Google · altri · {}", email(book.account_id())),
        BookKind::Mail => format!("Posta · {}", email(book.account_id())),
    }
}

fn mail_source(person: &ObservedPerson) -> SourceContact {
    SourceContact {
        key: format!("mail:{}:{}", person.account_id, person.address),
        book: BookId::mail(person.account_id),
        fields: ContactFields {
            name: person.name.clone(),
            emails: vec![Labeled { label: String::new(), value: person.address.clone() }],
            ..Default::default()
        },
        ..Default::default()
    }
}

fn phone_key(value: &str) -> String {
    phone_tail(value).unwrap_or_else(|| value.chars().filter(|c| c.is_ascii_digit()).collect())
}
fn address_key(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// Applies to `target` only what changed between `old` and `new`.
fn apply_edit(old: &ContactFields, new: &ContactFields, target: &mut ContactFields) {
    if old.name != new.name {
        target.name = new.name.clone();
    }
    if old.org != new.org {
        target.org = new.org.clone();
    }
    if old.title != new.title {
        target.title = new.title.clone();
    }
    if old.birthday != new.birthday {
        target.birthday = new.birthday.clone();
    }
    if old.notes != new.notes {
        target.notes = new.notes.clone();
    }
    fn list(old: &[Labeled], new: &[Labeled], target: &mut Vec<Labeled>, key: fn(&str) -> String) {
        let keys = |items: &[Labeled]| -> Vec<String> { items.iter().map(|i| key(&i.value)).collect() };
        let (old_keys, new_keys) = (keys(old), keys(new));
        let removed: Vec<&String> = old_keys.iter().filter(|k| !new_keys.contains(k)).collect();
        target.retain(|item| !removed.contains(&&key(&item.value)));
        for item in new.iter().filter(|i| !old_keys.contains(&key(&i.value))) {
            if !target.iter().any(|t| key(&t.value) == key(&item.value)) {
                target.push(item.clone());
            }
        }
    }
    list(&old.emails, &new.emails, &mut target.emails, normalize_email);
    list(&old.phones, &new.phones, &mut target.phones, phone_key);
    list(&old.addresses, &new.addresses, &mut target.addresses, address_key);
}

fn matches_text(fields: &ContactFields, needle: &str) -> bool {
    let needle = needle.trim().to_lowercase();
    if needle.is_empty() {
        return true;
    }
    let digits: String = needle.chars().filter(|c| c.is_ascii_digit()).collect();
    let mut haystack = vec![fields.name.to_lowercase(), fields.org.to_lowercase(), fields.title.to_lowercase(), fields.notes.to_lowercase()];
    haystack.extend(fields.emails.iter().map(|e| e.value.to_lowercase()));
    haystack.extend(fields.addresses.iter().map(|a| a.value.to_lowercase()));
    haystack.extend(fields.phones.iter().map(|p| p.value.to_lowercase()));
    haystack.iter().any(|h| h.contains(&needle))
        || (digits.len() >= 3 && fields.phones.iter().any(|p| p.value.chars().filter(|c| c.is_ascii_digit()).collect::<String>().contains(&digits)))
}

/// Queues a remote write unless an equal or stronger one is already waiting.
pub(crate) fn queue_write(state: &mut ContactsState, key: &str, op: WriteOp) {
    let covered = state.queue.iter().any(|q| q.key == key && (q.op == op || (op == WriteOp::Update && q.op == WriteOp::Create)));
    if !covered {
        state.queue.push(QueuedWrite { key: key.to_string(), op });
    }
}

/// A fresh id and the card key derived from it, skipping keys already in use.
pub(crate) fn new_source_key(state: &mut ContactsState) -> (u64, String) {
    loop {
        let id = state.new_id();
        let key = format!("c{id}");
        if !state.sources.iter().any(|s| s.key == key) {
            return (id, key);
        }
    }
}

/// Removes a card: Google cards already sent become a tombstone until the server confirms.
pub(crate) fn drop_source(state: &mut ContactsState, key: &str) {
    let Some(position) = state.sources.iter().position(|s| s.key == key) else { return };
    let card = &state.sources[position];
    let remote = card.book.kind() == BookKind::Google && (card.remote_id.is_some() || card.create_request.is_some());
    state.queue.retain(|q| q.key != key);
    if remote {
        state.sources[position].deleted = true;
        state.sources[position].sync = SyncState::Pending;
        state.queue.push(QueuedWrite { key: key.to_string(), op: WriteOp::Delete });
    } else {
        state.sources.remove(position);
    }
    state.hidden.retain(|h| h != key);
    for link in &mut state.links {
        link.keys.retain(|k| k != key);
    }
    state.links.retain(|l| l.keys.len() >= 2);
}

impl ContactService {
    pub fn new(context: Arc<ServiceContext>, store: Arc<dyn ContactStore>, gateway: Arc<dyn ContactGateway>) -> Self {
        Self { context, store, gateway, flush_lock: Arc::new(Mutex::new(())), include_local: true }
    }

    /// A view without the local book, for callers limited to some accounts.
    pub fn without_local(mut self) -> Self {
        self.include_local = false;
        self
    }

    pub(crate) fn gmail_account(&self, book: &BookId) -> AppResult<Account> {
        let mail = self.context.read()?;
        book.account_id()
            .and_then(|id| mail.account(id))
            .filter(|a| a.resolved_provider_id() == "gmail")
            .cloned()
            .ok_or_else(|| AppError::validation("This address book belongs to an account that is not connected to Google"))
    }

    /// Asks for write permission on each Google book that lacks it. A refusal stops the caller before anything is saved.
    pub(crate) fn ensure_write(&self, books: &[BookId]) -> AppResult<()> {
        let granted = self.store.read()?.settings.write_granted;
        for book in books.iter().filter(|b| b.kind() == BookKind::Google && !granted.contains(&b.0)) {
            let account = self.gmail_account(book)?;
            self.gateway.authorize(&account, true).map_err(|e| AppError::new("consent_required", e.message))?;
            self.update_state(|state| {
                if !state.settings.write_granted.contains(&book.0) {
                    state.settings.write_granted.push(book.0.clone());
                }
                Ok(())
            })?;
        }
        Ok(())
    }

    pub(crate) fn snapshot(&self) -> AppResult<Snapshot> {
        let mail = self.context.read()?;
        let state = self.store.read()?;
        let live = |book: &BookId| book.account_id().is_none_or(|id| mail.account(id).is_some());
        let mut sources: Vec<SourceContact> = state
            .sources
            .iter()
            .filter(|s| !s.deleted && live(&s.book) && (self.include_local || s.book != BookId::local()))
            .cloned()
            .collect();
        sources.extend(observed_people(&mail.emails).iter().filter(|p| mail.account(p.account_id).is_some()).map(mail_source));
        let region = region_of(&self.context.clock.regional(&mail.settings).locale);
        let cx = MatchContext { region: &region, roles: &state.settings.roles, rejected: &state.rejected, links: &state.links };
        let unified = unify(&sources, &state.links, &state.hidden, &state.settings.priority, &cx);
        Ok(Snapshot { state, mail, sources, unified, region })
    }

    pub(crate) fn view(&self, snapshot: &Snapshot, contact: &UnifiedContact) -> ContactView {
        ContactView {
            id: contact.id.clone(),
            fields: contact.fields.clone(),
            hidden: contact.hidden,
            origins: contact
                .origins
                .iter()
                .map(|o| OriginView {
                    key: o.key.clone(),
                    book: o.book.0.clone(),
                    label: book_label(&o.book, &snapshot.mail.accounts),
                    writable: o.writable,
                    sync: o.sync.clone(),
                    modified: o.modified,
                })
                .collect(),
        }
    }

    fn find<'a>(snapshot: &'a Snapshot, id: &str) -> AppResult<&'a UnifiedContact> {
        snapshot
            .unified
            .iter()
            .find(|c| c.id == id || c.origins.iter().any(|o| o.key == id))
            .ok_or_else(|| AppError::missing("Contact"))
    }

    pub fn list(&self, query: ContactQuery) -> AppResult<ContactPage> {
        let snapshot = self.snapshot()?;
        let matching: Vec<ContactView> = snapshot
            .unified
            .iter()
            .filter(|c| query.include_hidden || !c.hidden)
            .filter(|c| query.book.as_ref().is_none_or(|book| c.origins.iter().any(|o| &o.book.0 == book)))
            .filter(|c| matches_text(&c.fields, &query.text))
            .map(|c| self.view(&snapshot, c))
            .collect();
        let total = matching.len();
        let limit = if query.limit == 0 { 60 } else { query.limit };
        let end = (query.offset + limit).min(total);
        let items = matching.into_iter().skip(query.offset).take(limit).collect();
        Ok(ContactPage { items, total, next_offset: (end < total).then_some(end) })
    }

    pub fn get(&self, id: &str) -> AppResult<ContactView> {
        let snapshot = self.snapshot()?;
        Ok(self.view(&snapshot, Self::find(&snapshot, id)?))
    }

    pub fn books(&self) -> AppResult<Vec<BookView>> {
        let mail = self.context.read()?;
        let state = self.store.read()?;
        let book = |id: BookId, kind: BookKind, writable: bool, granted: bool| BookView {
            label: book_label(&id, &mail.accounts),
            sync_enabled: !state.settings.sync_off.contains(&id.0),
            last_sync: state.synced_at.get(&id.0).copied(),
            id: id.0,
            kind,
            writable,
            write_granted: granted,
        };
        let mut books = if self.include_local { vec![book(BookId::local(), BookKind::Local, true, true)] } else { Vec::new() };
        for account in &mail.accounts {
            if account.resolved_provider_id() == "gmail" {
                let google = BookId::google(account.id);
                let granted = state.settings.write_granted.contains(&google.0);
                books.push(book(google, BookKind::Google, true, granted));
                books.push(book(BookId::google_other(account.id), BookKind::GoogleOther, false, false));
            }
            books.push(book(BookId::mail(account.id), BookKind::Mail, false, false));
        }
        Ok(books)
    }

    pub fn similar_to(&self, fields: &ContactFields) -> AppResult<Vec<ContactView>> {
        let snapshot = self.snapshot()?;
        let probe = SourceContact { key: "\u{0}probe".into(), book: BookId::local(), fields: fields.clone(), ..Default::default() };
        let mut all = snapshot.sources.clone();
        all.push(probe);
        let cx = MatchContext { region: &snapshot.region, roles: &snapshot.state.settings.roles, rejected: &[], links: &[] };
        let mut keys: Vec<String> = certain_groups(&all, &cx)
            .into_iter()
            .filter(|g| g.iter().any(|k| k == "\u{0}probe"))
            .flatten()
            .filter(|k| k != "\u{0}probe")
            .collect();
        let name = normalize_name(&fields.name);
        let mut found: Vec<&UnifiedContact> = Vec::new();
        for contact in &snapshot.unified {
            let same_name = !name.is_empty() && normalize_name(&contact.fields.name) == name;
            let shares_key = contact.origins.iter().any(|o| keys.contains(&o.key));
            if (same_name || shares_key) && !found.iter().any(|f| f.id == contact.id) {
                found.push(contact);
            }
        }
        keys.clear();
        Ok(found.into_iter().map(|c| self.view(&snapshot, c)).collect())
    }

    pub fn create(&self, book: &str, fields: ContactFields) -> AppResult<ContactView> {
        if book.trim().is_empty() {
            return Err(AppError::new("book_required", "Choose an address book"));
        }
        let book_id = BookId(book.trim().to_string());
        let mail = self.context.read()?;
        let usable = match book_id.kind() {
            BookKind::Local => book_id.writable() && self.include_local,
            BookKind::Google => book_id.account_id().and_then(|id| mail.account(id)).is_some_and(|a| a.resolved_provider_id() == "gmail"),
            _ => false,
        };
        if !usable {
            return Err(AppError::validation("This address book cannot receive new contacts"));
        }
        let blank = |s: &str| s.trim().is_empty();
        if blank(&fields.name) && fields.emails.iter().all(|e| blank(&e.value)) && fields.phones.iter().all(|p| blank(&p.value)) {
            return Err(AppError::validation("Add a name, an email address or a phone number"));
        }
        self.ensure_write(&[book_id.clone()])?;
        let now = self.context.clock.now();
        let key = self.update_state(|state| {
            let (id, key) = new_source_key(state);
            let remote = book_id.kind() == BookKind::Google;
            state.sources.push(SourceContact {
                key: key.clone(),
                book: book_id.clone(),
                uid: Some(format!("qamille-{id}-{now}")),
                fields: fields.clone(),
                modified: now,
                sync: if remote { SyncState::Pending } else { SyncState::Synced },
                ..Default::default()
            });
            if remote {
                state.queue.push(QueuedWrite { key: key.clone(), op: WriteOp::Create });
            }
            state.settings.last_book = Some(book_id.0.clone());
            Ok(key)
        })?;
        self.get(&key)
    }

    pub fn prefill(&self, id: &str) -> AppResult<ContactFields> {
        Ok(self.get(id)?.fields)
    }

    pub fn update_targets(&self, id: &str) -> AppResult<Vec<BookView>> {
        let contact = self.get(id)?;
        let writable: Vec<String> = contact.origins.iter().filter(|o| o.writable).map(|o| o.book.clone()).collect();
        Ok(self.books()?.into_iter().filter(|b| writable.contains(&b.id)).collect())
    }

    pub fn update(&self, id: &str, fields: ContactFields) -> AppResult<ContactView> {
        let snapshot = self.snapshot()?;
        let contact = Self::find(&snapshot, id)?;
        let keys: Vec<String> = contact.origins.iter().filter(|o| o.writable).map(|o| o.key.clone()).collect();
        if keys.is_empty() {
            return Err(AppError::validation("This contact is only seen in mail. Add it to an address book first"));
        }
        let (old, anchor) = (contact.fields.clone(), contact.id.clone());
        if let Some(origin) = contact.origins.iter().find(|o| o.writable && o.sync == SyncState::Conflict) {
            return Err(AppError::new(
                "conflict_pending",
                format!("Resolve the conflict on {} before editing this contact", book_label(&origin.book, &snapshot.mail.accounts)),
            ));
        }
        let books: Vec<BookId> = contact.origins.iter().filter(|o| o.writable).map(|o| o.book.clone()).collect();
        self.ensure_write(&books)?;
        let now = self.context.clock.now();
        self.update_state(|state| {
            for key in &keys {
                let Some(position) = state.sources.iter().position(|s| &s.key == key) else { continue };
                let remote = state.sources[position].book.kind() == BookKind::Google;
                apply_edit(&old, &fields, &mut state.sources[position].fields);
                state.sources[position].modified = now;
                if remote {
                    state.sources[position].sync = SyncState::Pending;
                    queue_write(state, key, WriteOp::Update);
                }
            }
            Ok(())
        })?;
        self.get(&anchor)
    }

    pub fn delete(&self, id: &str, books: Vec<String>) -> AppResult<()> {
        let snapshot = self.snapshot()?;
        let contact = Self::find(&snapshot, id)?;
        let mut keys = Vec::new();
        for book in &books {
            if !BookId(book.clone()).writable() {
                return Err(AppError::validation("Cards seen in mail cannot be deleted; hide them instead"));
            }
            let found: Vec<&str> = contact.origins.iter().filter(|o| &o.book.0 == book).map(|o| o.key.as_str()).collect();
            if found.is_empty() {
                return Err(AppError::validation("The contact is not in that address book"));
            }
            keys.extend(found.into_iter().map(String::from));
        }
        self.ensure_write(&books.iter().map(|b| BookId(b.clone())).collect::<Vec<_>>())?;
        self.update_state(|state| {
            for key in &keys {
                drop_source(state, key);
            }
            Ok(())
        })
    }

    pub fn hide(&self, id: &str, hidden: bool) -> AppResult<()> {
        let snapshot = self.snapshot()?;
        let keys: Vec<String> = Self::find(&snapshot, id)?.origins.iter().map(|o| o.key.clone()).collect();
        self.update_state(|state| {
            state.hidden.retain(|h| !keys.contains(h));
            if hidden {
                state.hidden.extend(keys.clone());
            }
            Ok(())
        })
    }

    pub fn suggest(&self, text: &str, limit: usize) -> AppResult<Vec<Suggestion>> {
        let needle = text.trim().to_lowercase();
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        let snapshot = self.snapshot()?;
        let mut found: Vec<(bool, String, Suggestion)> = Vec::new();
        for contact in snapshot.unified.iter().filter(|c| !c.hidden) {
            let name = contact.fields.name.trim().to_string();
            let books: Vec<String> = contact.origins.iter().map(|o| o.book.0.clone()).collect();
            for email in &contact.fields.emails {
                let address = email.value.trim().to_string();
                let lowered = address.to_lowercase();
                if address.is_empty() || !format!("{} {}", name.to_lowercase(), lowered).contains(&needle) {
                    continue;
                }
                if found.iter().any(|(_, _, s)| s.address.to_lowercase() == lowered) {
                    continue;
                }
                let prefix = lowered.starts_with(&needle) || name.to_lowercase().split_whitespace().any(|w| w.starts_with(&needle));
                found.push((!prefix, name.to_lowercase(), Suggestion { name: name.clone(), address, books: books.clone() }));
            }
        }
        found.sort_by(|a, b| (a.0, &a.1, &a.2.address).cmp(&(b.0, &b.1, &b.2.address)));
        Ok(found.into_iter().take(limit.max(1)).map(|(_, _, s)| s).collect())
    }

    /// One atomic change of the contacts file.
    pub(crate) fn update_state<T>(&self, operation: impl FnOnce(&mut ContactsState) -> AppResult<T>) -> AppResult<T> {
        let mut operation = Some(operation);
        let mut result = None;
        self.store.transaction(&mut |state| {
            let run = operation.take().ok_or_else(|| AppError::storage("Contacts transaction ran twice"))?;
            result = Some(run(state)?);
            Ok(())
        })?;
        result.ok_or_else(|| AppError::storage("Contacts transaction did not run"))
    }
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ImportReport {
    pub added: usize,
    pub skipped: usize,
}

fn identity(fields: &ContactFields) -> (String, Vec<String>, Vec<String>, String) {
    let mut emails: Vec<String> = fields.emails.iter().map(|e| normalize_email(&e.value)).collect();
    let mut phones: Vec<String> = fields.phones.iter().map(|p| phone_key(&p.value)).collect();
    emails.sort();
    phones.sort();
    (normalize_name(&fields.name), emails, phones, normalize_name(&fields.org))
}

impl ContactService {
    /// Adds cards from a `.vcf` text to the local book; cards already there are skipped.
    pub fn import_vcf(&self, text: &str) -> AppResult<ImportReport> {
        let cards = parse_vcards(text);
        let now = self.context.clock.now();
        self.update_state(|state| {
            let mut report = ImportReport { added: 0, skipped: 0 };
            for card in &cards {
                let blank = |s: &str| s.trim().is_empty();
                let empty = blank(&card.fields.name) && card.fields.emails.iter().all(|e| blank(&e.value)) && card.fields.phones.iter().all(|p| blank(&p.value));
                let wanted = identity(&card.fields);
                let known = state.sources.iter().any(|s| {
                    s.book == BookId::local()
                        && !s.deleted
                        && (identity(&s.fields) == wanted || (card.uid.is_some() && s.uid == card.uid))
                });
                if empty || known {
                    report.skipped += 1;
                    continue;
                }
                let (id, key) = new_source_key(state);
                state.sources.push(SourceContact {
                    key,
                    book: BookId::local(),
                    uid: card.uid.clone().or_else(|| Some(format!("qamille-{id}-{now}"))),
                    fields: card.fields.clone(),
                    modified: now,
                    unmanaged: if card.unmanaged.is_empty() { serde_json::Value::Null } else { serde_json::json!({ "vcard": card.unmanaged }) },
                    ..Default::default()
                });
                report.added += 1;
            }
            Ok(report)
        })
    }

    /// vCard 3.0 text of every card in a writable book.
    pub fn export_vcf(&self, book: &str) -> AppResult<String> {
        let id = BookId(book.to_string());
        if !id.writable() || (id.kind() == BookKind::Local && book != "local") {
            return Err(AppError::validation("Only the local and Google address books can be exported"));
        }
        let state = self.store.read()?;
        let mut out = String::new();
        for card in state.sources.iter().filter(|s| s.book == id && !s.deleted) {
            let extra: Vec<String> = card.unmanaged["vcard"]
                .as_array()
                .map(|lines| lines.iter().filter_map(|l| l.as_str().map(String::from)).collect())
                .unwrap_or_default();
            // A stable identifier across exports: it keeps the prefix from before the
            // rebranding, so re-importing an older export updates the same contacts.
            let uid = card.uid.clone().unwrap_or_else(|| format!("sparklone-{}", card.key));
            out.push_str(&write_vcard(&uid, &card.fields, &extra, VCardVersion::V3));
        }
        Ok(out)
    }
}
