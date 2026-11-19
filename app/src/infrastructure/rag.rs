//! Local semantic search (RAG). Vectors and passages come from the compute server through
//! [`ComputeGateway`] (spec D3: EmbeddingGemma 2 runs in `qamille-server`); the index, BM25,
//! the scores and the fusion stay here, on the user's machine. The index stores vectors and
//! verifiable source references, never passage text.

use crate::application::compute::{ComputeGateway, EmbeddingIdentity};
use crate::attach;
use crate::model::{Email, Store};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Vector length of the index (D2, D4): EmbeddingGemma 2 truncated to 256 and normalized.
pub const DIM: usize = crate::application::compute::EMBEDDING_DIMENSIONS;
pub const RRF_K: f64 = 60.0; // usual constant for rank fusion
pub const SCORE_WINDOW: f32 = 0.12; // estimated: keeps passages close to the best match

/// The search model is installed on the compute server.
/// Characters of one message body sent to the server for passages by this path (the indexing
/// of `rag_segments` sends windows of 200 000 bytes instead).
pub const MAX_DOC_CHARS: usize = 200_000;

/// Keeps the first `limit` characters of `text`.
fn truncate_chars(text: &mut String, limit: usize) {
    if let Some((end, _)) = text.char_indices().nth(limit) {
        text.truncate(end);
    }
}

pub fn model_installed(compute: &dyn ComputeGateway) -> bool {
    compute.health().is_ok_and(|health| health.embedding_ready())
}

/// Stable fingerprint: a changed mail is embedded again.
pub(super) fn fingerprint(e: &Email) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    let mut feed = |s: &str| {
        for b in s.bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x100000001b3);
        }
        h ^= 0xff;
        h = h.wrapping_mul(0x100000001b3);
    };
    feed(&e.subject);
    feed(&e.body);
    feed(&e.from_name);
    feed(&e.remote_content_hash);
    for attachment in &e.attachments {
        feed(&attachment.name);
        if let Some(text) = &attachment.inline_text { feed(text); }
    }
    h
}

fn searchable(e: &Email) -> bool {
    !matches!(e.folder.as_str(), "Trash" | "Spam")
}

// ---------- Index ----------

#[derive(Clone, Debug)]
pub struct Passage {
    pub account_id: u32,
    pub email_id: u64,
    /// 0 means the mail's own text.
    pub attachment_id: u64,
    pub fingerprint: u64,
    pub text: String,
    pub reference: Option<PassageReference>,
    pub vector: Vec<f32>,
}

#[derive(Clone, Debug)]
pub struct PassageReference {
    pub start: u32,
    pub end: u32,
    pub source_hash: [u8; 32],
}

#[derive(Default, Clone)]
pub struct Index {
    pub passages: Vec<Passage>,
}

const MAGIC: &[u8; 7] = b"SLRAG04";
const LEGACY_MAGIC: &[u8; 7] = b"SLRAG03";
/// SLRAG04 with the model that made the vectors after the header (D4): a block of
/// [`IDENTITY_BYTES`], the model id (up to 47 bytes) then the quantization (up to 15), padded
/// with zeros. The dimensions are the header's.
const IDENTITY_MAGIC: &[u8; 7] = b"SLRAG05";
const HEADER_BYTES: u64 = 19;
const IDENTITY_BYTES: u64 = 64;
/// Header of the files written now (SLRAG05).
pub const FILE_HEADER_BYTES: u64 = HEADER_BYTES + IDENTITY_BYTES;

/// Header length of a file with this magic (SLRAG03/04 have no identity block).
fn header_len(magic: &[u8]) -> u64 {
    if magic == IDENTITY_MAGIC { FILE_HEADER_BYTES } else { HEADER_BYTES }
}

fn known_magic(magic: &[u8]) -> bool {
    magic == MAGIC || magic == LEGACY_MAGIC || magic == IDENTITY_MAGIC
}

fn identity_block(identity: &EmbeddingIdentity) -> [u8; IDENTITY_BYTES as usize] {
    let mut block = [0u8; IDENTITY_BYTES as usize];
    let model = identity.model.as_bytes();
    let model = &model[..model.len().min(47)];
    block[..model.len()].copy_from_slice(model);
    let quantization = identity.quantization.as_bytes();
    let quantization = &quantization[..quantization.len().min(15)];
    block[48..48 + quantization.len()].copy_from_slice(quantization);
    block
}

fn parse_identity_block(block: &[u8], dimensions: usize) -> Option<EmbeddingIdentity> {
    let text = |bytes: &[u8]| -> Option<String> {
        let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
        String::from_utf8(bytes[..end].to_vec()).ok()
    };
    let model = text(block.get(..48)?)?;
    if model.is_empty() {
        return None;
    }
    Some(EmbeddingIdentity { model, dimensions, quantization: text(block.get(48..64)?)? })
}

/// The model that made the vectors of one file of passages (D4). Files older than D4 (SLRAG03,
/// SLRAG04) count as the current local model; a missing file is `None`.
pub fn read_identity(path: &Path) -> std::io::Result<Option<EmbeddingIdentity>> {
    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let mut header = [0u8; FILE_HEADER_BYTES as usize];
    let mut read = 0;
    while read < header.len() {
        match file.read(&mut header[read..])? {
            0 => break,
            n => read += n,
        }
    }
    if read < HEADER_BYTES as usize || !known_magic(&header[..7]) {
        return Err(invalid_index());
    }
    let dimensions = u32::from_le_bytes(header[7..11].try_into().unwrap()) as usize;
    if &header[..7] != IDENTITY_MAGIC {
        return Ok(Some(EmbeddingIdentity { dimensions, ..EmbeddingIdentity::local() }));
    }
    if read < FILE_HEADER_BYTES as usize {
        return Err(invalid_index());
    }
    parse_identity_block(&header[HEADER_BYTES as usize..], dimensions).map(Some).ok_or_else(invalid_index)
}
const RECORD_BYTES: u64 = 4 + 8 + 8 + 8 + 4 + 4 + 32 + (DIM as u64 * 4);

pub(super) struct DecodedIndex {
    pub(super) index: Index,
    pub(super) message_bytes: HashMap<u64, u64>,
}

struct IndexTemporaryFile(PathBuf);
impl Drop for IndexTemporaryFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn invalid_index() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "The semantic index cannot be read. Its original file is unchanged.",
    )
}

fn index_read_error(error: std::io::Error) -> std::io::Error {
    if error.kind() == std::io::ErrorKind::UnexpectedEof {
        // A truncated existing cache is corrupt data, not a permission or disk
        // failure. The repository can preserve it and rebuild the optional index.
        invalid_index()
    } else {
        error
    }
}

impl Index {
    pub fn path() -> PathBuf {
        Store::data_dir().join("rag_index.bin")
    }

    pub fn load() -> Index {
        Self::read_at(&Self::path())
            .map(|decoded| decoded.index)
            .unwrap_or_default()
    }

    pub(super) fn read_at(path: &Path) -> std::io::Result<DecodedIndex> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(DecodedIndex {
                    index: Index::default(),
                    message_bytes: HashMap::new(),
                });
            }
            Err(error) => return Err(error),
        };
        Self::decode(&bytes).ok_or_else(invalid_index)
    }

    fn decode(b: &[u8]) -> Option<DecodedIndex> {
        let legacy = b.get(..7)? == LEGACY_MAGIC;
        if !known_magic(b.get(..7)?) {
            return None;
        }
        let header = header_len(b.get(..7)?);
        let mut p = 7usize;
        let mut take = |n: usize| -> Option<&[u8]> {
            let end = p.checked_add(n)?;
            let s = b.get(p..end)?;
            p = end;
            Some(s)
        };
        let dim = u32::from_le_bytes(take(4)?.try_into().ok()?) as usize;
        if dim != DIM {
            return None;
        }
        let count = usize::try_from(u64::from_le_bytes(take(8)?.try_into().ok()?)).ok()?;
        if header > HEADER_BYTES {
            take((header - HEADER_BYTES) as usize)?;
        }
        let min_record = if legacy {
            28 + DIM * 4
        } else {
            RECORD_BYTES as usize
        };
        if count > b.len().saturating_sub(header as usize) / min_record {
            return None;
        }
        let mut passages = Vec::with_capacity(count);
        let mut message_bytes = HashMap::new();
        for _ in 0..count {
            let account_id = if legacy {
                0
            } else {
                u32::from_le_bytes(take(4)?.try_into().ok()?)
            };
            let email_id = u64::from_le_bytes(take(8)?.try_into().ok()?);
            let attachment_id = u64::from_le_bytes(take(8)?.try_into().ok()?);
            let fingerprint = u64::from_le_bytes(take(8)?.try_into().ok()?);
            let (reference, bytes) = if legacy {
                let len = u32::from_le_bytes(take(4)?.try_into().ok()?) as usize;
                take(len)?; // Never surface an unverified legacy excerpt as a current citation.
                (None, 28 + len as u64 + (DIM as u64 * 4))
            } else {
                let start = u32::from_le_bytes(take(4)?.try_into().ok()?);
                let end = u32::from_le_bytes(take(4)?.try_into().ok()?);
                let source_hash: [u8; 32] = take(32)?.try_into().ok()?;
                let reference =
                    (end > start && source_hash != [0; 32]).then_some(PassageReference {
                        start,
                        end,
                        source_hash,
                    });
                (reference, RECORD_BYTES)
            };
            let raw = take(dim * 4)?;
            let vector: Vec<f32> = raw
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect();
            if vector.iter().any(|value| !value.is_finite()) {
                return None;
            }
            *message_bytes.entry(email_id).or_insert(0u64) += bytes;
            passages.push(Passage {
                account_id,
                email_id,
                attachment_id,
                fingerprint,
                text: String::new(),
                reference,
                vector,
            });
        }
        if p != b.len() {
            return None;
        }
        if let Some(first) = passages.first() {
            *message_bytes.entry(first.email_id).or_default() += header;
        }
        Some(DecodedIndex {
            index: Index { passages },
            message_bytes,
        })
    }

    pub fn save(&self) -> std::io::Result<()> {
        self.save_at(&Store::data_dir())
    }

    fn save_at(&self, root: &Path) -> std::io::Result<()> {
        self.save_path(&root.join("rag_index.bin"))
    }

    pub(super) fn save_path(&self, path: &Path) -> std::io::Result<()> {
        self.save_path_with(path, &EmbeddingIdentity::local())
    }

    /// Writes the passages with the model that made their vectors (D4).
    pub(super) fn save_path_with(&self, path: &Path, identity: &EmbeddingIdentity) -> std::io::Result<()> {
        let root = path.parent().ok_or_else(invalid_index)?;
        std::fs::create_dir_all(root)?;
        let bytes =
            FILE_HEADER_BYTES.saturating_add(RECORD_BYTES.saturating_mul(self.passages.len() as u64));
        let _reservation = super::storage_fs::reserve(root, bytes)
            .map_err(|e| std::io::Error::other(e.message))?;
        if self
            .passages
            .iter()
            .any(|p| p.vector.len() != DIM || p.vector.iter().any(|v| !v.is_finite()))
        {
            return Err(invalid_index());
        }
        static NEXT_WRITE: AtomicU64 = AtomicU64::new(0);
        let tmp = path.with_extension(format!(
            "{}-{}.tmp",
            std::process::id(),
            NEXT_WRITE.fetch_add(1, Ordering::Relaxed)
        ));
        let _temporary = IndexTemporaryFile(tmp.clone());
        let mut w = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
        w.write_all(IDENTITY_MAGIC)?;
        w.write_all(&(DIM as u32).to_le_bytes())?;
        w.write_all(&(self.passages.len() as u64).to_le_bytes())?;
        w.write_all(&identity_block(identity))?;
        for p in &self.passages {
            w.write_all(&p.account_id.to_le_bytes())?;
            w.write_all(&p.email_id.to_le_bytes())?;
            w.write_all(&p.attachment_id.to_le_bytes())?;
            w.write_all(&p.fingerprint.to_le_bytes())?;
            let reference = p.reference.as_ref();
            w.write_all(&reference.map_or(0, |r| r.start).to_le_bytes())?;
            w.write_all(&reference.map_or(0, |r| r.end).to_le_bytes())?;
            w.write_all(&reference.map_or([0; 32], |r| r.source_hash))?;
            for x in &p.vector {
                w.write_all(&x.to_le_bytes())?;
            }
        }
        w.flush()?;
        w.get_ref().sync_all()?;
        drop(w);
        std::fs::rename(tmp, path)
    }

    pub fn email_count(&self) -> usize {
        self.passages
            .iter()
            .map(|p| p.email_id)
            .collect::<HashSet<_>>()
            .len()
    }
}

/// Scan record headers without loading vectors or legacy text into memory.
fn measure_index(root: &Path, store: &Store) -> std::io::Result<(HashMap<u64, u64>, bool)> {
    let file = match std::fs::File::open(root.join("rag_index.bin")) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((HashMap::new(), false));
        }
        Err(error) => return Err(error),
    };
    let length = file.metadata()?.len();
    let mut reader = std::io::BufReader::new(file);
    let mut header = [0u8; HEADER_BYTES as usize];
    reader.read_exact(&mut header).map_err(index_read_error)?;
    let legacy = &header[..7] == LEGACY_MAGIC;
    if !known_magic(&header[..7]) {
        return Err(invalid_index());
    }
    if u32::from_le_bytes(header[7..11].try_into().unwrap()) as usize != DIM {
        return Err(invalid_index());
    }
    let count = u64::from_le_bytes(header[11..19].try_into().unwrap());
    let header_bytes = header_len(&header[..7]);
    if header_bytes > HEADER_BYTES {
        if length < header_bytes {
            return Err(invalid_index());
        }
        reader.seek_relative((header_bytes - HEADER_BYTES) as i64)?;
    }
    let minimum = if legacy {
        28 + DIM as u64 * 4
    } else {
        RECORD_BYTES
    };
    if count > length.saturating_sub(header_bytes) / minimum {
        return Err(invalid_index());
    }
    let known: HashMap<_, _> = store.emails.iter().map(|email| (email.id, email)).collect();
    let mut sizes = HashMap::new();
    let mut position = header_bytes;
    let mut needs_reconcile = false;
    for index in 0..count {
        let (account_id, email_id, bytes, skipped) = if legacy {
            let mut record = [0u8; 28];
            reader.read_exact(&mut record).map_err(index_read_error)?;
            let email_id = u64::from_le_bytes(record[..8].try_into().unwrap());
            let text_len = u32::from_le_bytes(record[24..28].try_into().unwrap()) as u64;
            let skipped = text_len + DIM as u64 * 4;
            (0, email_id, 28 + skipped, skipped)
        } else {
            let mut record = [0u8; 68];
            reader.read_exact(&mut record).map_err(index_read_error)?;
            let account_id = u32::from_le_bytes(record[..4].try_into().unwrap());
            let email_id = u64::from_le_bytes(record[4..12].try_into().unwrap());
            (account_id, email_id, RECORD_BYTES, DIM as u64 * 4)
        };
        position = position.checked_add(bytes).ok_or_else(invalid_index)?;
        if position > length {
            return Err(invalid_index());
        }
        reader.seek_relative(skipped as i64)?;
        *sizes.entry(email_id).or_insert(0u64) += bytes + if index == 0 { header_bytes } else { 0 };
        needs_reconcile |= !known.get(&email_id).is_some_and(|email| {
            !email.storage_evicted && (account_id == 0 || account_id == email.account_id)
        });
    }
    if position != length {
        return Err(invalid_index());
    }
    Ok((sizes, needs_reconcile))
}

/// Account usage is attributed to stable local message IDs, including record/header bytes.
pub fn message_index_bytes(root: &Path, store: &Store) -> std::io::Result<HashMap<u64, u64>> {
    let mut sizes = measure_index(root, store)?.0;
    for (id, bytes) in super::rag_segments::message_bytes(root)? { *sizes.entry(id).or_default() += bytes; }
    for (id,bytes) in super::lexical_index::message_bytes(root,store)? {*sizes.entry(id).or_default()+=bytes;}
    Ok(sizes)
}

/// Called while the repository's store lock is held; never acquires that lock itself.
pub fn reconcile_index(root: &Path, store: &Store) -> std::io::Result<()> {
    super::rag_segments::reconcile(root, store)?;
    super::lexical_index::reconcile(root,store)?;
    if !measure_index(root, store)?.1 {
        return Ok(());
    }
    // Maintenance must be bounded too: never materialize the legacy mailbox index.
    use std::io::{Seek, SeekFrom};
    let path = root.join("rag_index.bin");
    let _reservation = super::storage_fs::reserve(root, std::fs::metadata(&path)?.len())
        .map_err(|e| std::io::Error::other(e.message))?;
    let tmp = root.join(format!("rag_index.{}.reconcile.tmp", std::process::id()));
    let _temporary = IndexTemporaryFile(tmp.clone());
    let mut writer = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
    writer.write_all(MAGIC)?;
    writer.write_all(&(DIM as u32).to_le_bytes())?;
    writer.write_all(&0u64.to_le_bytes())?;
    let mut count = 0u64;
    let mut failure = None;
    let known: HashMap<_, _> = store.emails.iter().map(|email| (email.id, email)).collect();
    visit_records(&path, &mut |passage| {
        if failure.is_some() { return; }
        let Some(email) = known.get(&passage.email_id).filter(|email| !email.storage_evicted && (passage.account_id == 0 || passage.account_id == email.account_id)) else { return };
        let written = (|| -> std::io::Result<()> {
            writer.write_all(&email.account_id.to_le_bytes())?;
            writer.write_all(&passage.email_id.to_le_bytes())?;
            writer.write_all(&passage.attachment_id.to_le_bytes())?;
            writer.write_all(&passage.fingerprint.to_le_bytes())?;
            let reference = passage.reference.as_ref();
            writer.write_all(&reference.map_or(0, |r| r.start).to_le_bytes())?;
            writer.write_all(&reference.map_or(0, |r| r.end).to_le_bytes())?;
            writer.write_all(&reference.map_or([0; 32], |r| r.source_hash))?;
            for value in passage.vector { writer.write_all(&value.to_le_bytes())?; }
            Ok(())
        })();
        if let Err(error) = written { failure = Some(error); } else { count += 1; }
    })?;
    if let Some(error) = failure { return Err(error); }
    writer.seek(SeekFrom::Start(11))?;
    writer.write_all(&count.to_le_bytes())?;
    writer.flush()?;
    writer.get_ref().sync_all()?;
    drop(writer);
    std::fs::rename(tmp, path)
}

pub struct IndexReport {
    pub added_emails: usize,
    pub removed_emails: usize,
    pub unreadable_attachments: usize,
    pub skipped_emails: usize,
}

/// Brings the index up to date: drops mail that changed or left, embeds what's new.
pub fn update_index(
    emails: &[Email],
    cancel: &AtomicBool,
    progress: &(dyn Fn(usize, usize) + Sync),
    compute: &dyn ComputeGateway,
) -> Result<(Index, IndexReport), String> {
    let mut persisted: Store = serde_json::from_slice(
        &std::fs::read(Store::path()).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("Mail data cannot be read; the semantic index is unchanged: {e}"))?;
    super::portable_files::resolve_store_paths(&mut persisted, &Store::data_dir()).map_err(|error| error.message)?;
    update_index_for_store(&persisted, emails, cancel, progress, compute)
}

fn current_message_bytes(index: &Index) -> HashMap<u64, u64> {
    let mut sizes = HashMap::new();
    for passage in &index.passages {
        *sizes.entry(passage.email_id).or_insert(0u64) += RECORD_BYTES;
    }
    if let Some(first) = index.passages.first() {
        *sizes.entry(first.email_id).or_default() += FILE_HEADER_BYTES;
    }
    sizes
}

fn fits_account_peak(
    store: &Store,
    account_id: u32,
    old: &HashMap<u64, u64>,
    next: &Index,
    drafts: &HashMap<u32, crate::domain::storage::DraftStorageBytes>,
) -> bool {
    let Some(account) = store.account(account_id) else {
        return false;
    };
    let Some(limit) = account.storage_policy.admission_limit_bytes() else {
        return true;
    };
    let mut peak = old.clone();
    for (id, size) in current_message_bytes(next) {
        *peak.entry(id).or_default() += size;
    }
    crate::domain::storage::usage_with_storage_bytes(store, account_id, &peak, drafts)
        .is_some_and(|usage| usage.used_bytes <= limit)
}

fn merge_current_index(index: &mut Index, current: &Index, store: &Store) {
    let eligible = |passage: &Passage| {
        store.email(passage.email_id).is_some_and(|email| {
            searchable(email)
                && !email.storage_evicted
                && (passage.account_id == 0 || passage.account_id == email.account_id)
                && (email.remote_only || passage.fingerprint == fingerprint(email))
        })
    };
    index.passages.retain(&eligible);
    let desired: HashSet<_> = index
        .passages
        .iter()
        .map(|passage| passage.email_id)
        .collect();
    // Another completed job may have added a different account while model work ran.
    index.passages.extend(
        current
            .passages
            .iter()
            .filter(|passage| !desired.contains(&passage.email_id) && eligible(passage))
            .cloned(),
    );
    for passage in &mut index.passages {
        if let Some(email) = store.email(passage.email_id) {
            passage.account_id = email.account_id;
        }
    }
}

/// Lock order is store, then index replacement. Model work never holds the store lock.
fn commit_index(root: &Path, index: &mut Index) -> Result<(), String> {
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join("store.lock"))
        .map_err(|e| e.to_string())?;
    lock.lock().map_err(|e| e.to_string())?;
    // The active engine decides the source (Task 4.3): under the SQLite
    // engine the JSON file is a stale pre-migration backup.
    let store = super::store_migration::read_active_store(root).map_err(|error| {
        format!(
            "Mail data cannot be read; the semantic index is unchanged: {}",
            error.message
        )
    })?;
    let old = Index::read_at(&root.join("rag_index.bin")).map_err(|e| e.to_string())?;
    merge_current_index(index, &old.index, &store);
    let drafts = super::repository::draft_storage_bytes(root, &store).map_err(|error| error.message)?;
    for account in &store.accounts {
        if !fits_account_peak(&store, account.id, &old.message_bytes, index, &drafts) {
            return Err(format!(
                "Semantic indexing paused: {} needs more local space for an atomic index update. Existing mail and the last saved index are preserved.",
                account.email
            ));
        }
    }
    index.save_at(root).map_err(|e| e.to_string())
}

/// `persisted` owns disk usage; `hydrated` is bounded, temporary content supplied by the caller.
pub fn update_index_for_store(
    persisted: &Store,
    hydrated: &[Email],
    cancel: &AtomicBool,
    progress: &(dyn Fn(usize, usize) + Sync),
    compute: &dyn ComputeGateway,
) -> Result<(Index, IndexReport), String> {
    let root = Store::data_dir();
    let decoded = Index::read_at(&root.join("rag_index.bin")).map_err(|e| e.to_string())?;
    let drafts = super::repository::draft_storage_bytes(&root, persisted).map_err(|error| error.message)?;
    let mut old_bytes = decoded.message_bytes;
    let mut index = decoded.index;
    let wanted: HashMap<u64, &Email> = persisted
        .emails
        .iter()
        .filter(|e| searchable(e) && !e.storage_evicted)
        .map(|e| (e.id, e))
        .collect();
    let available: HashMap<u64, &Email> = hydrated
        .iter()
        .filter(|e| !e.remote_only || !e.body.is_empty())
        .map(|e| (e.id, e))
        .collect();
    let before: HashSet<u64> = index.passages.iter().map(|p| p.email_id).collect();
    index.passages.retain(|p| {
        wanted.get(&p.email_id).is_some_and(|email| {
            (p.account_id == 0 || p.account_id == email.account_id)
                && available.get(&p.email_id).is_none_or(|source| {
                    source.account_id == email.account_id && fingerprint(source) == p.fingerprint
                })
        })
    });
    let kept: HashSet<u64> = index.passages.iter().map(|p| p.email_id).collect();
    let removed_emails = before.iter().filter(|id| !wanted.contains_key(id)).count();
    let todo: Vec<&Email> = hydrated
        .iter()
        .filter(|e| {
            available.contains_key(&e.id)
                && wanted
                    .get(&e.id)
                    .is_some_and(|stored| stored.account_id == e.account_id)
                && !kept.contains(&e.id)
        })
        .collect();
    let mut report = IndexReport {
        added_emails: 0,
        removed_emails,
        unreadable_attachments: 0,
        skipped_emails: 0,
    };
    if todo.is_empty() {
        commit_index(&root, &mut index)?;
        progress(0, 0);
        return Ok((index, report));
    }
    let total = todo.len();
    let mut added = HashSet::new();
    for (i, e) in todo.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let fp = fingerprint(e);
        // Automatic indexing covers the body only; it never downloads or extracts attachments.
        let mut text = format!("From {}. {}", e.from_name, e.body);
        // One `/chunks` request per message: within the server's JSON limit (8 MiB).
        truncate_chars(&mut text, MAX_DOC_CHARS);
        let source_hash: [u8; 32] = Sha256::digest(text.as_bytes()).into();
        let mut additions = Vec::new();
        let pieces = compute.chunk_text(&text).map_err(|error| error.message)?;
        for batch in pieces.chunks(crate::application::compute::EMBEDDING_BATCH) {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            let vectors = compute.embed_documents(&e.subject, batch).map_err(|error| error.message)?;
            for (piece, vector) in batch.iter().zip(vectors) {
                let start = text
                    .find(piece.as_str())
                    .ok_or("The passage could not be linked to its source")?;
                let end = start + piece.len();
                additions.push(Passage {
                    account_id: e.account_id,
                    email_id: e.id,
                    attachment_id: 0,
                    fingerprint: fp,
                    text: String::new(),
                    reference: Some(PassageReference {
                        start: start as u32,
                        end: end as u32,
                        source_hash,
                    }),
                    vector,
                });
            }
        }
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let length = index.passages.len();
        index.passages.extend(additions);
        if fits_account_peak(persisted, e.account_id, &old_bytes, &index, &drafts) {
            report.added_emails += 1;
            added.insert(e.id);
        } else {
            index.passages.truncate(length);
            report.skipped_emails += 1;
        }
        progress(i + 1, total);
        if (i + 1) % 25 == 0 {
            commit_index(&root, &mut index)?;
            old_bytes = message_index_bytes(&root, persisted).map_err(|e| e.to_string())?;
        }
    }
    commit_index(&root, &mut index)?;
    let committed: HashSet<_> = index.passages.iter().map(|p| p.email_id).collect();
    report.added_emails = added.intersection(&committed).count();
    Ok((index, report))
}

// ---------- Search ----------

#[derive(Clone, Debug)]
pub struct Hit {
    pub email_id: u64,
    pub attachment_id: u64,
    pub score: f32,
    pub passage: String,
    pub reference: Option<PassageReference>,
}

/// Best passage per mail, most similar first, keeping only what's close to the top match.
pub fn semantic_hits(
    index: &Index,
    query_vec: &[f32],
    limit: usize,
    allow: &dyn Fn(u64) -> bool,
) -> Vec<Hit> {
    let mut best: HashMap<u64, (f32, &Passage)> = HashMap::new();
    for p in &index.passages {
        if !allow(p.email_id) {
            continue;
        }
        let score: f32 = p.vector.iter().zip(query_vec).map(|(a, b)| a * b).sum();
        match best.get(&p.email_id) {
            Some((s, _)) if *s >= score => {}
            _ => {
                best.insert(p.email_id, (score, p));
            }
        }
    }
    let mut v: Vec<Hit> = best
        .into_values()
        .map(|(score, p)| Hit {
            email_id: p.email_id,
            attachment_id: p.attachment_id,
            score,
            passage: String::new(),
            reference: p.reference.clone(),
        })
        .collect();
    v.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then(b.email_id.cmp(&a.email_id))
    });
    if let Some(top) = v.first().map(|h| h.score) {
        v.retain(|h| h.score >= top - SCORE_WINDOW);
    }
    v.truncate(limit);
    v
}

/// Combines meaning and exact words by rank, so names and invoice numbers still match.
pub fn fuse(semantic: &[Hit], keyword: &[u64], limit: usize) -> Vec<Hit> {
    let mut scores: HashMap<u64, f64> = HashMap::new();
    for (rank, h) in semantic.iter().enumerate() {
        *scores.entry(h.email_id).or_default() += 1.0 / (RRF_K + rank as f64 + 1.0);
    }
    for (rank, id) in keyword.iter().enumerate() {
        *scores.entry(*id).or_default() += 1.0 / (RRF_K + rank as f64 + 1.0);
    }
    let by_id: HashMap<u64, &Hit> = semantic.iter().map(|h| (h.email_id, h)).collect();
    let mut ordered: Vec<(u64, f64)> = scores.into_iter().collect();
    ordered.sort_by(|a, b| b.1.total_cmp(&a.1).then(b.0.cmp(&a.0)));
    ordered
        .into_iter()
        .take(limit)
        .map(|(id, _)| match by_id.get(&id) {
            Some(h) => (*h).clone(),
            None => Hit {
                email_id: id,
                attachment_id: 0,
                score: 0.0,
                passage: String::new(),
                reference: None,
            },
        })
        .collect()
}

/// Full search over a store. Falls back to exact words when the model isn't usable.
pub fn hybrid_search(
    store: &Store,
    index: &Index,
    query: &str,
    limit: usize,
    compute: &dyn ComputeGateway,
) -> (Vec<Hit>, Option<String>) {
    let mut prepared = store.clone();
    if let Err(error) =
        super::search_support::prepare(&mut prepared, query, None, crate::model::now_ts())
    {
        return (Vec::new(), Some(error));
    }
    let store = &prepared;
    let parsed =
        match crate::domain::search::ParsedQuery::parse(store, query, crate::model::now_ts()) {
            Ok(parsed) => parsed,
            Err(error) => return (Vec::new(), Some(error)),
        };
    let keyword =
        match crate::domain::search::execute(store, query, None, crate::model::now_ts(), "") {
            Ok(ids) => ids,
            Err(error) => return (Vec::new(), Some(error)),
        };
    let words = parsed.semantic_words();
    let keyword_hits = |ids: &[u64]| -> Vec<Hit> {
        ids.iter()
            .take(limit)
            .map(|id| Hit {
                email_id: *id,
                attachment_id: 0,
                score: 0.0,
                passage: String::new(),
                reference: None,
            })
            .collect()
    };
    if words.trim().is_empty() {
        return (keyword_hits(&keyword), None);
    }
    if index.passages.is_empty() {
        return (
            keyword_hits(&keyword),
            Some("Semantic index is empty, so only exact words were matched.".into()),
        );
    }
    let qv = match compute.embed_query(&words) {
        Ok(v) => v,
        Err(e) => return (keyword_hits(&keyword), Some(e.message)),
    };
    let allow = |id: u64| -> bool {
        store
            .email(id)
            .is_some_and(|e| parsed.semantic_matches(e, store))
    };
    let sem = semantic_hits(index, &qv, limit, &allow);
    let mut hits = fuse(&sem, &keyword, limit);
    hydrate_hits(store, &mut hits);
    (hits, None)
}

/// Visit one vector at a time, including legacy indexes. Large indexes do not
/// have to fit in the UI process merely to rank a query.
pub(super) fn visit_records(path: &Path, visit: &mut dyn FnMut(Passage)) -> std::io::Result<()> {
    use std::io::{Seek, SeekFrom};
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let length = file.metadata()?.len();
    let mut reader = std::io::BufReader::new(file);
    let mut header = [0; HEADER_BYTES as usize];
    reader.read_exact(&mut header).map_err(index_read_error)?;
    let legacy = &header[..7] == LEGACY_MAGIC;
    if !known_magic(&header[..7]) || u32::from_le_bytes(header[7..11].try_into().unwrap()) as usize != DIM { return Err(invalid_index()); }
    let count = u64::from_le_bytes(header[11..19].try_into().unwrap());
    let header_bytes = header_len(&header[..7]);
    if header_bytes > HEADER_BYTES {
        if length < header_bytes { return Err(invalid_index()); }
        reader.seek(SeekFrom::Current((header_bytes - HEADER_BYTES) as i64))?;
    }
    let minimum = if legacy { 28 + DIM as u64 * 4 } else { RECORD_BYTES };
    if count > length.saturating_sub(header_bytes) / minimum { return Err(invalid_index()); }
    for _ in 0..count {
        let mut record = [0u8; 68];
        let (account_id, email_id, attachment_id, fingerprint, reference) = if legacy {
            reader.read_exact(&mut record[..28]).map_err(index_read_error)?;
            let text_len = u32::from_le_bytes(record[24..28].try_into().unwrap()) as u64;
            if reader.stream_position()?.saturating_add(text_len).saturating_add(DIM as u64 * 4) > length { return Err(invalid_index()); }
            reader.seek(SeekFrom::Current(text_len as i64))?;
            (0, u64::from_le_bytes(record[..8].try_into().unwrap()), u64::from_le_bytes(record[8..16].try_into().unwrap()), u64::from_le_bytes(record[16..24].try_into().unwrap()), None)
        } else {
            reader.read_exact(&mut record).map_err(index_read_error)?;
            (u32::from_le_bytes(record[..4].try_into().unwrap()), u64::from_le_bytes(record[4..12].try_into().unwrap()), u64::from_le_bytes(record[12..20].try_into().unwrap()), u64::from_le_bytes(record[20..28].try_into().unwrap()), Some(PassageReference {
                start: u32::from_le_bytes(record[28..32].try_into().unwrap()), end: u32::from_le_bytes(record[32..36].try_into().unwrap()), source_hash: record[36..68].try_into().unwrap(),
            }))
        };
        let mut bytes = [0u8; DIM * 4];
        reader.read_exact(&mut bytes).map_err(index_read_error)?;
        let vector: Vec<f32> = bytes.chunks_exact(4).map(|b| f32::from_le_bytes(b.try_into().unwrap())).collect();
        if vector.iter().any(|v| !v.is_finite()) { return Err(invalid_index()); }
        visit(Passage { account_id, email_id, attachment_id, fingerprint, reference, vector, text: String::new() });
    }
    if reader.stream_position()? != length { return Err(invalid_index()); }
    Ok(())
}

pub fn search_disk(store: &Store, query: &str, limit: usize, compute: &dyn ComputeGateway) -> Result<Vec<Hit>, String> {
    search_disk_reported(store, query, limit, compute).map(|(hits, _)| hits)
}

/// [`search_disk`], and why the results are words only when the query vector could not be had
/// (D8: the compute server is unreachable, the device was unlinked, another model): the
/// results are then the lexical ones (BM25 and the word filter), never an error.
pub fn search_disk_reported(store: &Store, query: &str, limit: usize, compute: &dyn ComputeGateway) -> Result<(Vec<Hit>, Option<crate::application::AppError>), String> {
    let mut prepared = store.clone();
    super::search_support::resolve_region(&mut prepared);
    let store = &prepared;
    let parsed = crate::domain::search::ParsedQuery::parse(store, query, crate::model::now_ts())?;
    let words = parsed.semantic_words();
    let root = Store::data_dir();
    let mut readers=HashMap::new();
    let mut lexical=Vec::new();
    for account in &store.accounts {
        if let Some(reader)=super::lexical_index::Reader::open(&root,account.id).map_err(|e|e.message)? {
            lexical.extend(reader.rank(store,&parsed,limit).map_err(|e|e.message)?);
            readers.insert(account.id,reader);
        }
    }
    lexical.sort_by(|a,b|a.1.total_cmp(&b.1).then(b.0.cmp(&a.0)));
    let mut allowed = HashSet::new();
    let mut fallback = Vec::new();
    let terms = parsed.words();
    for email in &store.emails {
        let reader=readers.get(&email.account_id);
        let indexed=match reader {Some(reader)=>reader.current(email).map_err(|e|e.message)?,None=>false};
        if indexed {
            let reader=reader.unwrap();
            if reader.matches(email,&parsed,true).map_err(|e|e.message)? {allowed.insert(email.id);}
            if reader.matches(email,&parsed,false).map_err(|e|e.message)? && !lexical.iter().any(|hit|hit.0==email.id) {fallback.push((email.id,0,email.date));}
        } else {
            let candidate=super::search_support::prepare_email(email,&parsed);
            if parsed.matches(&candidate,store) {fallback.push((email.id,crate::rules::score(&candidate,&terms),email.date));}
            if parsed.semantic_matches(&candidate,store) {allowed.insert(email.id);}
        }
    }
    fallback.sort_by_key(|&(id,score,date)|std::cmp::Reverse((score,date,id)));
    let mut keyword:Vec<_>=lexical.into_iter().map(|(id,_)|id).collect();
    keyword.extend(fallback.into_iter().map(|(id,_,_)|id));
    if words.trim().is_empty() {return Ok((fuse(&[],&keyword,limit),None));}
    // D4: a query vector of another model than the index's finds nothing that means anything.
    let embedded=super::rag_segments::checked_identity(compute,&root).and_then(|_|compute.embed_query(&words));
    let qv=match embedded {Ok(vector)=>vector,Err(error)=>return Ok((fuse(&[],&keyword,limit),Some(error)))};
    let segments = super::rag_segments::files(&root).map_err(|e| e.to_string())?;
    let segmented: HashSet<u64> = segments.iter().map(|(_, id, _)| *id).collect();
    let mut best: Vec<Hit> = Vec::new();
    let mut accept = |passage: Passage| {
        let Some(email) = store.email(passage.email_id) else { return };
        if email.search_index.version >= 2 && email.search_index.content_hash != email.remote_content_hash { return; }
        if email.storage_evicted || (passage.account_id != 0 && email.account_id != passage.account_id) || !allowed.contains(&email.id) { return; }
        let score: f32 = passage.vector.iter().zip(&qv).map(|(a, b)| a * b).sum();
        if let Some(index) = best.iter().position(|hit| hit.email_id == email.id) {
            if best[index].score >= score { return; }
            best.remove(index);
        }
        best.push(Hit { email_id: email.id, attachment_id: passage.attachment_id, score, passage: String::new(), reference: passage.reference });
        best.sort_by(|a, b| b.score.total_cmp(&a.score).then(b.email_id.cmp(&a.email_id)));
        best.truncate(limit);
    };
    visit_records(&root.join("rag_index.bin"), &mut |passage| { if !segmented.contains(&passage.email_id) { accept(passage); } }).map_err(|e| e.to_string())?;
    for (_, _, path) in segments { visit_records(&path, &mut accept).map_err(|e| e.to_string())?; }
    if let Some(top) = best.first().map(|hit| hit.score) { best.retain(|hit| hit.score >= top - SCORE_WINDOW); }
    let mut hits = fuse(&best, &keyword, limit);
    hydrate_hits(store, &mut hits);
    Ok((hits, None))
}

/// A vector is not a quotation. Fill excerpts only after source bytes match their saved hash.
pub fn hydrate_hits(store: &Store, hits: &mut [Hit]) {
    for hit in hits {
        hit.passage.clear();
        let Some(reference) = &hit.reference else {
            continue;
        };
        let Some(email) = store.email(hit.email_id) else {
            continue;
        };
        let source = if hit.attachment_id == 0 {
            if email.remote_only && email.body.is_empty() {
                continue;
            }
            if email.search_index.version >= 2 { super::rag_segments::source_text(email) } else { format!("From {}. {}", email.from_name, email.body) }
        } else {
            let Some(attachment) = email.attachments.iter().find(|a| a.id == hit.attachment_id)
            else {
                continue;
            };
            let Ok(text) = attach::read_text(attachment) else {
                continue;
            };
            text
        };
        let hash: [u8; 32] = Sha256::digest(source.as_bytes()).into();
        if hash != reference.source_hash {
            continue;
        }
        if let Some(text) = source.get(reference.start as usize..reference.end as usize) {
            hit.passage = text.into();
        }
    }
}

/// Turns hits into the context handed to the AI model, quoting the passage that matched.
pub fn context_from_hits(store: &Store, hits: &[Hit]) -> String {
    let mut out = String::new();
    for h in hits {
        let Some(e) = store.email(h.email_id) else {
            continue;
        };
        let src = e
            .attachments
            .iter()
            .find(|a| a.id == h.attachment_id)
            .map(|a| format!("attachment {} of ", a.name))
            .unwrap_or_default();
        let text = if h.passage.is_empty() {
            e.snippet(1200)
        } else {
            h.passage.clone()
        };
        out.push_str(&format!(
            "[{}email {}] From: {} | Subject: {} | Date: {}\n{}\n\n",
            src,
            e.id,
            e.from_name,
            e.subject,
            if e.date_missing {
                "Date unavailable".into()
            } else {
                crate::infrastructure::timezone::format_timestamp(&store.settings, e.date)
            },
            text
        ));
    }
    out
}

pub type Progress = Arc<Mutex<RagStatus>>;

#[derive(Clone, Default)]
pub struct RagStatus {
    pub message: String,
    pub busy: bool,
    pub fraction: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_bodies_are_cut_to_the_size_of_one_passages_request() {
        let mut text = "è".repeat(MAX_DOC_CHARS + 10);
        truncate_chars(&mut text, MAX_DOC_CHARS);
        assert_eq!(text.chars().count(), MAX_DOC_CHARS);
        let mut short = "breve".to_owned();
        truncate_chars(&mut short, MAX_DOC_CHARS);
        assert_eq!(short, "breve");
    }

    fn hit(id: u64, score: f32) -> Hit {
        Hit {
            email_id: id,
            attachment_id: 0,
            score,
            passage: String::new(),
            reference: None,
        }
    }

    #[test]
    fn semantic_query_uses_the_shared_grammar() {
        let store = Store::default();
        let query = crate::domain::search::ParsedQuery::parse(
            &store,
            "from:maria is:unread how much budget",
            0,
        )
        .unwrap();
        assert_eq!(query.semantic_words(), "how much budget");
        assert!(crate::domain::search::ParsedQuery::parse(&store, "re: budget", 0).is_err());
        let exact = crate::domain::search::ParsedQuery::parse(
            &store,
            "subject:budget \"contract signed\" estimate",
            0,
        )
        .unwrap();
        assert_eq!(exact.semantic_words(), "estimate");
    }

    #[test]
    fn best_passage_per_mail_and_score_window() {
        let p = |id, x: f32| Passage {
            account_id: 1,
            email_id: id,
            attachment_id: 0,
            fingerprint: 0,
            text: format!("{id}-{x}"),
            reference: None,
            vector: vec![x, (1.0 - x * x).max(0.0).sqrt()],
        };
        let index = Index {
            passages: vec![p(1, 0.2), p(1, 0.9), p(2, 0.85), p(3, 0.1)],
        };
        let hits = semantic_hits(&index, &[1.0, 0.0], 10, &|_| true);
        assert_eq!(
            hits.iter().map(|h| h.email_id).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert!(
            hits[0].passage.is_empty(),
            "unverified index text is not a source quotation"
        );
        assert!(semantic_hits(&index, &[1.0, 0.0], 10, &|id| id == 3).len() == 1);
    }

    #[test]
    fn fusion_rewards_both_lists() {
        let out = fuse(&[hit(1, 0.9), hit(2, 0.8)], &[2, 3], 10);
        assert_eq!(out[0].email_id, 2);
        assert_eq!(out.len(), 3);
    }

    fn temporary_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "qamille-rag-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn remove_fixture(root: &Path) {
        let _ = std::fs::remove_file(root.join("rag_index.bin"));
        let _ = std::fs::remove_dir(root);
    }

    #[test]
    fn index_round_trip_keeps_vectors_and_references_without_text() {
        let dir = temporary_root();
        let marker = "private body città ".repeat(50);
        let i = Index {
            passages: vec![Passage {
                account_id: 1,
                email_id: 7,
                attachment_id: 9,
                fingerprint: 11,
                text: marker.clone(),
                reference: Some(PassageReference {
                    start: 3,
                    end: 99,
                    source_hash: [7; 32],
                }),
                vector: vec![0.5; DIM],
            }],
        };
        i.save_at(&dir).unwrap();
        let bytes = std::fs::read(dir.join("rag_index.bin")).unwrap();
        assert!(
            !bytes
                .windows(marker.len())
                .any(|window| window == marker.as_bytes())
        );
        let r = Index::read_at(&dir.join("rag_index.bin")).unwrap().index;
        assert_eq!(r.passages.len(), 1);
        assert_eq!(r.passages[0].account_id, 1);
        assert!(r.passages[0].text.is_empty());
        assert_eq!(r.passages[0].reference.as_ref().unwrap().start, 3);
        assert_eq!(
            r.passages[0].reference.as_ref().unwrap().source_hash,
            [7; 32]
        );
        assert_eq!(r.passages[0].vector.len(), DIM);
        assert_eq!(
            message_index_bytes(&dir, &Store::default()).unwrap()[&7],
            bytes.len() as u64
        );
        remove_fixture(&dir);
    }

    #[test]
    fn legacy_index_is_counted_and_preserved_until_a_real_eviction() {
        let root = temporary_root();
        let marker = b"legacy body must not become an unverified quote";
        let mut bytes = LEGACY_MAGIC.to_vec();
        bytes.extend((DIM as u32).to_le_bytes());
        bytes.extend(1u64.to_le_bytes());
        for value in [7u64, 0, 11] {
            bytes.extend(value.to_le_bytes());
        }
        bytes.extend((marker.len() as u32).to_le_bytes());
        bytes.extend(marker);
        for _ in 0..DIM {
            bytes.extend(0.5f32.to_le_bytes());
        }
        std::fs::write(root.join("rag_index.bin"), &bytes).unwrap();
        let mut store = Store {
            emails: vec![Email {
                id: 7,
                account_id: 1,
                remote_only: true,
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(
            message_index_bytes(&root, &store).unwrap()[&7],
            bytes.len() as u64
        );
        let decoded = Index::read_at(&root.join("rag_index.bin")).unwrap();
        assert!(decoded.index.passages[0].text.is_empty());
        reconcile_index(&root, &store).unwrap();
        assert_eq!(
            std::fs::read(root.join("rag_index.bin")).unwrap(),
            bytes,
            "online-only is not quota-evicted"
        );
        store.emails[0].storage_evicted = true;
        reconcile_index(&root, &store).unwrap();
        assert!(
            Index::read_at(&root.join("rag_index.bin"))
                .unwrap()
                .index
                .passages
                .is_empty()
        );
        remove_fixture(&root);
    }

    #[test]
    fn unreadable_index_is_not_overwritten_by_maintenance() {
        let root = temporary_root();
        let bytes = b"SLRAG04 truncated";
        std::fs::write(root.join("rag_index.bin"), bytes).unwrap();
        assert_eq!(message_index_bytes(&root, &Store::default()).unwrap_err().kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(reconcile_index(&root, &Store::default()).unwrap_err().kind(), std::io::ErrorKind::InvalidData);
        assert_eq!(std::fs::read(root.join("rag_index.bin")).unwrap(), bytes);
        remove_fixture(&root);
    }

    #[test]
    fn quotes_require_current_source_hash_and_valid_utf8_range() {
        let mut store = Store {
            emails: vec![Email {
                id: 7,
                from_name: "Alice".into(),
                body: "città verified".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let source = format!("From Alice. {}", store.emails[0].body);
        let start = source.find("città").unwrap() as u32;
        let mut hits = vec![Hit {
            email_id: 7,
            attachment_id: 0,
            score: 0.8,
            passage: "stale quote".into(),
            reference: Some(PassageReference {
                start,
                end: source.len() as u32,
                source_hash: Sha256::digest(source.as_bytes()).into(),
            }),
        }];
        hydrate_hits(&store, &mut hits);
        assert_eq!(hits[0].passage, "città verified");
        store.emails[0].body = "changed body".into();
        hydrate_hits(&store, &mut hits);
        assert!(hits[0].passage.is_empty());
        store.emails[0].body.clear();
        store.emails[0].remote_only = true;
        hydrate_hits(&store, &mut hits);
        assert!(hits[0].passage.is_empty());
    }

    #[test]
    fn index_peak_admission_includes_old_and_new_records_per_account() {
        use crate::domain::{
            model::Account,
            storage::{AccountStoragePolicy, StorageLimit},
        };
        let mut store = Store {
            accounts: vec![
                Account {
                    id: 1,
                    storage_policy: AccountStoragePolicy::legacy(),
                    ..Default::default()
                },
                Account {
                    id: 2,
                    storage_policy: AccountStoragePolicy::legacy(),
                    ..Default::default()
                },
            ],
            emails: vec![Email {
                id: 7,
                account_id: 1,
                remote_only: true,
                ..Default::default()
            }],
            ..Default::default()
        };
        let next = Index {
            passages: vec![Passage {
                account_id: 1,
                email_id: 7,
                attachment_id: 0,
                fingerprint: 0,
                text: String::new(),
                reference: None,
                vector: vec![0.5; DIM],
            }],
        };
        let old = HashMap::from([(7, RECORD_BYTES)]);
        let without_index = crate::domain::storage::usage(&store, 1).unwrap().used_bytes;
        store.accounts[0].storage_policy.limit = StorageLimit::Limited {
            max_bytes: without_index + RECORD_BYTES,
        };
        assert!(!fits_account_peak(&store, 1, &old, &next, &HashMap::new()));
        assert!(
            fits_account_peak(&store, 2, &old, &next, &HashMap::new()),
            "one account's index does not consume another account's quota"
        );
        store.accounts[0].storage_policy.limit = StorageLimit::Unlimited;
        assert!(fits_account_peak(&store, 1, &old, &next, &HashMap::new()));
    }

    #[test]
    fn index_commit_keeps_other_accounts_added_during_embedding() {
        let passage = |account_id, email_id| Passage {
            account_id,
            email_id,
            attachment_id: 0,
            fingerprint: 0,
            text: String::new(),
            reference: None,
            vector: vec![0.5; DIM],
        };
        let mut store = Store {
            emails: vec![
                Email {
                    id: 7,
                    account_id: 1,
                    remote_only: true,
                    ..Default::default()
                },
                Email {
                    id: 8,
                    account_id: 2,
                    remote_only: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let mut pending = Index {
            passages: vec![passage(1, 7)],
        };
        let latest = Index {
            passages: vec![passage(2, 8)],
        };
        merge_current_index(&mut pending, &latest, &store);
        assert_eq!(pending.email_count(), 2);
        store.emails[1].storage_evicted = true;
        merge_current_index(&mut pending, &latest, &store);
        assert_eq!(pending.email_count(), 1);
        assert_eq!(pending.passages[0].email_id, 7);
    }
}
