//! Per-message records bound incremental writes to one document, not a mailbox.
use super::rag::{self, Index, Passage, PassageReference};
use crate::{application::{AppError, AppResult, compute::{self, ComputeGateway}, indexing_ports::*}, model::{Email, Store}};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, path::{Path, PathBuf}, sync::atomic::{AtomicBool, Ordering}};

fn path(root: &Path, account: u32, email: u64) -> PathBuf {
    root.join("search-index").join(account.to_string()).join(format!("{email}.bin"))
}
pub fn files(root: &Path) -> std::io::Result<Vec<(u32, u64, PathBuf)>> {
    let mut paths = Vec::new();
    let folders = match std::fs::read_dir(root.join("search-index")) {
        Ok(folders) => folders,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(paths),
        Err(error) => return Err(error),
    };
    for folder in folders {
        let folder = folder?;
        if !folder.file_type()?.is_dir() { continue; }
        let Ok(account) = folder.file_name().to_string_lossy().parse::<u32>() else { continue };
        for file in std::fs::read_dir(folder.path())? {
            let file = file?;
            if !file.file_type()?.is_file() { continue; }
            let name = file.file_name();
            let name = name.to_string_lossy();
            let Some(id) = name.strip_suffix(".bin").and_then(|id| id.parse::<u64>().ok()) else { continue };
            paths.push((account, id, file.path()));
        }
    }
    Ok(paths)
}
pub fn message_bytes(root: &Path) -> std::io::Result<HashMap<u64, u64>> {
    let mut sizes = HashMap::new();
    for (_, id, path) in files(root)? { *sizes.entry(id).or_default() += std::fs::metadata(path)?.len(); }
    Ok(sizes)
}
pub fn reconcile(root: &Path, store: &Store) -> std::io::Result<()> {
    for (account, id, path) in files(root)? {
        if !store.email(id).is_some_and(|mail| mail.account_id == account && !mail.storage_evicted) {
            std::fs::remove_file(path)?;
        }
    }
    Ok(())
}
pub fn source_text(email: &Email) -> String {
    format!("Subject: {}\nFrom: {} <{}>\nTo: {}\nCc: {}\nDate: {}\nAttachments: {}\n\n{}",
        email.subject, email.from_name, email.from_addr, email.to.join(", "), email.cc.join(", "), email.date,
        email.attachments.iter().map(|attachment| attachment.name.as_str()).collect::<Vec<_>>().join(", "), email.body)
}
/// A pause says nothing about the message, so it must never use up its attempts (INV-3). A
/// device the server no longer knows (`401`, A8) and a server with another model (D4) stop the
/// account's indexing without touching the message either.
fn embed_error(error: AppError) -> AppError {
    if compute::is_transient(error.code) {
        AppError::new("index_transient", error.message)
    } else if error.code == compute::codes::UNAUTHORIZED {
        AppError::new(COMPUTE_UNAUTHORIZED, error.message)
    } else if error.code == compute::codes::MODEL_MISMATCH {
        error
    } else {
        AppError::external(error.message)
    }
}
/// The indexing code of a `401` from the compute server after sign-in.
pub const COMPUTE_UNAUTHORIZED: &str = "compute_unauthorized";

/// The model of the vectors already in the index (D4): the first file of passages found; files
/// older than D4 count as the local model. `None`: no vectors yet.
pub fn index_identity(root: &Path) -> Option<compute::EmbeddingIdentity> {
    let folders = std::fs::read_dir(root.join("search-index")).ok()?;
    for folder in folders.flatten() {
        if !folder.file_type().is_ok_and(|kind| kind.is_dir()) { continue; }
        let Ok(files) = std::fs::read_dir(folder.path()) else { continue };
        for file in files.flatten() {
            if !file.file_name().to_string_lossy().ends_with(".bin") { continue; }
            if let Ok(Some(identity)) = rag::read_identity(&file.path()) { return Some(identity); }
        }
    }
    // The mailbox index of older versions holds vectors of the local model.
    rag::read_identity(&root.join("rag_index.bin")).ok().flatten()
}

/// The model the server declares, refused when the index holds vectors of another (D4). The
/// errors are the gateway's own codes.
pub fn checked_identity(compute: &dyn ComputeGateway, root: &Path) -> AppResult<compute::EmbeddingIdentity> {
    let server = compute.health()?.embedding_identity();
    match index_identity(root) {
        Some(index) if !index.same_vectors(&server) => Err(AppError::new(compute::codes::MODEL_MISMATCH, compute::MODEL_MISMATCH_MESSAGE)),
        _ => Ok(server),
    }
}

/// [`checked_identity`] with the codes of indexing.
fn server_identity(compute: &dyn ComputeGateway, root: &Path) -> AppResult<compute::EmbeddingIdentity> {
    checked_identity(compute, root).map_err(embed_error)
}

/// Removes every vector (D4 «reindex»): the files of passages and the old mailbox index. The
/// word index stays; the messages are embedded again by the next passes.
pub fn reset_vectors(root: &Path) -> AppResult<()> {
    let lock = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(root.join("store.lock"))?;
    lock.lock()?;
    for (_, _, path) in files(root)? {
        std::fs::remove_file(path)?;
    }
    match std::fs::remove_file(root.join("rag_index.bin")) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
        _ => {}
    }
    Ok(())
}
/// Passages per embedding request: one request per batch, cancellation between batches.
const BATCH: usize = compute::EMBEDDING_BATCH;
pub fn index_document(store: &Store, content: &IndexContent, cancel: &AtomicBool, compute: &dyn ComputeGateway) -> AppResult<DocumentIndexReport> {
    let email = &content.email;
    let mut report = DocumentIndexReport { indexed: false, warnings: content.warnings.clone() };
    // Upgrading the lexical index reuses already committed vectors.
    if store.email(email.id).is_some_and(|old|old.search_index.version==2 && old.search_index.content_hash==email.remote_content_hash) {
        if let Ok(index)=read(&Store::data_dir(),email.account_id,email.id) {
            let identity = rag::read_identity(&path(&Store::data_dir(), email.account_id, email.id)).ok().flatten().unwrap_or_else(compute::EmbeddingIdentity::local);
            commit(store,email,&index,&identity)?;
            report.indexed=true;
            return Ok(report);
        }
    }
    // D4: the vectors of one index come from one model.
    let identity = server_identity(compute, &Store::data_dir())?;
    let mut index = Index::default();
    let fingerprint = rag::fingerprint(email);
    let mut documents = vec![(0, source_text(email))];
    documents.extend(email.attachments.iter().filter_map(|attachment| attachment.inline_text.as_ref().map(|text| (attachment.id, text.clone()))));
    for (attachment_id, text) in documents {
        let source_hash: [u8; 32] = Sha256::digest(text.as_bytes()).into();
        // Process every bounded window, rather than silently truncating long text.
        let mut offset = 0;
        while offset < text.len() {
            if cancel.load(Ordering::Relaxed) { return Err(AppError::new("index_paused", "Indexing paused")); }
            let mut end = (offset + 200_000).min(text.len());
            while !text.is_char_boundary(end) { end -= 1; }
            let window = &text[offset..end];
            let pieces = compute.chunk_text(window).map_err(embed_error)?;
            for batch in pieces.chunks(BATCH) {
                if cancel.load(Ordering::Relaxed) { return Err(AppError::new("index_paused", "Indexing paused")); }
                let vectors = compute.embed_documents(&email.subject, batch).map_err(embed_error)?;
                for (piece, vector) in batch.iter().zip(vectors) {
                    let start = offset + window.find(piece.as_str()).ok_or_else(|| AppError::external("Cannot link an indexed passage to its source"))?;
                    index.passages.push(Passage { account_id: email.account_id, email_id: email.id, attachment_id, fingerprint, text: String::new(), reference: Some(PassageReference { start: start as u32, end: (start + piece.len()) as u32, source_hash }), vector });
                    check_size(&index)?;
                }
            }
            offset = end;
        }
    }
    for media in &content.media {
        if cancel.load(Ordering::Relaxed) { return Err(AppError::new("index_paused", "Indexing paused")); }
        if !media.mime.starts_with("image/") { report.warnings.push(format!("Unsupported embedding modality: {}", media.mime)); continue; }
        let vector = match compute.embed_image(&media.bytes) {
            Ok(vector) => vector,
            Err(error) if compute::is_transient(error.code) => return Err(embed_error(error)),
            // A bad image costs only that image, never the message.
            Err(error) => {
                report.warnings.push(format!("Image part {}: {}", media.attachment_id, error.message));
                continue;
            },
        };
        index.passages.push(Passage { account_id: email.account_id, email_id: email.id, attachment_id: media.attachment_id, fingerprint, text: String::new(), reference: Some(PassageReference { start: 0, end: 0, source_hash: Sha256::digest(&media.bytes).into() }), vector });
        check_size(&index)?;
    }
    if index.passages.is_empty() { return Err(AppError::new("index_content_unreadable", "No indexable content was found")); }
    commit(store, email, &index, &identity)?;
    report.indexed = true;
    Ok(report)
}
/// Writes the word index of one message without vectors (no search model yet).
pub fn index_lexical(_store: &Store, content: &IndexContent, cancel: &AtomicBool) -> AppResult<DocumentIndexReport> {
    if cancel.load(Ordering::Relaxed) { return Err(AppError::new("index_paused", "Indexing paused")); }
    let email = &content.email;
    let root = Store::data_dir();
    let lock = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(root.join("store.lock"))?;
    lock.lock()?;
    let store = super::store_migration::read_active_store(&root)?;
    let current = store.email(email.id).filter(|current| current.account_id == email.account_id && !current.storage_evicted)
        .ok_or_else(|| AppError::new("index_source_changed", "Message was removed while indexing"))?;
    if current.remote_content_hash != email.remote_content_hash { return Err(AppError::new("index_source_changed", "Message changed while indexing")); }
    let measured = rag::message_index_bytes(&root, &store)?;
    let drafts = super::repository::draft_storage_bytes(&root, &store)?;
    let usage = crate::domain::storage::usage_with_storage_bytes(&store, email.account_id, &measured, &drafts).ok_or_else(|| AppError::missing("Account"))?;
    if usage.policy.admission_limit_bytes().is_some_and(|limit| usage.used_bytes > limit) {
        return Err(AppError::new("storage_full", "Indexing paused: increase this account's storage limit to index more mail"));
    }
    let remaining = usage.policy.admission_limit_bytes().map(|limit| limit.saturating_sub(usage.used_bytes));
    super::lexical_index::write(&root, email, remaining)?;
    Ok(DocumentIndexReport { indexed: true, warnings: content.warnings.clone() })
}
fn check_size(index: &Index) -> AppResult<()> {
    if rag::FILE_HEADER_BYTES as usize + index.passages.len() * (68 + rag::DIM * 4) > 8 * 1024 * 1024 {
        return Err(AppError::new("index_content_limit", "Message exceeds the 8 MiB vector working budget; source preserved"));
    }
    Ok(())
}
fn commit(_snapshot: &Store, email: &Email, index: &Index, identity: &compute::EmbeddingIdentity) -> AppResult<()> {
    let root = Store::data_dir();
    let lock = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(root.join("store.lock"))?;
    lock.lock()?;
    // The active engine decides the source (Task 4.3); the caller holds
    // `store.lock`, so no repository locking happens here.
    let store = super::store_migration::read_active_store(&root)?;
    let current = store.email(email.id).filter(|current| current.account_id == email.account_id && !current.storage_evicted)
        .ok_or_else(|| AppError::new("index_source_changed", "Message was removed while indexing"))?;
    if current.remote_content_hash != email.remote_content_hash { return Err(AppError::new("index_source_changed", "Message changed while indexing")); }
    let mut measured = rag::message_index_bytes(&root, &store)?;
    let bytes = rag::FILE_HEADER_BYTES + index.passages.len() as u64 * (68 + rag::DIM as u64 * 4);
    *measured.entry(email.id).or_default() += bytes; // Existing + atomic replacement.
    let drafts = super::repository::draft_storage_bytes(&root, &store)?;
    let usage = crate::domain::storage::usage_with_storage_bytes(&store, email.account_id, &measured, &drafts).ok_or_else(|| AppError::missing("Account"))?;
    if usage.policy.admission_limit_bytes().is_some_and(|limit| usage.used_bytes > limit) {
        return Err(AppError::new("storage_full", "Indexing paused: increase this account's storage limit to index more mail"));
    }
    let remaining=usage.policy.admission_limit_bytes().map(|limit|limit.saturating_sub(usage.used_bytes));
    super::lexical_index::write(&root,email,remaining)?;
    index.save_path_with(&path(&root, email.account_id, email.id), identity)?;
    Ok(())
}
pub fn read(root: &Path, account: u32, id: u64) -> std::io::Result<Index> {
    let path = path(root, account, id);
    if std::fs::metadata(&path)?.len() > 10 * 1024 * 1024 { return Err(std::io::Error::other("Message index exceeds its memory budget")); }
    Ok(Index::read_at(&path)?.index)
}
