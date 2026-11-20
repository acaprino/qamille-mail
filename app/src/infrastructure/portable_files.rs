//! Managed file references are portable on disk and absolute only in memory.
use crate::{
    application::{AppError, AppResult},
    model::Store,
};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    sync::{LazyLock, Mutex},
};

const WORKSPACE_PREFIX: &str = "workspace:/";

fn visit_paths(
    store: &mut Store,
    mut visit: impl FnMut(&mut String) -> AppResult<()>,
) -> AppResult<()> {
    for attachment in store
        .emails
        .iter_mut()
        .flat_map(|email| &mut email.attachments)
    {
        if let Some(path) = &mut attachment.path {
            visit(path)?;
        }
    }
    for path in store
        .drafts
        .iter_mut()
        .flat_map(|draft| &mut draft.attachments)
        .chain(
            store
                .outbox
                .iter_mut()
                .flat_map(|delivery| &mut delivery.draft.attachments),
        )
        .chain(&mut store.cache_removals)
    {
        visit(path)?;
    }
    for meeting in &mut store.meetings {
        if let Some(path) = &mut meeting.audio_path {
            visit(path)?;
        }
    }
    Ok(())
}

fn owned_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

fn persist_path_in(value: &str, root: &Path, canonical_root: Option<&Path>) -> String {
    let path = Path::new(value);
    let relative = path
        .strip_prefix(root)
        .ok()
        .or_else(|| canonical_root.and_then(|root| path.strip_prefix(root).ok()));
    if let Some(relative) = relative.filter(|path| owned_relative(path)) {
        return format!(
            "{WORKSPACE_PREFIX}{}",
            relative.to_string_lossy().replace('\\', "/")
        );
    }
    value.to_owned()
}

/// A stable managed reference for persistence and delivery identity. Canonical
/// Windows paths may include the extended-length prefix, unlike the exe path.
pub fn persistent_path(value: &str, root: &Path) -> AppResult<String> {
    let root = std::path::absolute(root)?;
    Ok(persist_path_in(
        value,
        &root,
        root.canonicalize().ok().as_deref(),
    ))
}

pub fn resolve_path(value: &str, root: &Path) -> AppResult<String> {
    if let Some(relative) = value.strip_prefix(WORKSPACE_PREFIX) {
        let relative = Path::new(relative);
        if !owned_relative(relative) {
            return Err(AppError::storage(
                "A portable file reference leaves the data directory",
            ));
        }
        // Saved with `/`: rebuilt with this system's separator, so the path in memory is the one
        // the file was saved under (on Windows `attachments\x`, never `attachments/x`) and
        // compares equal to it.
        return Ok(std::path::absolute(root)?
            .join(relative.iter().collect::<PathBuf>())
            .to_string_lossy()
            .into_owned());
    }
    Ok(value.to_owned())
}

pub fn persist_store_paths(store: &mut Store, root: &Path) -> AppResult<()> {
    let root = std::path::absolute(root)?;
    let canonical_root = root.canonicalize().ok();
    visit_paths(store, |value| {
        *value = persist_path_in(value, &root, canonical_root.as_deref());
        Ok(())
    })
}

pub fn resolve_store_paths(store: &mut Store, root: &Path) -> AppResult<()> {
    let root = std::path::absolute(root)?;
    visit_paths(store, |value| {
        *value = resolve_path(value, &root)?;
        Ok(())
    })
}

/// Copy selected files into the workspace before a draft references them. The
/// source belongs to the user and is never modified or removed.
pub fn import_draft_attachments(root: &Path, paths: &[PathBuf]) -> AppResult<Vec<String>> {
    Ok(stage_draft_attachments(root, paths)?.paths)
}

/// A picker result remains protected until the editor acknowledges this exact
/// import after a successful save, including files removed before that save.
#[derive(serde::Serialize)]
pub struct StagedAttachments {
    pub paths: Vec<String>,
    pub lease_id: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ImportLease {
    version: u32,
    paths: Vec<String>,
    expires_at: i64,
}

const STAGING_LIFETIME: i64 = 600;
static ACTIVE_IMPORTS: LazyLock<Mutex<BTreeMap<PathBuf, File>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

fn import_token() -> AppResult<String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|_| AppError::storage("Attachment import identity unavailable"))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn plain_metadata(path: &Path) -> std::io::Result<std::fs::Metadata> {
    let metadata = path.symlink_metadata()?;
    if metadata.file_type().is_symlink() {
        return Err(std::io::Error::other(
            "Managed attachment paths must not be links",
        ));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(std::io::Error::other(
                "Managed attachment paths must not be reparse points",
            ));
        }
    }
    Ok(metadata)
}

fn directory(path: &Path) -> AppResult<()> {
    match plain_metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(AppError::storage("Managed attachment directory is invalid")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir_all(path)?;
            if !plain_metadata(path)?.is_dir() {
                return Err(AppError::storage("Managed attachment directory is invalid"));
            }
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

fn import_lock(root: &Path) -> AppResult<File> {
    let path = root.join("import.lock");
    match plain_metadata(&path) {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => return Err(AppError::storage("Attachment import lock is invalid")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?;
    lock.lock()?;
    Ok(lock)
}

pub fn stage_draft_attachments(root: &Path, paths: &[PathBuf]) -> AppResult<StagedAttachments> {
    let mut files = Vec::with_capacity(paths.len());
    for source in paths {
        let name = source
            .file_name()
            .ok_or_else(|| AppError::validation("Invalid attachment name"))?;
        let file = File::open(source)?;
        if !file.metadata()?.is_file() {
            return Err(AppError::validation("Attachments must be files"));
        }
        let mut bytes = Vec::new();
        file.take(crate::rules::ATTACHMENT_CAP_BYTES + 1)
            .read_to_end(&mut bytes)?;
        files.push((name.to_os_string(), bytes));
    }
    stage_draft_attachment_bytes(root, files)
}

/// Stage files that exist only in memory, such as messages exported as `.eml`
/// to attach them to a draft. Each name must be a single plain file name.
pub fn stage_draft_attachment_bytes(
    root: &Path,
    files: Vec<(std::ffi::OsString, Vec<u8>)>,
) -> AppResult<StagedAttachments> {
    use sha2::{Digest, Sha256};
    if files.is_empty() {
        return Ok(StagedAttachments {
            paths: Vec::new(),
            lease_id: None,
        });
    }
    let root = std::path::absolute(root)?.join("draft-attachments");
    directory(&root)?;
    let root = root.canonicalize()?;
    let _lock = import_lock(&root)?;
    let staging = root.join(".staging");
    directory(&staging)?;
    let token = import_token()?;
    let record = staging.join(format!("{token}.json"));
    let record_temporary = staging.join(format!(".{token}.tmp"));
    let lease_lock = staging.join(format!("{token}.lock"));
    let mut imported = Vec::new();
    let mut created = Vec::new();
    let mut temporary = None;
    let result = (|| {
        for (name, bytes) in &files {
            let name = name.as_os_str();
            if name.is_empty()
                || name == "."
                || name == ".."
                || Path::new(name).file_name() != Some(name)
            {
                return Err(AppError::validation("Invalid attachment name"));
            }
            if bytes.len() as u64 > crate::rules::ATTACHMENT_CAP_BYTES {
                return Err(AppError::validation("File is larger than 25 MB."));
            }
            let folder = root.join(format!("{:x}", Sha256::digest(&bytes)));
            let target = folder.join(name);
            if target.exists() {
                if !plain_metadata(&folder)?.is_dir()
                    || !plain_metadata(&target)?.is_file()
                    || target.metadata()?.len() != bytes.len() as u64
                    || std::fs::read(&target)? != *bytes
                {
                    return Err(AppError::storage("A managed attachment has changed"));
                }
            } else {
                let _space = super::storage_fs::reserve(&root, bytes.len() as u64)?;
                directory(&folder)?;
                let pending = folder.join(format!(".{token}.import"));
                temporary = Some(pending.clone());
                let mut output = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&pending)?;
                output.write_all(&bytes)?;
                output.sync_all()?;
                drop(output);
                std::fs::rename(&pending, &target)?;
                temporary = None;
                created.push(target.clone());
            }
            imported.push(target.to_string_lossy().into_owned());
        }
        let guard = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&lease_lock)?;
        guard.lock()?;
        let lease = ImportLease {
            version: 1,
            paths: imported
                .iter()
                .map(|path| {
                    Path::new(path)
                        .strip_prefix(&root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/")
                })
                .collect(),
            expires_at: crate::model::now_ts() + STAGING_LIFETIME,
        };
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&record_temporary)?;
        file.write_all(&serde_json::to_vec(&lease)?)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&record_temporary, &record)?;
        ACTIVE_IMPORTS
            .lock()
            .map_err(|_| AppError::storage("Attachment staging lock failed"))?
            .insert(record.clone(), guard);
        Ok(StagedAttachments {
            paths: imported,
            lease_id: Some(token),
        })
    })();
    if result.is_err() {
        if let Some(path) = temporary {
            let _ = std::fs::remove_file(path);
        }
        for path in created {
            let _ = std::fs::remove_file(&path);
            if let Some(folder) = path.parent() {
                let _ = std::fs::remove_dir(folder);
            }
        }
        let _ = std::fs::remove_file(record);
        let _ = std::fs::remove_file(record_temporary);
        let _ = std::fs::remove_file(lease_lock);
    }
    result
}

/// Release only the import IDs captured with the saved editor snapshot. A newer
/// picker response must stay staged while an older asynchronous save finishes.
pub fn release_draft_attachment_imports(root: &Path, ids: &[String]) -> AppResult<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let root = std::path::absolute(root)?.join("draft-attachments");
    if !root.exists() {
        return Ok(());
    }
    if !plain_metadata(&root)?.is_dir() {
        return Err(AppError::storage("Managed attachment directory is invalid"));
    }
    let root = root.canonicalize()?;
    let _lock = import_lock(&root)?;
    let staging = root.join(".staging");
    if !plain_metadata(&staging)?.is_dir() {
        return Err(AppError::storage("Attachment staging directory is invalid"));
    }
    let mut active = ACTIVE_IMPORTS
        .lock()
        .map_err(|_| AppError::storage("Attachment staging lock failed"))?;
    for id in ids {
        if id.len() != 32 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(AppError::validation("Invalid attachment import identity"));
        }
        let record = staging.join(format!("{id}.json"));
        if let Some(guard) = active.remove(&record) {
            drop(guard);
            std::fs::remove_file(&record)?;
            std::fs::remove_file(record.with_extension("lock"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod staging_tests {
    use super::*;

    #[test]
    fn a_path_saved_and_read_back_is_the_same_text() {
        let root = std::env::temp_dir().join("qamille-portable-roundtrip");
        let file = std::path::absolute(root.join("attachments").join("gmail-kept.bin"))
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let saved = persistent_path(&file, &root).unwrap();
        assert_eq!(saved, "workspace:/attachments/gmail-kept.bin");
        assert_eq!(resolve_path(&saved, &root).unwrap(), file);
    }

    #[test]
    fn live_editor_survives_lease_expiry_but_crashed_import_is_collected() {
        let root = std::env::temp_dir().join(format!(
            "qamille-draft-staging-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let source = root.join("source.txt");
        std::fs::write(&source, b"long editor session").unwrap();
        let staged = stage_draft_attachments(&root, &[source.clone()]).unwrap();
        let record = root
            .join("draft-attachments/.staging")
            .canonicalize()
            .unwrap()
            .join(format!("{}.json", staged.lease_id.unwrap()));
        let future = crate::model::now_ts() + STAGING_LIFETIME + 1;
        cleanup_draft_attachments_at(&root, &Store::default(), future).unwrap();
        assert_eq!(
            std::fs::read(&staged.paths[0]).unwrap(),
            b"long editor session"
        );
        // Closing the OS handle models a process terminating without an editor
        // acknowledgment. The durable record still supplies its crash grace.
        drop(ACTIVE_IMPORTS.lock().unwrap().remove(&record).unwrap());
        cleanup_draft_attachments_at(&root, &Store::default(), crate::model::now_ts()).unwrap();
        assert!(Path::new(&staged.paths[0]).exists());
        cleanup_draft_attachments_at(&root, &Store::default(), future).unwrap();
        assert!(!Path::new(&staged.paths[0]).exists());
        assert!(!record.exists());
        assert_eq!(std::fs::read(source).unwrap(), b"long editor session");
        let checked = root.canonicalize().unwrap();
        assert!(checked.starts_with(std::env::temp_dir().canonicalize().unwrap()));
        assert!(
            checked
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("qamille-draft-staging-")
        );
        std::fs::remove_dir_all(checked).unwrap();
    }
}

/// Called with the repository lock held. Lock order is store, then import; an
/// importer never takes the store lock, so cleanup sees a committed snapshot.
pub fn cleanup_draft_attachments(root: &Path, store: &Store) -> AppResult<()> {
    cleanup_draft_attachments_at(root, store, crate::model::now_ts())
}

fn cleanup_draft_attachments_at(root: &Path, store: &Store, now: i64) -> AppResult<()> {
    use sha2::{Digest, Sha256};
    let managed = root.join("draft-attachments");
    match plain_metadata(&managed) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => return Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    }
    let managed = managed.canonicalize()?;
    let _lock = import_lock(&managed)?;
    let mut keep = HashSet::new();
    let mut referenced = store.clone();
    visit_paths(&mut referenced, |path| {
        let path = resolve_path(path, root)?;
        if let Ok(path) = Path::new(&path).canonicalize() {
            keep.insert(path);
        }
        Ok(())
    })?;
    let staging = managed.join(".staging");
    if staging.exists() {
        if !plain_metadata(&staging)?.is_dir() {
            return Err(AppError::storage("Attachment staging directory is invalid"));
        }
        for entry in std::fs::read_dir(&staging)? {
            let path = entry?.path();
            if path.extension().is_none_or(|ext| ext != "json") || !plain_metadata(&path)?.is_file()
            {
                continue;
            }
            let Some(id) = path.file_stem().and_then(|id| id.to_str()) else {
                continue;
            };
            if id.len() != 32
                || !id
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
            {
                continue;
            }
            let lease: ImportLease = serde_json::from_slice(&std::fs::read(&path)?)?;
            if lease.version != 1 {
                return Err(AppError::storage(
                    "Attachment staging format is unsupported",
                ));
            }
            let lock_path = path.with_extension("lock");
            let guard = if lock_path.exists() {
                if !plain_metadata(&lock_path)?.is_file() {
                    return Err(AppError::storage("Attachment staging lock is invalid"));
                }
                Some(OpenOptions::new().read(true).write(true).open(&lock_path)?)
            } else {
                None
            };
            let live = guard.as_ref().is_some_and(|file| file.try_lock().is_err());
            if live || lease.expires_at > now {
                for relative in lease.paths {
                    let relative = Path::new(&relative);
                    if !owned_relative(relative) {
                        return Err(AppError::storage(
                            "Attachment staging path leaves managed storage",
                        ));
                    }
                    if let Ok(path) = managed.join(relative).canonicalize() {
                        keep.insert(path);
                    }
                }
            } else {
                drop(guard);
                std::fs::remove_file(&path)?;
                if lock_path.exists() {
                    std::fs::remove_file(lock_path)?;
                }
            }
        }
        // A process can terminate between creating its lease lock/temporary
        // record and publishing the JSON record. import.lock excludes writers;
        // retain live locks and any published record before reclaiming debris.
        let mut orphan_ids = HashSet::new();
        for entry in std::fs::read_dir(&staging)? {
            let path = entry?.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let id = name.strip_suffix(".lock").or_else(|| {
                name.strip_prefix('.')
                    .and_then(|name| name.strip_suffix(".tmp"))
            });
            if let Some(id) = id.filter(|id| {
                id.len() == 32
                    && id
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
            }) {
                orphan_ids.insert(id.to_owned());
            }
        }
        for id in orphan_ids {
            match std::fs::symlink_metadata(staging.join(format!("{id}.json"))) {
                Ok(_) => continue,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            let lock_path = staging.join(format!("{id}.lock"));
            let guard = match plain_metadata(&lock_path) {
                Ok(metadata) if metadata.is_file() => {
                    let file = OpenOptions::new().read(true).write(true).open(&lock_path)?;
                    if file.try_lock().is_err() {
                        continue;
                    }
                    Some(file)
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                _ => continue,
            };
            let temporary = staging.join(format!(".{id}.tmp"));
            if plain_metadata(&temporary).is_ok_and(|metadata| metadata.is_file()) {
                std::fs::remove_file(temporary)?;
            }
            if guard.is_some() {
                drop(guard);
                std::fs::remove_file(lock_path)?;
            }
        }
    }
    for entry in std::fs::read_dir(&managed)? {
        let folder = entry?.path();
        let name = folder.file_name().unwrap().to_string_lossy();
        if name.len() != 64
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            continue;
        }
        let Ok(metadata) = plain_metadata(&folder) else {
            continue;
        };
        if !metadata.is_dir() {
            continue;
        }
        for entry in std::fs::read_dir(&folder)? {
            let path = entry?.path();
            let Ok(metadata) = plain_metadata(&path) else {
                continue;
            };
            if !metadata.is_file() || keep.contains(&path.canonicalize()?) {
                continue;
            }
            let interrupted_import = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_prefix('.'))
                .and_then(|name| name.strip_suffix(".import"))
                .is_some_and(|token| {
                    token.len() == 32
                        && token
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
                });
            if interrupted_import {
                // Holding import.lock proves no importer can still be writing
                // this generated temporary file. A crash may leave partial
                // bytes whose digest does not match the destination folder.
                std::fs::remove_file(path)?;
                continue;
            }
            // The content digest proves this is an imported copy, even if an
            // unrelated cloud draft happens to use a hexadecimal folder name.
            let mut file = File::open(&path)?;
            let mut hash = Sha256::new();
            std::io::copy(&mut file, &mut hash)?;
            if format!("{:x}", hash.finalize()) == name {
                drop(file);
                std::fs::remove_file(path)?;
            }
        }
        // Unknown files, subdirectories and links keep their folder intact.
        if std::fs::read_dir(&folder)?.next().is_none() {
            std::fs::remove_dir(folder)?;
        }
    }
    Ok(())
}
