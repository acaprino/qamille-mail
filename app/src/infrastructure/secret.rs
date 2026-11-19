//! Workspace credentials encrypted with a portable, workspace-owned key.
//! Copying the whole data folder also copies access to its credentials: this is
//! portable storage, not protection against someone who can read both files.
use ring::{
    aead,
    rand::{SecureRandom, SystemRandom},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

const VERSION: u32 = 1;
const AUTHENTICATED_FORMAT: &[u8] = b"Qamille portable credentials v1 AES-256-GCM";
/// Tags written before the rebrandings (Sparklone, then Qemille): still accepted when
/// reading, so existing credentials open; the next save rewrites them with the new tag.
const LEGACY_AUTHENTICATED_FORMATS: [&[u8]; 2] = [
    b"Qemille portable credentials v1 AES-256-GCM",
    b"Sparklone portable credentials v1 AES-256-GCM",
];

pub fn get_result(name: &str) -> Result<String, String> {
    Vault::new(crate::model::Store::data_dir()).get(name)
}

pub fn get(name: &str) -> String {
    get_result(name).unwrap_or_default()
}

pub fn set(name: &str, value: &str) -> Result<(), String> {
    Vault::new(crate::model::Store::data_dir()).set(name, value)
}

pub fn account_key(email: &str) -> String {
    format!("gmail:{}", email.to_lowercase())
}

pub const AI_KEY: &str = "ai-key";
/// The link to a Qamille server (spec A1): one JSON value, written with one `set`.
pub const SERVER_SESSION: &str = "server-session";

pub fn remove(name: &str) -> Result<(), String> {
    Vault::new(crate::model::Store::data_dir()).set(name, "")
}

struct Vault {
    root: PathBuf,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyDocument {
    version: u32,
    key: [u8; 32],
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EncryptedDocument {
    version: u32,
    nonce: [u8; 12],
    ciphertext: Vec<u8>,
}

struct Contents {
    key: Option<[u8; 32]>,
    values: BTreeMap<String, String>,
}

impl Vault {
    fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn lock(&self) -> Result<File, String> {
        fs::create_dir_all(&self.root).map_err(storage_error)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.root.join("credentials.lock"))
            .map_err(storage_error)?;
        file.lock().map_err(storage_error)?;
        Ok(file)
    }

    fn load(&self) -> Result<Contents, String> {
        let key = match read_optional(&self.root.join("credentials.key"))? {
            None => None,
            Some(bytes) => {
                let document: KeyDocument = serde_json::from_slice(&bytes).map_err(|_| {
                    "The portable credentials key is damaged. The original files are unchanged."
                        .to_string()
                })?;
                if document.version != VERSION {
                    return Err("The portable credentials key format is unsupported. The original files are unchanged.".into());
                }
                Some(document.key)
            }
        };
        let Some(bytes) = read_optional(&self.root.join("credentials.json"))? else {
            return Ok(Contents {
                key,
                values: BTreeMap::new(),
            });
        };
        let key_bytes = key.ok_or("The portable credentials key is missing. Restore credentials.key together with credentials.json.")?;
        let document: EncryptedDocument = serde_json::from_slice(&bytes).map_err(|_| {
            "Portable credentials cannot be read. The original files are unchanged.".to_string()
        })?;
        if document.version != VERSION {
            return Err(
                "The portable credentials format is unsupported. The original files are unchanged."
                    .into(),
            );
        }
        let cipher = encryption_key(&key_bytes)?;
        let plaintext = std::iter::once(AUTHENTICATED_FORMAT)
            .chain(LEGACY_AUTHENTICATED_FORMATS)
            .find_map(|format| {
                let mut sealed = document.ciphertext.clone();
                cipher
                    .open_in_place(
                        aead::Nonce::assume_unique_for_key(document.nonce),
                        aead::Aad::from(format),
                        &mut sealed,
                    )
                    .ok()
                    .map(|plaintext| plaintext.to_vec())
            })
            .ok_or_else(|| "Portable credentials failed authentication. Restore the matching credentials.key and credentials.json files.".to_string())?;
        let plaintext = plaintext.as_slice();
        let values = serde_json::from_slice(plaintext).map_err(|_| {
            "Decrypted portable credentials are invalid. The original files are unchanged."
                .to_string()
        })?;
        Ok(Contents { key, values })
    }

    fn get(&self, name: &str) -> Result<String, String> {
        let _lock = self.lock()?;
        Ok(self.load()?.values.get(name).cloned().unwrap_or_default())
    }

    fn set(&self, name: &str, value: &str) -> Result<(), String> {
        let _lock = self.lock()?;
        let mut contents = self.load()?;
        if value.is_empty() {
            if contents.values.remove(name).is_none() {
                return Ok(());
            }
        } else {
            contents.values.insert(name.to_string(), value.to_string());
        }
        let key = match contents.key {
            Some(key) => key,
            None => {
                let key = random_bytes::<32>()?;
                let bytes = serde_json::to_vec(&KeyDocument {
                    version: VERSION,
                    key,
                })
                .map_err(|error| error.to_string())?;
                publish(&self.root.join("credentials.key"), &bytes, false)?;
                key
            }
        };
        let nonce = random_bytes::<12>()?;
        let mut ciphertext =
            serde_json::to_vec(&contents.values).map_err(|error| error.to_string())?;
        encryption_key(&key)?
            .seal_in_place_append_tag(
                aead::Nonce::assume_unique_for_key(nonce),
                aead::Aad::from(AUTHENTICATED_FORMAT),
                &mut ciphertext,
            )
            .map_err(|_| "Portable credentials encryption failed.".to_string())?;
        let document = EncryptedDocument {
            version: VERSION,
            nonce,
            ciphertext,
        };
        let bytes = serde_json::to_vec(&document).map_err(|error| error.to_string())?;
        publish(&self.root.join("credentials.json"), &bytes, true)
    }
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>, String> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(storage_error(error)),
    }
}

fn random_bytes<const N: usize>() -> Result<[u8; N], String> {
    let mut bytes = [0; N];
    SystemRandom::new()
        .fill(&mut bytes)
        .map_err(|_| "Secure randomness for portable credentials is unavailable.".to_string())?;
    Ok(bytes)
}

fn encryption_key(key: &[u8; 32]) -> Result<aead::LessSafeKey, String> {
    aead::UnboundKey::new(&aead::AES_256_GCM, key)
        .map(aead::LessSafeKey::new)
        .map_err(|_| "The portable credentials encryption key is invalid.".to_string())
}

fn storage_error(error: std::io::Error) -> String {
    format!("Portable credentials storage failed: {error}")
}

fn publish(path: &Path, bytes: &[u8], replace: bool) -> Result<(), String> {
    let suffix: String = random_bytes::<12>()?
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let name = path
        .file_name()
        .ok_or("Portable credentials path is invalid.")?
        .to_string_lossy();
    let temporary = path.with_file_name(format!(".{name}.{}.{suffix}.tmp", std::process::id()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(storage_error)?;
        file.write_all(bytes).map_err(storage_error)?;
        file.sync_all().map_err(storage_error)?;
        drop(file);
        publish_file(&temporary, path, replace).map_err(storage_error)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(windows)]
fn publish_file(temporary: &Path, target: &Path, replace: bool) -> std::io::Result<()> {
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    let mut temporary = super::win_path::verbatim(temporary)?;
    temporary.push(0);
    let mut target = super::win_path::verbatim(target)?;
    target.push(0);
    let flags = MOVEFILE_WRITE_THROUGH
        | if replace {
            MOVEFILE_REPLACE_EXISTING
        } else {
            0
        };
    // Without REPLACE_EXISTING, key publication fails if a key is already present.
    if unsafe { MoveFileExW(temporary.as_ptr(), target.as_ptr(), flags) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(windows))]
fn publish_file(temporary: &Path, target: &Path, replace: bool) -> std::io::Result<()> {
    if replace {
        fs::rename(temporary, target)
    } else {
        fs::hard_link(temporary, target)?;
        fs::remove_file(temporary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::PathBuf};

    struct Workspace(PathBuf);

    impl Workspace {
        fn new() -> Self {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "qamille-portable-credentials-{}-{stamp}",
                std::process::id()
            ));
            fs::create_dir(&root).unwrap();
            Self(root)
        }

        fn vault(&self) -> Vault {
            Vault::new(&self.0)
        }
    }

    impl Drop for Workspace {
        fn drop(&mut self) {
            let root = self.0.canonicalize().unwrap();
            assert!(root.starts_with(std::env::temp_dir().canonicalize().unwrap()));
            assert!(
                root.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("qamille-portable-credentials-")
            );
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn publication_works_beyond_the_classic_windows_path_limit() {
        // std::fs handles long paths itself; the raw rename must do the same.
        let workspace = Workspace::new();
        let deep = workspace.0.join("a".repeat(120)).join("b".repeat(120));
        fs::create_dir_all(&deep).unwrap();
        let path = deep.join("credentials.bin");
        assert!(path.as_os_str().len() > 260);
        publish(&path, b"sealed", false).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"sealed");
    }

    #[test]
    fn credentials_sealed_before_the_rebranding_open_and_move_to_the_new_tag() {
        for legacy_format in LEGACY_AUTHENTICATED_FORMATS {
            let workspace = Workspace::new();
            workspace.vault().set("seed", "x").unwrap();
            let key: KeyDocument =
                serde_json::from_slice(&fs::read(workspace.0.join("credentials.key")).unwrap()).unwrap();
            // Rewrite the document exactly as a version released before the rebranding did.
            let nonce = [7u8; 12];
            let mut ciphertext = serde_json::to_vec(&BTreeMap::from([(
                "gmail:old@example.test".to_string(),
                "old-refresh-token".to_string(),
            )]))
            .unwrap();
            encryption_key(&key.key)
                .unwrap()
                .seal_in_place_append_tag(
                    aead::Nonce::assume_unique_for_key(nonce),
                    aead::Aad::from(legacy_format),
                    &mut ciphertext,
                )
                .unwrap();
            let legacy = EncryptedDocument { version: VERSION, nonce, ciphertext };
            fs::write(workspace.0.join("credentials.json"), serde_json::to_vec(&legacy).unwrap()).unwrap();

            let vault = workspace.vault();
            assert_eq!(vault.get("gmail:old@example.test").unwrap(), "old-refresh-token");
            vault.set("ai-key", "new").unwrap();
            // The next save seals with the new tag only.
            let saved: EncryptedDocument =
                serde_json::from_slice(&fs::read(workspace.0.join("credentials.json")).unwrap()).unwrap();
            let mut sealed = saved.ciphertext.clone();
            assert!(encryption_key(&key.key)
                .unwrap()
                .open_in_place(
                    aead::Nonce::assume_unique_for_key(saved.nonce),
                    aead::Aad::from(AUTHENTICATED_FORMAT),
                    &mut sealed,
                )
                .is_ok());
            assert_eq!(vault.get("gmail:old@example.test").unwrap(), "old-refresh-token");
            assert_eq!(vault.get("ai-key").unwrap(), "new");
        }
    }

    #[test]
    fn copied_workspace_reopens_encrypted_credentials() {
        let source = Workspace::new();
        let copied = Workspace::new();
        source
            .vault()
            .set("gmail:portable@example.test", "portable-refresh-token")
            .unwrap();
        source.vault().set("ai-key", "portable-ai-secret").unwrap();
        for name in ["credentials.key", "credentials.json"] {
            fs::copy(source.0.join(name), copied.0.join(name)).unwrap();
        }
        let bytes = fs::read(copied.0.join("credentials.json")).unwrap();
        let stored = String::from_utf8(bytes).unwrap();
        assert!(!stored.contains("portable-refresh-token"));
        assert!(!stored.contains("portable-ai-secret"));
        assert_eq!(
            copied.vault().get("gmail:portable@example.test").unwrap(),
            "portable-refresh-token"
        );
        assert_eq!(copied.vault().get("ai-key").unwrap(), "portable-ai-secret");
    }

    #[test]
    fn workspaces_do_not_share_credentials() {
        let first = Workspace::new();
        let second = Workspace::new();
        first
            .vault()
            .set("ai-key", "first-workspace-secret")
            .unwrap();
        assert_eq!(second.vault().get("ai-key").unwrap(), "");
        assert!(!second.0.join("credentials.key").exists());
        second
            .vault()
            .set("ai-key", "second-workspace-secret")
            .unwrap();
        assert_eq!(
            first.vault().get("ai-key").unwrap(),
            "first-workspace-secret"
        );
        assert_eq!(
            second.vault().get("ai-key").unwrap(),
            "second-workspace-secret"
        );
    }

    #[test]
    fn clearing_one_credential_preserves_the_others() {
        let workspace = Workspace::new();
        workspace.vault().set("ai-key", "keep-me").unwrap();
        workspace
            .vault()
            .set("google-client-secret", "remove-me")
            .unwrap();
        workspace.vault().set("google-client-secret", "").unwrap();
        assert_eq!(workspace.vault().get("google-client-secret").unwrap(), "");
        assert_eq!(workspace.vault().get("ai-key").unwrap(), "keep-me");
    }

    #[test]
    fn writes_use_fresh_nonces_without_replacing_the_key() {
        let workspace = Workspace::new();
        workspace.vault().set("ai-key", "same-value").unwrap();
        let first = fs::read(workspace.0.join("credentials.json")).unwrap();
        let key = fs::read(workspace.0.join("credentials.key")).unwrap();
        workspace.vault().set("ai-key", "same-value").unwrap();
        assert_ne!(
            fs::read(workspace.0.join("credentials.json")).unwrap(),
            first
        );
        assert_eq!(fs::read(workspace.0.join("credentials.key")).unwrap(), key);
        assert_eq!(workspace.vault().get("ai-key").unwrap(), "same-value");
    }

    #[test]
    fn corrupted_ciphertext_cannot_be_read_or_overwritten() {
        let workspace = Workspace::new();
        workspace.vault().set("ai-key", "original").unwrap();
        let key = fs::read(workspace.0.join("credentials.key")).unwrap();
        let mut document: serde_json::Value =
            serde_json::from_slice(&fs::read(workspace.0.join("credentials.json")).unwrap())
                .unwrap();
        let first = document["ciphertext"][0].as_u64().unwrap();
        document["ciphertext"][0] = serde_json::json!(first ^ 1);
        let damaged = serde_json::to_vec(&document).unwrap();
        fs::write(workspace.0.join("credentials.json"), &damaged).unwrap();
        assert!(workspace.vault().get("ai-key").is_err());
        assert!(workspace.vault().set("other", "replacement").is_err());
        assert_eq!(
            fs::read(workspace.0.join("credentials.json")).unwrap(),
            damaged
        );
        assert_eq!(fs::read(workspace.0.join("credentials.key")).unwrap(), key);
    }

    #[test]
    fn missing_key_is_an_error_without_creating_a_replacement() {
        let workspace = Workspace::new();
        workspace.vault().set("ai-key", "original").unwrap();
        let original = fs::read(workspace.0.join("credentials.json")).unwrap();
        fs::remove_file(workspace.0.join("credentials.key")).unwrap();
        assert!(workspace.vault().get("ai-key").is_err());
        assert!(workspace.vault().set("ai-key", "replacement").is_err());
        assert!(!workspace.0.join("credentials.key").exists());
        assert_eq!(
            fs::read(workspace.0.join("credentials.json")).unwrap(),
            original
        );
    }

    #[test]
    fn invalid_key_or_json_is_not_overwritten() {
        for name in ["credentials.key", "credentials.json"] {
            let workspace = Workspace::new();
            workspace.vault().set("ai-key", "original").unwrap();
            fs::write(workspace.0.join(name), b"{damaged original").unwrap();
            let before_key = fs::read(workspace.0.join("credentials.key")).unwrap();
            let before_data = fs::read(workspace.0.join("credentials.json")).unwrap();
            assert!(workspace.vault().get("ai-key").is_err());
            assert!(workspace.vault().set("ai-key", "replacement").is_err());
            assert_eq!(
                fs::read(workspace.0.join("credentials.key")).unwrap(),
                before_key
            );
            assert_eq!(
                fs::read(workspace.0.join("credentials.json")).unwrap(),
                before_data
            );
        }
    }

    #[test]
    fn concurrent_writers_preserve_each_credential() {
        let workspace = Workspace::new();
        let handles: Vec<_> = (0..4)
            .map(|worker| {
                let root = workspace.0.clone();
                std::thread::spawn(move || {
                    for entry in 0..5 {
                        Vault::new(&root)
                            .set(&format!("{worker}:{entry}"), "value")
                            .unwrap();
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        for worker in 0..4 {
            for entry in 0..5 {
                assert_eq!(
                    workspace.vault().get(&format!("{worker}:{entry}")).unwrap(),
                    "value"
                );
            }
        }
    }

    #[test]
    fn key_publication_never_replaces_an_existing_key() {
        let workspace = Workspace::new();
        let path = workspace.0.join("credentials.key");
        fs::write(&path, b"existing key must survive").unwrap();
        assert!(publish(&path, b"replacement key", false).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"existing key must survive");
        assert_eq!(fs::read_dir(&workspace.0).unwrap().count(), 1);
    }

    #[test]
    fn mismatched_workspace_key_cannot_overwrite_credentials() {
        let first = Workspace::new();
        let second = Workspace::new();
        first.vault().set("ai-key", "first credential").unwrap();
        second.vault().set("ai-key", "second credential").unwrap();
        fs::copy(
            second.0.join("credentials.key"),
            first.0.join("credentials.key"),
        )
        .unwrap();
        let original = fs::read(first.0.join("credentials.json")).unwrap();
        let key = fs::read(first.0.join("credentials.key")).unwrap();
        assert!(first.vault().get("ai-key").is_err());
        assert!(first.vault().set("ai-key", "replacement").is_err());
        assert_eq!(
            fs::read(first.0.join("credentials.json")).unwrap(),
            original
        );
        assert_eq!(fs::read(first.0.join("credentials.key")).unwrap(), key);
    }
}
