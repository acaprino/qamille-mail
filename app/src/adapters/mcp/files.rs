//! Explicitly allowed imports and managed exports for MCP. No unrestricted path reads.
use crate::application::{AppError, AppResult};
use crate::model::Store;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

const LIFETIME: i64 = 86400;
const MAX_FILE: u64 = 256 * 1024 * 1024;

pub fn bind_profile(handle: &str, profile: &str, grants: &str) -> AppResult<()> {
    let id = valid_handle(handle)?;
    let path = root()?.join(format!("{id}.json"));
    let mut value: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    if let Some(owner) = value.get("owner_profile") {
        if owner != profile || value["owner_grants"] != grants {
            return Err(AppError::new(
                "permission_denied",
                "File belongs to another assistant grant",
            ));
        }
        return Ok(());
    }
    value["owner_profile"] = json!(profile);
    value["owner_grants"] = json!(grants);
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    let mut file = File::create(&temporary)?;
    file.write_all(&serde_json::to_vec(&value)?)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(temporary, path)?;
    Ok(())
}
pub fn check_profile(handle: &str, profile: &str, grants: &str) -> AppResult<()> {
    let id = valid_handle(handle)?;
    let value: Value = serde_json::from_slice(&std::fs::read(root()?.join(format!("{id}.json")))?)?;
    if value
        .get("owner_profile")
        .is_some_and(|owner| owner != profile || value["owner_grants"] != grants)
    {
        return Err(AppError::new(
            "permission_denied",
            "File is outside this assistant profile or its permissions changed",
        ));
    }
    Ok(())
}
#[derive(Serialize, Deserialize)]
struct Descriptor {
    path: PathBuf,
    name: String,
    mime_type: String,
    expires_at: i64,
    #[serde(default)]
    account_ids: Option<Vec<u32>>,
}

fn root() -> AppResult<PathBuf> {
    let p = Store::data_dir().join("mcp-files");
    std::fs::create_dir_all(&p)?;
    Ok(p.canonicalize()?)
}
fn token() -> AppResult<String> {
    let mut bytes = [0u8; 24];
    getrandom::fill(&mut bytes).map_err(|_| AppError::storage("File handle unavailable"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn valid_handle(handle: &str) -> AppResult<&str> {
    let id = handle.strip_prefix("qamille://files/").unwrap_or(handle);
    if id.len() != 48 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(AppError::validation("Invalid managed file handle"));
    }
    Ok(id)
}
fn allowed(path: &Path) -> AppResult<PathBuf> {
    let canonical = path.canonicalize()?;
    let managed = root()?;
    let mut roots = Vec::new();
    if let Some(value) = std::env::var_os("QAMILLE_MCP_FILE_ROOTS") {
        roots.extend(std::env::split_paths(&value).filter_map(|p| p.canonicalize().ok()));
    }
    allowed_import(&canonical, &managed, &roots)?;
    validate_file(&canonical)?;
    Ok(canonical)
}
fn allowed_import(canonical: &Path, managed: &Path, roots: &[PathBuf]) -> AppResult<()> {
    // Managed descriptors and payloads must only be accessed through their original
    // handles: importing either would strip their profile/account grant ownership.
    if canonical.starts_with(managed) {
        return Err(AppError::new(
            "permission_denied",
            "Use the original file handle to access managed MCP files",
        ));
    }
    if !roots.iter().any(|r| canonical.starts_with(r)) {
        return Err(AppError::new(
            "permission_denied",
            "Choose a file inside QAMILLE_MCP_FILE_ROOTS",
        ));
    }
    Ok(())
}
fn validate_file(canonical: &Path) -> AppResult<()> {
    let meta = std::fs::metadata(&canonical)?;
    if !meta.is_file() || meta.len() > MAX_FILE {
        return Err(AppError::validation(
            "Managed files must be regular files up to 256 MiB",
        ));
    }
    Ok(())
}
fn save_descriptor(
    path: PathBuf,
    name: String,
    mime: &str,
    account_ids: Option<Vec<u32>>,
) -> AppResult<Value> {
    let id = token()?;
    let size = std::fs::metadata(&path)?.len();
    let managed = root()?;
    let relative = path.strip_prefix(&managed)
        .map_err(|_| AppError::storage("File is outside managed storage"))?.to_path_buf();
    let descriptor = Descriptor {
        path: relative,
        name: name.clone(),
        mime_type: mime.into(),
        expires_at: crate::model::now_ts() + LIFETIME,
        account_ids,
    };
    let mut f = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(root()?.join(format!("{id}.json")))?;
    f.write_all(&serde_json::to_vec(&descriptor)?)?;
    f.sync_all()?;
    Ok(
        json!({"handle":id,"uri":format!("qamille://files/{id}"),"name":name,"mime_type":mime,"size":size,"expires_at":descriptor.expires_at}),
    )
}
pub fn register(path: &Path) -> AppResult<Value> {
    let source = allowed(path)?;
    let name = source
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("file")
        .to_owned();
    // Snapshot inputs: a user changing the original file cannot change a queued send.
    // Keep the original basename intact for providers that derive MIME filenames
    // from attachment paths. The random identity belongs to its parent directory.
    let target = payload_target(source.file_name().unwrap_or_else(|| std::ffi::OsStr::new("file")))?;
    std::fs::copy(&source, &target)?;
    OpenOptions::new().write(true).open(&target)?.sync_all()?;
    save_descriptor(target, name, "application/octet-stream", None)
}
fn payload_target(name: &std::ffi::OsStr) -> AppResult<PathBuf> {
    let mut components = Path::new(name).components();
    if !matches!(
        (components.next(), components.next()),
        (Some(std::path::Component::Normal(_)), None)
    ) {
        return Err(AppError::validation("Managed files need a single filename"));
    }
    let directory = root()?.join(token()?);
    std::fs::create_dir(&directory)?;
    Ok(directory.join(name))
}
fn safe_name(name: &str) -> String {
    let value: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .take(100)
        .collect();
    if value.is_empty() || matches!(value.as_str(), "." | "..") {
        "file".into()
    } else {
        value
    }
}
pub fn write(name: &str, mime: &str, bytes: &[u8]) -> AppResult<Value> {
    write_internal(name, mime, bytes, None)
}
pub fn write_scoped(name: &str, mime: &str, bytes: &[u8], account_ids: &[u32]) -> AppResult<Value> {
    write_internal(name, mime, bytes, Some(account_ids.to_vec()))
}
fn write_internal(
    name: &str,
    mime: &str,
    bytes: &[u8],
    account_ids: Option<Vec<u32>>,
) -> AppResult<Value> {
    if bytes.len() as u64 > MAX_FILE {
        return Err(AppError::validation("Export exceeds 256 MiB"));
    }
    let target = payload_target(std::ffi::OsStr::new(&safe_name(name)))?;
    let mut f = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&target)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    save_descriptor(target, safe_name(name), mime, account_ids)
}
pub fn resolve(handle: &str) -> AppResult<PathBuf> {
    resolve_scoped(handle, None)
}
pub fn resolve_scoped(handle: &str, allowed_accounts: Option<&[u32]>) -> AppResult<PathBuf> {
    let id = valid_handle(handle)?;
    let descriptor_path = root()?.join(format!("{id}.json"));
    if descriptor_path.canonicalize()?.parent() != Some(root()?.as_path()) {
        return Err(AppError::new(
            "permission_denied",
            "Invalid file descriptor",
        ));
    }
    let descriptor: Descriptor = serde_json::from_slice(&std::fs::read(descriptor_path)?)?;
    if descriptor.expires_at < crate::model::now_ts() {
        return Err(AppError::new("handle_expired", "Register the file again"));
    }
    if let (Some(allowed), Some(required)) = (allowed_accounts, &descriptor.account_ids) {
        if !required.iter().all(|id| allowed.contains(id)) {
            return Err(AppError::new(
                "permission_denied",
                "This file contains data from accounts outside the active profile",
            ));
        }
    }
    // Only snapshots owned by the managed directory may be resolved.
    let p = root()?.join(descriptor.path).canonicalize()?;
    if !p.starts_with(root()?) {
        return Err(AppError::new(
            "permission_denied",
            "File is outside managed storage",
        ));
    }
    validate_file(&p)?;
    Ok(p)
}
pub fn read(handle: &str, offset: u64, limit: usize) -> AppResult<Value> {
    read_scoped(handle, offset, limit, None)
}
pub fn read_scoped(
    handle: &str,
    offset: u64,
    limit: usize,
    allowed_accounts: Option<&[u32]>,
) -> AppResult<Value> {
    let path = resolve_scoped(handle, allowed_accounts)?;
    let mut file = File::open(&path)?;
    let total = file.metadata()?.len();
    if offset > total {
        return Err(AppError::validation("File offset exceeds size"));
    }
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = vec![0; limit.clamp(1, 48 * 1024).min((total - offset) as usize)];
    file.read_exact(&mut bytes)?;
    let next = offset + bytes.len() as u64;
    Ok(
        json!({"encoding":"base64","data":base64(&bytes),"offset":offset,"total_bytes":total,"next_offset":(next<total).then_some(next)}),
    )
}
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in bytes.chunks(3) {
        let a = c[0] as u32;
        let b = c.get(1).copied().unwrap_or(0) as u32;
        let d = c.get(2).copied().unwrap_or(0) as u32;
        let n = (a << 16) | (b << 8) | d;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(if c.len() > 1 {
            TABLE[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            TABLE[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn encoding_matches_standard_vectors() {
        assert_eq!(super::base64(b""), "");
        assert_eq!(super::base64(b"f"), "Zg==");
        assert_eq!(super::base64(b"fo"), "Zm8=");
        assert_eq!(super::base64(b"foo"), "Zm9v");
    }
    #[test]
    fn handles_cannot_traverse_directories() {
        for s in ["../secret", "qamille://files/../secret", "C:\\secret", ""] {
            assert!(super::valid_handle(s).is_err());
        }
    }
    #[test]
    fn managed_descriptors_and_payloads_cannot_be_imported_under_broad_roots() {
        use std::path::PathBuf;
        let external = PathBuf::from("workspace");
        let managed = external.join("mcp-files");
        let roots = vec![external.clone(), managed.clone()];
        for path in [managed.join("grant.json"), managed.join("payload.ics")] {
            assert_eq!(
                super::allowed_import(&path, &managed, &roots).unwrap_err().code,
                "permission_denied"
            );
        }
        assert!(super::allowed_import(&external.join("import.ics"), &managed, &roots).is_ok());
    }
    #[test]
    fn export_names_cannot_resolve_to_parent_or_current_directory() {
        for name in ["", ".", ".."] {
            assert_eq!(super::safe_name(name), "file");
        }
        assert_eq!(super::safe_name("../private.ics"), ".._private.ics");
    }
}
