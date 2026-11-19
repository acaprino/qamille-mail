//! Runtime boundary: OS region detection and attachment I/O stay outside domain rules.
use crate::domain::{
    model::{Attachment, Store},
    search::ParsedQuery,
};
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

/// Largest file read for text, as the compute server accepts it (D2).
const MAX_FILE_BYTES: usize = 25 * 1024 * 1024;

/// Text of a file known only by its bytes (a part inside an attached message).
/// Documents go to the compute server's parser process, exactly like saved attachments.
pub fn bytes_attachment_text(name: &str, bytes: &[u8]) -> Result<String, String> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err("File is larger than 25 MB.".into());
    }
    let text = super::attach::bytes_to_text(name, bytes)?;
    if text.len() > 4 * 1024 * 1024 {
        return Err("The attachment text is too long to show.".into());
    }
    Ok(text)
}

pub fn cached_attachment_text(attachment: &Attachment) -> Option<String> {
    static CACHE: OnceLock<Mutex<HashMap<String, Option<String>>>> = OnceLock::new();
    if let Some(text) = &attachment.inline_text {
        return Some(text.clone());
    }
    let path = attachment.path.as_ref()?;
    let metadata = std::fs::metadata(path).ok()?;
    let key = format!("{path}:{}:{:?}", metadata.len(), metadata.modified().ok());
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(cached) = cache.lock() {
        if let Some(text) = cached.get(&key) {
            return text.clone();
        }
    }
    let text = if metadata.len() <= MAX_FILE_BYTES as u64 {
        let Ok(bytes) = std::fs::read(path) else { return None };
        match super::attach::document_text(&attachment.name, &bytes) {
            Ok(text) => Some(text).filter(|text| text.len() <= 4 * 1024 * 1024),
            // The compute server is not there yet: try again next time, never remember it.
            Err(error) if crate::application::compute::is_transient(error.code) => return None,
            Err(_) => None,
        }
    } else { None };
    if let Ok(mut cached) = cache.lock() {
        let bytes = text.as_ref().map_or(0, String::len);
        let used: usize = cached
            .values()
            .filter_map(|v| v.as_ref())
            .map(String::len)
            .sum();
        if cached.len() >= 128 || used.saturating_add(bytes) > 32 * 1024 * 1024 {
            cached.clear();
        }
        if bytes <= 32 * 1024 * 1024 {
            cached.insert(key, text.clone());
        }
    }
    text
}

pub fn resolve_region(store: &mut Store) {
    store.settings.time_zone = super::timezone::resolved_timezone(&store.settings);
    store.settings.locale = super::timezone::resolved_locale(&store.settings);
}

/// Search can discard each extracted attachment before examining the next mail.
pub fn prepare_email(email: &crate::model::Email, parsed: &ParsedQuery) -> crate::model::Email {
    let mut email = email.clone();
    if !email.remote_only && parsed.needs_attachment_content() {
        let mut used = 0usize;
        for attachment in &mut email.attachments {
            if attachment.inline_text.is_none() && used < 8 * 1024 * 1024 {
                attachment.inline_text = cached_attachment_text(attachment);
            }
            used += attachment.inline_text.as_ref().map_or(0, String::len);
        }
    }
    email
}

/// Hydration is ephemeral: callers must not persist this prepared view.
pub fn prepare(
    store: &mut Store,
    text: &str,
    account_id: Option<u32>,
    now: i64,
) -> Result<(), String> {
    resolve_region(store);
    let parsed = ParsedQuery::parse_scoped(store, text, now, account_id)?;
    if parsed.needs_attachment_content() {
        for email in &mut store.emails {
            if account_id.is_some_and(|id| email.account_id != id) || email.remote_only {
                continue;
            }
            for attachment in &mut email.attachments {
                if attachment.inline_text.is_none() {
                    attachment.inline_text = cached_attachment_text(attachment);
                }
            }
        }
    }
    Ok(())
}

/// Validation needs resolved region, but never extracts attachment contents.
pub fn parse(
    store: &Store,
    text: &str,
    account_id: Option<u32>,
    now: i64,
) -> Result<ParsedQuery, String> {
    let mut prepared = store.clone();
    resolve_region(&mut prepared);
    ParsedQuery::parse_scoped(&prepared, text, now, account_id)
}

pub fn execute(
    store: &Store,
    text: &str,
    account_id: Option<u32>,
    now: i64,
    sort: &str,
) -> Result<Vec<u64>, String> {
    let mut prepared = store.clone();
    prepare(&mut prepared, text, account_id, now)?;
    crate::domain::search::execute(&prepared, text, account_id, now, sort)
}
