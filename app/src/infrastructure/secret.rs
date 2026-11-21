//! Workspace credentials (`credentials.json`), sealed with AES-256-GCM under a key kept beside
//! them in the data folder.
//!
//! On Windows (`KeyStore::Dpapi`, both editions: owner ruling R1 of phase 3) the key is wrapped
//! with DPAPI for the current Windows user in `credentials.key.dpapi`. A data folder copied to
//! another Windows user or computer keeps its mail and settings, but its credentials open there
//! only with the transfer passphrase, when the person set one (`credentials.key.transfer`, an
//! Argon2id copy of the key). Without it the person signs in to the accounts again: the old
//! files are set aside in a folder, never deleted. The plain `credentials.key` of earlier
//! versions is wrapped at the first load and removed only once the wrapped copy reads back and
//! opens the credentials, so a crash at any step leaves a key that opens them.
//!
//! Elsewhere (`KeyStore::Portable`) the key stays plain in `credentials.key`: copying the whole
//! data folder also copies access to its credentials.
use ring::{
    aead, digest,
    rand::{SecureRandom, SystemRandom},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
use unicode_normalization::UnicodeNormalization;
use zeroize::{Zeroize, Zeroizing};
use crate::application::error::vault_codes;

const VERSION: u32 = 1;
const AUTHENTICATED_FORMAT: &[u8] = b"Qamille portable credentials v1 AES-256-GCM";
/// Tags written before the rebrandings (Sparklone, then Qemille): still accepted when
/// reading, so existing credentials open; the next save rewrites them with the new tag.
const LEGACY_AUTHENTICATED_FORMATS: [&[u8]; 2] = [
    b"Qemille portable credentials v1 AES-256-GCM",
    b"Sparklone portable credentials v1 AES-256-GCM",
];

const VAULT_FILE: &str = "credentials.json";
const PLAIN_KEY_FILE: &str = "credentials.key";
const PROTECTED_KEY_FILE: &str = "credentials.key.dpapi";
const PROTECTED_KEY_TEMPORARY: &str = "credentials.key.dpapi.tmp";
const TRANSFER_KEY_FILE: &str = "credentials.key.transfer";
/// Names the folder of a start-over until every file is in it (see `set_aside`).
const SET_ASIDE_MARKER: &str = "credentials.set-aside";
/// Names the folder of a start-over that a sign-in made, until the interface has told the person.
const SET_ASIDE_NOTICE: &str = "credentials.set-aside.notice";
/// A wrapped key that opens nothing here, moved out of the way so the plain key can be wrapped.
const UNUSED_PROTECTED_KEY_PREFIX: &str = "credentials.key.dpapi.unused-";
const SET_ASIDE_PREFIX: &str = "credentials-set-aside-";
/// What a start-over moves, credentials first. Mail, settings and everything else stay.
const SET_ASIDE_FILES: [&str; 5] =
    [VAULT_FILE, PROTECTED_KEY_FILE, TRANSFER_KEY_FILE, PLAIN_KEY_FILE, PROTECTED_KEY_TEMPORARY];

/// Names the key in its wrapped copies without revealing it, so a copy that Windows cannot
/// open here can still be matched with the transfer copy of the same key.
const KEY_CHECK_DOMAIN: &[u8] = b"Qamille credentials key check v1";
const TRANSFER_FORMAT: &[u8] = b"Qamille credentials transfer key v1 Argon2id AES-256-GCM";
const ARGON2ID: &str = "argon2id";
const ARGON2_VERSION: u32 = 0x13;
/// Argon2id for the transfer copy: RFC 9106, second recommended option (64 MiB, 3 passes,
/// 4 lanes). Each copy stores the parameters it was written with.
const TRANSFER_KDF: KdfParams = KdfParams { memory_kib: 64 * 1024, iterations: 3, parallelism: 4 };
/// Bounds for the parameters read from a transfer copy: a damaged file must not make an unlock
/// allocate more than 256 MiB or run for more than a few seconds.
const MAX_MEMORY_KIB: u32 = 256 * 1024;
const MAX_ITERATIONS: u32 = 10;
const MAX_PARALLELISM: u32 = 16;
pub const MIN_TRANSFER_PASSPHRASE_CHARS: usize = 8;
const MAX_TRANSFER_PASSPHRASE_BYTES: usize = 1024;

const FOREIGN_KEY: &str = "The credentials key belongs to another Windows user or another computer: sign in to the accounts again in Settings.";
const LOCKED_WITH_TRANSFER: &str = "The credentials key belongs to another Windows user or another computer: enter the transfer passphrase in Settings, or sign in to the accounts again.";
const DAMAGED_PROTECTED_KEY: &str = "The protected credentials key is damaged. The original files are unchanged.";
const UNSUPPORTED_PROTECTED_KEY: &str = "The protected credentials key format is unsupported. The original files are unchanged.";
const MISSING_PROTECTED_KEY: &str = "The credentials key is missing. Restore credentials.key.dpapi together with credentials.json.";
const PROTECTION_NOT_VERIFIED: &str = "The credentials key protected by Windows did not read back. The plain key is kept.";
const WRONG_PASSPHRASE: &str = "The transfer passphrase is not correct.";
const DAMAGED_TRANSFER: &str = "The transfer copy of the credentials key is damaged. The original files are unchanged.";
const TRANSFER_MISMATCH: &str = "The transfer passphrase opens a key that does not match the saved credentials. The original files are unchanged.";
const NO_TRANSFER: &str = "No transfer passphrase was set for these credentials: sign in to the accounts again in Settings.";
const NEEDS_UNLOCKED: &str = "Open the credentials on this computer before changing the transfer passphrase.";
const TRANSFER_NEEDS_DPAPI: &str = "A transfer passphrase is used only when Windows protects the credentials key.";
const SHORT_PASSPHRASE: &str = "The transfer passphrase needs at least 8 characters that are not spaces.";
const LONG_PASSPHRASE: &str = "The transfer passphrase is too long.";
const NOTHING_TO_START_OVER: &str = "The credentials open on this computer: there is nothing to start over.";
const DAMAGED_SET_ASIDE_MARKER: &str = "The note of an interrupted start-over (credentials.set-aside) is damaged. The original files are unchanged.";
const NO_MEMORY_FOR_TRANSFER: &str = "There is not enough memory to open the transfer copy of the credentials key. Close other programs and try again.";
const DERIVATION_FAILED: &str = "The transfer passphrase could not be turned into a key. The original files are unchanged.";
const PORTABLE_KEY_MISSING: &str = "The portable credentials key is missing. Restore credentials.key together with credentials.json.";
const VAULT_UNREADABLE: &str = "Portable credentials cannot be read. The original files are unchanged.";
const VAULT_UNSUPPORTED: &str = "The portable credentials format is unsupported. The original files are unchanged.";
const PLAIN_KEY_DAMAGED: &str = "The portable credentials key is damaged. The original files are unchanged.";
const PLAIN_KEY_UNSUPPORTED: &str = "The portable credentials key format is unsupported. The original files are unchanged.";
const VAULT_NOT_AUTHENTIC: &str = "Portable credentials failed authentication. Restore the matching credentials.key and credentials.json files.";
const VAULT_CONTENT_INVALID: &str = "Decrypted portable credentials are invalid. The original files are unchanged.";
const NO_RANDOMNESS: &str = "Secure randomness for portable credentials is unavailable.";
const INVALID_KEY: &str = "The portable credentials encryption key is invalid.";
const SEALING_FAILED: &str = "Portable credentials encryption failed.";
const INVALID_PATH: &str = "Portable credentials path is invalid.";
const NO_DPAPI: &str = "Windows key protection is not available on this system.";
/// DPAPI failed for another reason than a key of another user: a sign-in never sets it aside.
const UNAVAILABLE_PREFIX: &str = "Windows could not open the credentials key right now";
/// `{STORAGE_FAILED} {the system's error}`.
const STORAGE_FAILED: &str = "Portable credentials storage failed:";

fn unavailable_message(code: u32) -> String {
    format!("{UNAVAILABLE_PREFIX} (error 0x{code:08X}). Nothing was changed: try again later.")
}

/// The code the interface words a vault message by (`vault_codes`); every message of this
/// module has one.
pub fn error_code(message: &str) -> &'static str {
    match message {
        FOREIGN_KEY => vault_codes::FOREIGN,
        LOCKED_WITH_TRANSFER => vault_codes::LOCKED,
        WRONG_PASSPHRASE => vault_codes::WRONG_PASSPHRASE,
        NO_TRANSFER => vault_codes::NO_TRANSFER,
        NEEDS_UNLOCKED => vault_codes::NEEDS_UNLOCK,
        SHORT_PASSPHRASE | LONG_PASSPHRASE => vault_codes::PASSPHRASE,
        NOTHING_TO_START_OVER => vault_codes::NOTHING_TO_START_OVER,
        TRANSFER_NEEDS_DPAPI | NO_DPAPI => vault_codes::NEEDS_DPAPI,
        NO_MEMORY_FOR_TRANSFER => vault_codes::UNAVAILABLE,
        message if message.starts_with(UNAVAILABLE_PREFIX) => vault_codes::UNAVAILABLE,
        message if message.starts_with(STORAGE_FAILED) => vault_codes::STORAGE,
        _ => vault_codes::DAMAGED,
    }
}

pub fn get_result(name: &str) -> Result<String, String> {
    workspace_vault().get(name)
}

pub fn get(name: &str) -> String {
    get_result(name).unwrap_or_default()
}

pub fn set(name: &str, value: &str) -> Result<(), String> {
    workspace_vault().set(name, value)
}

pub fn account_key(email: &str) -> String {
    format!("gmail:{}", email.to_lowercase())
}

pub const AI_KEY: &str = "ai-key";
/// The link to a Qamille server (spec A1): one JSON value, written with one `set`.
pub const SERVER_SESSION: &str = "server-session";

pub fn remove(name: &str) -> Result<(), String> {
    workspace_vault().set(name, "")
}

/// Where the credentials of this data folder stand, for Settings and the start-up notice.
pub fn status() -> VaultStatus {
    workspace_vault().status()
}

/// Writes (or replaces) the transfer copy of the key. The key must open here. The passphrase
/// is taken by value and wiped when done; it is never stored or logged.
pub fn set_transfer_passphrase(passphrase: String) -> Result<VaultStatus, String> {
    let passphrase = Zeroizing::new(passphrase);
    workspace_vault().set_transfer_passphrase(&passphrase)
}

pub fn remove_transfer_passphrase() -> Result<VaultStatus, String> {
    workspace_vault().remove_transfer_passphrase()
}

/// On another Windows user or computer: opens the key with the transfer passphrase and wraps
/// it again for the current user. The passphrase is wiped when done.
pub fn unlock_with_transfer_passphrase(passphrase: String) -> Result<VaultStatus, String> {
    let passphrase = Zeroizing::new(passphrase);
    workspace_vault().unlock_with_transfer_passphrase(&passphrase)
}

/// Sets credentials that do not open here aside (never deletes them) so the person can sign in
/// again. Returns the name of the folder, inside the data folder, that now holds them.
/// `unopenable_too`: the person tried again, Windows still cannot open the key, and they confirmed.
pub fn start_over(unopenable_too: bool) -> Result<String, String> {
    workspace_vault().start_over(unopenable_too)
}

/// Before a sign-in that will save a credential: an error when the save would be refused (a
/// transfer passphrase to enter first, damaged files), so no login and no device is made for
/// nothing. Credentials of another user without a passphrase pass: the save sets them aside.
pub fn check_writable() -> Result<(), String> {
    workspace_vault().check_writable()
}

/// The folder a sign-in set the old credentials aside in, until `dismiss_set_aside_notice`.
pub fn set_aside_notice() -> Option<String> {
    workspace_vault().set_aside_notice()
}

pub fn dismiss_set_aside_notice() -> Result<(), String> {
    workspace_vault().dismiss_set_aside_notice()
}

fn workspace_vault() -> Vault {
    Vault::with_key_store(crate::model::Store::data_dir(), KeyStore::of_this_system())
}

/// Where the key of the credentials lives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyStore {
    /// In clear in `credentials.key`, beside the credentials (systems without DPAPI).
    Portable,
    /// In `credentials.key.dpapi`, wrapped by Windows for the current user.
    Dpapi,
}

impl KeyStore {
    /// DPAPI on Windows, in both editions; the plain key elsewhere.
    pub fn of_this_system() -> Self {
        if cfg!(windows) { Self::Dpapi } else { Self::Portable }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum VaultStatus {
    /// The credentials open here. `protected`: Windows wraps their key for this user (or will,
    /// with the first credential saved). `transfer`: a transfer passphrase opens the same key
    /// on another Windows user or computer.
    Ready { protected: bool, transfer: bool },
    /// Windows cannot unwrap the key here: the data folder comes from another Windows user or
    /// computer. `transfer`: the transfer passphrase can open it.
    Locked { transfer: bool },
    /// No key opens the saved credentials, one of their files is damaged, or Windows cannot open
    /// the key right now (`code` `vault_unavailable` or `vault_storage`: try again, never start
    /// over). Nothing changed. `code` is one of `vault_codes`.
    Damaged { code: &'static str, message: String },
}

type Key = Zeroizing<[u8; 32]>;

pub struct Vault {
    root: PathBuf,
    store: KeyStore,
    kdf: KdfParams,
    /// Tests: the DPAPI check before a set-aside fails with this code.
    #[cfg(test)]
    broken_dpapi: Option<u32>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyDocument {
    version: u32,
    key: [u8; 32],
}

impl Drop for KeyDocument {
    fn drop(&mut self) {
        self.key.zeroize();
    }
}

/// `credentials.key.dpapi`: the DPAPI blob of the key and the key's check.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProtectedKeyDocument {
    version: u32,
    check: String,
    blob: Vec<u8>,
}

/// `credentials.key.transfer`: the key sealed with AES-256-GCM under an Argon2id derivation of
/// the transfer passphrase, with the parameters it was derived with. Never the passphrase.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TransferKeyDocument {
    version: u32,
    check: String,
    kdf: String,
    argon2_version: u32,
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
    salt: [u8; 16],
    nonce: [u8; 12],
    ciphertext: Vec<u8>,
}

impl TransferKeyDocument {
    fn clone_document(&self) -> Self {
        Self {
            version: self.version,
            check: self.check.clone(),
            kdf: self.kdf.clone(),
            argon2_version: self.argon2_version,
            memory_kib: self.memory_kib,
            iterations: self.iterations,
            parallelism: self.parallelism,
            salt: self.salt,
            nonce: self.nonce,
            ciphertext: self.ciphertext.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct KdfParams {
    memory_kib: u32,
    iterations: u32,
    parallelism: u32,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct EncryptedDocument {
    version: u32,
    nonce: [u8; 12],
    ciphertext: Vec<u8>,
}

struct Contents {
    key: Option<Key>,
    values: BTreeMap<String, String>,
    protected: bool,
    transfer: bool,
}

enum Loaded {
    Open(Contents),
    /// Windows refuses the key here, or it is missing; `transfer` is the copy of the same key
    /// that the transfer passphrase opens.
    Locked { transfer: Option<TransferKeyDocument> },
}

/// What `credentials.key.dpapi` (or its temporary file) holds.
enum Protected {
    Absent,
    /// Not a wrapped key: corrupt or unsupported file, not a DPAPI refusal.
    Damaged(&'static str),
    /// A well-formed file whose master key is not this user's: another user or computer.
    Refused { check: String },
    /// Windows did not unwrap it for another reason (damaged blob, profile not loaded): the code.
    Unavailable { code: u32 },
    Key(Key),
}

/// What `credentials.key.transfer` holds.
enum TransferCopy {
    Absent,
    /// A file that is not a transfer copy: reported, never a reason to set anything aside.
    Damaged,
    Copy(TransferKeyDocument),
}

impl TransferCopy {
    fn of(&self, check: &str) -> Option<&TransferKeyDocument> {
        match self {
            Self::Copy(copy) if copy.check == check => Some(copy),
            _ => None,
        }
    }
}

impl Vault {
    pub fn with_key_store(root: impl Into<PathBuf>, store: KeyStore) -> Self {
        Self {
            root: root.into(),
            store,
            kdf: TRANSFER_KDF,
            #[cfg(test)]
            broken_dpapi: None,
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
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

    fn empty(&self) -> Contents {
        Contents { key: None, values: BTreeMap::new(), protected: self.store == KeyStore::Dpapi, transfer: false }
    }

    fn load(&self) -> Result<Loaded, String> {
        self.finish_set_aside()?;
        let vault = read_vault(&self.path(VAULT_FILE))?;
        match self.store {
            KeyStore::Portable => {
                let key = read_plain_key(&self.path(PLAIN_KEY_FILE))?;
                let values = match (&vault, &key) {
                    (None, _) => BTreeMap::new(),
                    (Some(_), None) => return Err(PORTABLE_KEY_MISSING.into()),
                    (Some(vault), Some(key)) => unseal(vault, key)?,
                };
                Ok(Loaded::Open(Contents { key, values, protected: false, transfer: false }))
            }
            KeyStore::Dpapi => self.load_protected(vault),
        }
    }

    /// Picks the key that opens the credentials, in this order: the wrapped key, the plain key
    /// of an earlier version (wrapped now, then removed), a wrapped key left in the temporary
    /// file. A file is rewritten only with the same key, a wrapped key that opens nothing here is
    /// moved out of the way (never deleted), and a key file is removed only when the wrapped copy
    /// of the same key reads back and opens the credentials.
    fn load_protected(&self, vault: Option<EncryptedDocument>) -> Result<Loaded, String> {
        let opens = |key: &Key| vault.as_ref().map_or_else(|| Ok(BTreeMap::new()), |vault| unseal(vault, key));
        // A file that cannot be read now (held by another program) stops the load: only a file
        // read and found unusable may count as no key, since that can let a sign-in start over.
        let plain = parse_plain_key(read_optional(&self.path(PLAIN_KEY_FILE))?);
        let transfer = self.read_transfer()?;
        let matches = |key: &Key| transfer.of(&key_check(key)).is_some();
        let plain_key = plain.as_ref().ok().and_then(Option::as_ref);
        let plain_opening = plain_key.and_then(|key| opens(key).ok().map(|values| (key.clone(), values)));
        // The plain key opens, and the wrapped file beside it opens nothing here.
        let instead_of_stray = |(key, values): (Key, BTreeMap<String, String>)| {
            let protected = self.replace_stray_protected_key(&key, vault.as_ref());
            let transfer = matches(&key);
            Ok(Loaded::Open(Contents { key: Some(key), values, protected, transfer }))
        };

        match read_protected_key(&self.path(PROTECTED_KEY_FILE))? {
            Protected::Key(key) => match opens(&key) {
                Ok(values) => {
                    // An interrupted migration left the same key in clear: it can go now.
                    if plain_key.is_some_and(|plain| **plain == *key) {
                        let _ = fs::remove_file(self.path(PLAIN_KEY_FILE));
                    }
                    let _ = fs::remove_file(self.path(PROTECTED_KEY_TEMPORARY));
                    let transfer = matches(&key);
                    return Ok(Loaded::Open(Contents { key: Some(key), values, protected: true, transfer }));
                }
                Err(error) => return instead_of_stray(plain_opening.ok_or(error)?),
            },
            Protected::Refused { check } => {
                if let Some((key, values)) = plain_opening {
                    if key_check(&key) != check {
                        return instead_of_stray((key, values));
                    }
                    // Copied from another user or computer in the middle of a migration: the plain
                    // key still opens. Wrapping it here replaces only a copy of the same key.
                    let protected = self.protect_plain_key(&key, true, vault.as_ref()).is_ok();
                    let transfer = matches(&key);
                    return Ok(Loaded::Open(Contents { key: Some(key), values, protected, transfer }));
                }
                return match &transfer {
                    // A damaged transfer copy is not «no passphrase»: nothing is set aside for it.
                    TransferCopy::Damaged => Err(DAMAGED_TRANSFER.into()),
                    copy => Ok(Loaded::Locked { transfer: copy.of(&check).map(TransferKeyDocument::clone_document) }),
                };
            }
            Protected::Unavailable { code } => {
                // Maybe only for now: the wrapped file is left alone, and a plain key that opens
                // is used as it is until Windows opens the wrapped one again.
                let Some((key, values)) = plain_opening else { return Err(unavailable_message(code)) };
                let transfer = matches(&key);
                return Ok(Loaded::Open(Contents { key: Some(key), values, protected: false, transfer }));
            }
            Protected::Damaged(message) => return instead_of_stray(plain_opening.ok_or(message)?),
            Protected::Absent => {}
        }
        match plain {
            Err(message) => return Err(message),
            Ok(Some(key)) => {
                // A plain key that does not open the credentials is not wrapped.
                let values = opens(&key)?;
                // First load of this version: wrap the plain key. If Windows fails, the plain
                // key stays and the next load tries again.
                let protected = self.protect_plain_key(&key, false, vault.as_ref()).is_ok();
                let transfer = matches(&key);
                return Ok(Loaded::Open(Contents { key: Some(key), values, protected, transfer }));
            }
            Ok(None) => {}
        }
        if let Some(key) = self.promote_temporary(vault.as_ref())? {
            let values = opens(&key)?;
            let transfer = matches(&key);
            return Ok(Loaded::Open(Contents { key: Some(key), values, protected: true, transfer }));
        }
        match (&vault, transfer) {
            (None, _) => Ok(Loaded::Open(self.empty())),
            // The wrapped key was removed, but the transfer copy can open the credentials.
            (Some(_), TransferCopy::Copy(copy)) => Ok(Loaded::Locked { transfer: Some(copy) }),
            (Some(_), TransferCopy::Damaged) => Err(DAMAGED_TRANSFER.into()),
            (Some(_), TransferCopy::Absent) => Err(MISSING_PROTECTED_KEY.into()),
        }
    }

    /// The wrapped file opens nothing here (damaged, another vault's key, or another user's key
    /// of another vault) while the plain key opens the credentials: the file is renamed
    /// `credentials.key.dpapi.unused-<time>`, kept as it is, and the plain key is wrapped. A
    /// crash between the two leaves the plain key, which the next load wraps.
    fn replace_stray_protected_key(&self, key: &Key, vault: Option<&EncryptedDocument>) -> bool {
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
        let mut name = format!("{UNUSED_PROTECTED_KEY_PREFIX}{stamp}");
        let mut next = 2;
        while fs::symlink_metadata(self.path(&name)).is_ok() {
            name = format!("{UNUSED_PROTECTED_KEY_PREFIX}{stamp}-{next}");
            next += 1;
        }
        publish_file(&self.path(PROTECTED_KEY_FILE), &self.path(&name), false).is_ok()
            && self.protect_plain_key(key, false, vault).is_ok()
    }

    /// Wraps a plain key and removes it once the wrapped copy reads back and opens the
    /// credentials. On any failure the plain key stays in place.
    fn protect_plain_key(&self, key: &Key, replace: bool, vault: Option<&EncryptedDocument>) -> Result<(), String> {
        self.publish_protected(key, replace, vault)?;
        fs::remove_file(self.path(PLAIN_KEY_FILE)).map_err(storage_error)
    }

    /// Writes `credentials.key.dpapi` through its temporary file: written and flushed, renamed
    /// (replacing only when asked), then read back, unwrapped and checked against `vault`.
    fn publish_protected(&self, key: &Key, replace: bool, vault: Option<&EncryptedDocument>) -> Result<(), String> {
        let document = ProtectedKeyDocument { version: VERSION, check: key_check(key), blob: protect_bytes(&key[..])? };
        let bytes = serde_json::to_vec(&document).map_err(|error| error.to_string())?;
        let temporary = self.path(PROTECTED_KEY_TEMPORARY);
        let target = self.path(PROTECTED_KEY_FILE);
        write_synced(&temporary, &bytes)?;
        publish_file(&temporary, &target, replace).map_err(storage_error)?;
        match read_protected_key(&target)? {
            Protected::Key(back) if *back == **key && vault.is_none_or(|vault| unseal(vault, &back).is_ok()) => Ok(()),
            _ => Err(PROTECTION_NOT_VERIFIED.into()),
        }
    }

    /// A wrapped key left only in the temporary file (its plain key removed by hand): moved into
    /// place when it opens the credentials. A partial or foreign file stays where it is.
    fn promote_temporary(&self, vault: Option<&EncryptedDocument>) -> Result<Option<Key>, String> {
        let temporary = self.path(PROTECTED_KEY_TEMPORARY);
        let Protected::Key(key) = read_protected_key(&temporary)? else { return Ok(None) };
        if vault.is_some_and(|vault| unseal(vault, &key).is_err()) {
            return Ok(None);
        }
        publish_file(&temporary, &self.path(PROTECTED_KEY_FILE), false).map_err(storage_error)?;
        Ok(Some(key))
    }

    fn read_transfer(&self) -> Result<TransferCopy, String> {
        let Some(bytes) = read_optional(&self.path(TRANSFER_KEY_FILE))? else { return Ok(TransferCopy::Absent) };
        Ok(serde_json::from_slice::<TransferKeyDocument>(&bytes)
            .ok()
            .filter(|document| document.version == VERSION && document.kdf == ARGON2ID)
            .map_or(TransferCopy::Damaged, TransferCopy::Copy))
    }

    fn create_key(&self) -> Result<Key, String> {
        let key = Zeroizing::new(random_bytes::<32>()?);
        match self.store {
            KeyStore::Portable => {
                let document = KeyDocument { version: VERSION, key: *key };
                let bytes = Zeroizing::new(serde_json::to_vec(&document).map_err(|error| error.to_string())?);
                publish(&self.path(PLAIN_KEY_FILE), &bytes, false)?;
            }
            // Never in clear, not even for a moment.
            KeyStore::Dpapi => self.publish_protected(&key, false, None)?,
        }
        Ok(key)
    }

    pub fn get(&self, name: &str) -> Result<String, String> {
        let _lock = self.lock()?;
        match self.load()? {
            Loaded::Open(contents) => Ok(contents.values.get(name).cloned().unwrap_or_default()),
            Loaded::Locked { transfer } => Err(locked_message(transfer.is_some()).into()),
        }
    }

    pub fn set(&self, name: &str, value: &str) -> Result<(), String> {
        let _lock = self.lock()?;
        let mut contents = match self.load()? {
            Loaded::Open(contents) => contents,
            // Removing a credential that cannot be read here changes nothing.
            Loaded::Locked { .. } if value.is_empty() => return Ok(()),
            Loaded::Locked { transfer: Some(_) } => return Err(LOCKED_WITH_TRANSFER.into()),
            // Owner ruling R1: without a transfer passphrase, the credentials of another user or
            // computer give way when the person signs in again. Set aside, never deleted.
            Loaded::Locked { transfer: None } => {
                let folder = self.set_aside()?;
                // For the interface to tell the person where the old credentials went; losing
                // this note loses only the notice.
                let _ = publish(&self.path(SET_ASIDE_NOTICE), folder.as_bytes(), true);
                self.empty()
            }
        };
        if value.is_empty() {
            if contents.values.remove(name).is_none() {
                return Ok(());
            }
        } else {
            contents.values.insert(name.to_string(), value.to_string());
        }
        let key = match contents.key.take() {
            Some(key) => key,
            None => self.create_key()?,
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
            .map_err(|_| SEALING_FAILED.to_string())?;
        let document = EncryptedDocument {
            version: VERSION,
            nonce,
            ciphertext,
        };
        let bytes = serde_json::to_vec(&document).map_err(|error| error.to_string())?;
        publish(&self.path(VAULT_FILE), &bytes, true)
    }

    pub fn status(&self) -> VaultStatus {
        let _lock = match self.lock() {
            Ok(lock) => lock,
            Err(message) => return VaultStatus::Damaged { code: error_code(&message), message },
        };
        match self.load() {
            Ok(Loaded::Open(contents)) => VaultStatus::Ready { protected: contents.protected, transfer: contents.transfer },
            Ok(Loaded::Locked { transfer }) => VaultStatus::Locked { transfer: transfer.is_some() },
            Err(message) => VaultStatus::Damaged { code: error_code(&message), message },
        }
    }

    /// Changing or removing the passphrase needs the key open here: the old passphrase is not
    /// asked for, Windows already vouches for this user.
    pub fn set_transfer_passphrase(&self, passphrase: &str) -> Result<VaultStatus, String> {
        let passphrase = transfer_passphrase(passphrase, true)?;
        self.require_dpapi()?;
        let _lock = self.lock()?;
        let Loaded::Open(contents) = self.load()? else { return Err(NEEDS_UNLOCKED.into()) };
        let protected = contents.protected;
        let key = match contents.key {
            Some(key) => key,
            None => self.create_key()?,
        };
        let document = wrap_transfer(&key, &passphrase, self.kdf)?;
        // The copy must open with this passphrase before it replaces the previous one.
        if *unwrap_transfer(&document, &passphrase)? != *key {
            return Err(DAMAGED_TRANSFER.into());
        }
        let bytes = serde_json::to_vec(&document).map_err(|error| error.to_string())?;
        let path = self.path(TRANSFER_KEY_FILE);
        publish(&path, &bytes, true)?;
        if fs::read(&path).map_err(storage_error)? != bytes {
            return Err(DAMAGED_TRANSFER.into());
        }
        Ok(VaultStatus::Ready { protected, transfer: true })
    }

    pub fn remove_transfer_passphrase(&self) -> Result<VaultStatus, String> {
        self.require_dpapi()?;
        let _lock = self.lock()?;
        let Loaded::Open(contents) = self.load()? else { return Err(NEEDS_UNLOCKED.into()) };
        match fs::remove_file(self.path(TRANSFER_KEY_FILE)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(storage_error(error)),
        }
        Ok(VaultStatus::Ready { protected: contents.protected, transfer: false })
    }

    pub fn unlock_with_transfer_passphrase(&self, passphrase: &str) -> Result<VaultStatus, String> {
        let passphrase = transfer_passphrase(passphrase, false)?;
        self.require_dpapi()?;
        let _lock = self.lock()?;
        let transfer = match self.load()? {
            // Another window may have opened it already.
            Loaded::Open(contents) => return Ok(VaultStatus::Ready { protected: contents.protected, transfer: contents.transfer }),
            Loaded::Locked { transfer: None } => return Err(NO_TRANSFER.into()),
            Loaded::Locked { transfer: Some(transfer) } => transfer,
        };
        let key = unwrap_transfer(&transfer, &passphrase)?;
        if key_check(&key) != transfer.check {
            return Err(DAMAGED_TRANSFER.into());
        }
        let vault = read_vault(&self.path(VAULT_FILE))?;
        if let Some(vault) = &vault {
            unseal(vault, &key).map_err(|_| TRANSFER_MISMATCH.to_string())?;
        }
        // Locked means the wrapped file, if any, holds this same key for another user: wrapping
        // it again for this one replaces a copy of the same key.
        let replace = self.path(PROTECTED_KEY_FILE).exists();
        self.publish_protected(&key, replace, vault.as_ref())?;
        Ok(VaultStatus::Ready { protected: true, transfer: true })
    }

    /// Only for credentials that do not open: locked to another user, or damaged. A file that
    /// cannot be read right now is not damaged, and starting over then would drop good ones. A key
    /// that Windows cannot open now (`vault_unavailable`) may be good too: it is set aside only
    /// with `unopenable_too`, when trying again did not help and the person confirmed. Setting
    /// aside only moves files, so it can be undone by moving them back.
    pub fn start_over(&self, unopenable_too: bool) -> Result<String, String> {
        let _lock = self.lock()?;
        match self.load() {
            Ok(Loaded::Open(_)) => Err(NOTHING_TO_START_OVER.into()),
            Err(message) if error_code(&message) == vault_codes::STORAGE => Err(message),
            Err(message) if error_code(&message) == vault_codes::UNAVAILABLE && !unopenable_too => Err(message),
            Ok(Loaded::Locked { .. }) | Err(_) => self.set_aside(),
        }
    }

    pub fn check_writable(&self) -> Result<(), String> {
        let _lock = self.lock()?;
        match self.load()? {
            Loaded::Open(_) | Loaded::Locked { transfer: None } => Ok(()),
            Loaded::Locked { transfer: Some(_) } => Err(LOCKED_WITH_TRANSFER.into()),
        }
    }

    pub fn set_aside_notice(&self) -> Option<String> {
        let bytes = read_optional(&self.path(SET_ASIDE_NOTICE)).ok()??;
        String::from_utf8(bytes).ok().filter(|name| valid_set_aside_name(name))
    }

    pub fn dismiss_set_aside_notice(&self) -> Result<(), String> {
        match fs::remove_file(self.path(SET_ASIDE_NOTICE)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(storage_error(error)),
        }
    }

    fn require_dpapi(&self) -> Result<(), String> {
        if self.store == KeyStore::Dpapi { Ok(()) } else { Err(TRANSFER_NEEDS_DPAPI.into()) }
    }

    /// Moves the credentials and their keys into a new folder of the data folder. The marker
    /// written first names that folder, so a crash halfway is finished at the next load.
    fn set_aside(&self) -> Result<String, String> {
        // A new key is written right after: while Windows cannot protect one, nothing moves.
        if self.store == KeyStore::Dpapi {
            self.check_dpapi()?;
        }
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
        let mut name = format!("{SET_ASIDE_PREFIX}{stamp}");
        let mut next = 2;
        while fs::symlink_metadata(self.path(&name)).is_ok() {
            name = format!("{SET_ASIDE_PREFIX}{stamp}-{next}");
            next += 1;
        }
        publish(&self.path(SET_ASIDE_MARKER), name.as_bytes(), false)?;
        self.move_aside(&name)?;
        fs::remove_file(self.path(SET_ASIDE_MARKER)).map_err(storage_error)?;
        Ok(name)
    }

    /// Windows protects and opens a test value for this user right now.
    fn check_dpapi(&self) -> Result<(), String> {
        #[cfg(test)]
        if let Some(code) = self.broken_dpapi {
            return Err(unavailable_message(code));
        }
        dpapi_round_trip().map_err(unavailable_message)
    }

    fn finish_set_aside(&self) -> Result<(), String> {
        let marker = self.path(SET_ASIDE_MARKER);
        let Some(bytes) = read_optional(&marker)? else { return Ok(()) };
        let name = String::from_utf8(bytes)
            .ok()
            .filter(|name| valid_set_aside_name(name))
            .ok_or(DAMAGED_SET_ASIDE_MARKER)?;
        self.move_aside(&name)?;
        fs::remove_file(&marker).map_err(storage_error)
    }

    fn move_aside(&self, name: &str) -> Result<(), String> {
        let folder = self.path(name);
        match fs::create_dir(&folder) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(storage_error(error)),
        }
        for file in SET_ASIDE_FILES {
            let source = self.path(file);
            let target = folder.join(file);
            // Already moved, or never there. A name in both places is left in both.
            if fs::symlink_metadata(&source).is_err() || fs::symlink_metadata(&target).is_ok() {
                continue;
            }
            publish_file(&source, &target, false).map_err(storage_error)?;
        }
        Ok(())
    }
}

fn valid_set_aside_name(name: &str) -> bool {
    name.len() <= 64
        && name
            .strip_prefix(SET_ASIDE_PREFIX)
            .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit() || c == '-'))
}

fn locked_message(transfer: bool) -> &'static str {
    if transfer { LOCKED_WITH_TRANSFER } else { FOREIGN_KEY }
}

fn read_vault(path: &Path) -> Result<Option<EncryptedDocument>, String> {
    let Some(bytes) = read_optional(path)? else { return Ok(None) };
    let document: EncryptedDocument = serde_json::from_slice(&bytes).map_err(|_| VAULT_UNREADABLE.to_string())?;
    if document.version != VERSION {
        return Err(VAULT_UNSUPPORTED.into());
    }
    Ok(Some(document))
}

fn read_plain_key(path: &Path) -> Result<Option<Key>, String> {
    parse_plain_key(read_optional(path)?)
}

fn parse_plain_key(bytes: Option<Vec<u8>>) -> Result<Option<Key>, String> {
    let Some(bytes) = bytes else { return Ok(None) };
    let bytes = Zeroizing::new(bytes);
    let document: KeyDocument = serde_json::from_slice(&bytes).map_err(|_| PLAIN_KEY_DAMAGED.to_string())?;
    if document.version != VERSION {
        return Err(PLAIN_KEY_UNSUPPORTED.into());
    }
    Ok(Some(Zeroizing::new(document.key)))
}

/// Reads a wrapped key file. `Err` only when the file cannot be read at all.
fn read_protected_key(path: &Path) -> Result<Protected, String> {
    let Some(bytes) = read_optional(path)? else { return Ok(Protected::Absent) };
    let Ok(document) = serde_json::from_slice::<ProtectedKeyDocument>(&bytes) else {
        return Ok(Protected::Damaged(DAMAGED_PROTECTED_KEY));
    };
    if document.version != VERSION {
        return Ok(Protected::Damaged(UNSUPPORTED_PROTECTED_KEY));
    }
    let bytes = match unprotect_bytes(&document.blob) {
        Ok(bytes) => bytes,
        Err(refusal) if refusal.foreign => return Ok(Protected::Refused { check: document.check }),
        Err(refusal) => {
            // The code only: never key material.
            eprintln!("credentials key: Windows DPAPI did not open {} (error 0x{:08X})", path.display(), refusal.code);
            return Ok(Protected::Unavailable { code: refusal.code });
        }
    };
    // Windows unwrapped it, so these are the bytes that were wrapped: anything but a key with
    // its own check is a damaged file, not another user's.
    let Ok(key) = <[u8; 32]>::try_from(bytes.as_slice()) else {
        return Ok(Protected::Damaged(DAMAGED_PROTECTED_KEY));
    };
    let key = Zeroizing::new(key);
    if key_check(&key) != document.check {
        return Ok(Protected::Damaged(DAMAGED_PROTECTED_KEY));
    }
    Ok(Protected::Key(key))
}

fn unseal(document: &EncryptedDocument, key: &[u8; 32]) -> Result<BTreeMap<String, String>, String> {
    let cipher = encryption_key(key)?;
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
        .ok_or_else(|| VAULT_NOT_AUTHENTIC.to_string())?;
    serde_json::from_slice(&plaintext).map_err(|_| VAULT_CONTENT_INVALID.to_string())
}

fn key_check(key: &[u8; 32]) -> String {
    let mut context = digest::Context::new(&digest::SHA256);
    context.update(KEY_CHECK_DOMAIN);
    context.update(key);
    context.finish().as_ref()[..16].iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Why Windows did not unwrap a key: `foreign` for a key of another user or computer.
struct Refusal {
    foreign: bool,
    code: u32,
}

#[cfg(windows)]
fn protect_bytes(bytes: &[u8]) -> Result<Vec<u8>, String> {
    super::dpapi::protect(bytes).map_err(|error| error.to_string())
}

#[cfg(windows)]
fn unprotect_bytes(bytes: &[u8]) -> Result<Zeroizing<Vec<u8>>, Refusal> {
    super::dpapi::unprotect(bytes)
        .map(Zeroizing::new)
        .map_err(|error| Refusal { foreign: error.foreign(), code: error.code })
}

#[cfg(windows)]
fn dpapi_round_trip() -> Result<(), u32> {
    const PROBE: &[u8] = b"Qamille: can Windows protect a key now?";
    let blob = super::dpapi::protect(PROBE).map_err(|error| error.code)?;
    let back = Zeroizing::new(super::dpapi::unprotect(&blob).map_err(|error| error.code)?);
    // ERROR_INVALID_DATA: what came back is not what went in.
    if back.as_slice() == PROBE { Ok(()) } else { Err(0x8007_000D) }
}

#[cfg(not(windows))]
fn dpapi_round_trip() -> Result<(), u32> {
    Err(0)
}

#[cfg(not(windows))]
fn protect_bytes(_bytes: &[u8]) -> Result<Vec<u8>, String> {
    Err(NO_DPAPI.into())
}

#[cfg(not(windows))]
fn unprotect_bytes(_bytes: &[u8]) -> Result<Zeroizing<Vec<u8>>, Refusal> {
    Err(Refusal { foreign: false, code: 0 })
}

/// The passphrase in Unicode NFC, so the same words typed on another keyboard give the same
/// key. A new passphrase needs `MIN_TRANSFER_PASSPHRASE_CHARS` characters that are not spaces.
fn transfer_passphrase(passphrase: &str, new: bool) -> Result<Zeroizing<String>, String> {
    // Room for the worst NFC growth up front: a reallocation would leave a copy behind.
    let mut normalized = Zeroizing::new(String::with_capacity(passphrase.len() * 3 + 16));
    normalized.extend(passphrase.nfc());
    if normalized.len() > MAX_TRANSFER_PASSPHRASE_BYTES {
        return Err(LONG_PASSPHRASE.into());
    }
    if new && normalized.chars().filter(|c| !c.is_whitespace()).count() < MIN_TRANSFER_PASSPHRASE_CHARS {
        return Err(SHORT_PASSPHRASE.into());
    }
    Ok(normalized)
}

fn derive_transfer_key(passphrase: &str, salt: &[u8; 16], kdf: KdfParams) -> Result<Key, String> {
    let bounded = (1..=MAX_MEMORY_KIB).contains(&kdf.memory_kib)
        && (1..=MAX_ITERATIONS).contains(&kdf.iterations)
        && (1..=MAX_PARALLELISM).contains(&kdf.parallelism);
    if !bounded {
        return Err(DAMAGED_TRANSFER.into());
    }
    let params = argon2::Params::new(kdf.memory_kib, kdf.iterations, kdf.parallelism, Some(32))
        .map_err(|_| DAMAGED_TRANSFER.to_string())?;
    let blocks = params.block_count();
    let argon2 = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    // Asked for first: a failed allocation is an error, not the end of the program.
    let mut memory: Vec<argon2::Block> = Vec::new();
    memory.try_reserve_exact(blocks).map_err(|_| NO_MEMORY_FOR_TRANSFER.to_string())?;
    memory.resize(blocks, argon2::Block::default());
    let mut output = Zeroizing::new([0u8; 32]);
    let derived = argon2.hash_password_into_with_memory(passphrase.as_bytes(), salt, &mut output[..], &mut memory);
    memory.zeroize();
    derived.map_err(|_| DERIVATION_FAILED.to_string())?;
    Ok(output)
}

fn wrap_transfer(key: &Key, passphrase: &str, kdf: KdfParams) -> Result<TransferKeyDocument, String> {
    let salt = random_bytes::<16>()?;
    let nonce = random_bytes::<12>()?;
    let wrapping = derive_transfer_key(passphrase, &salt, kdf)?;
    let mut ciphertext = key.to_vec();
    encryption_key(&wrapping)?
        .seal_in_place_append_tag(aead::Nonce::assume_unique_for_key(nonce), aead::Aad::from(TRANSFER_FORMAT), &mut ciphertext)
        .map_err(|_| SEALING_FAILED.to_string())?;
    Ok(TransferKeyDocument {
        version: VERSION,
        check: key_check(key),
        kdf: ARGON2ID.into(),
        argon2_version: ARGON2_VERSION,
        memory_kib: kdf.memory_kib,
        iterations: kdf.iterations,
        parallelism: kdf.parallelism,
        salt,
        nonce,
        ciphertext,
    })
}

fn unwrap_transfer(document: &TransferKeyDocument, passphrase: &str) -> Result<Key, String> {
    if document.kdf != ARGON2ID || document.argon2_version != ARGON2_VERSION {
        return Err(DAMAGED_TRANSFER.into());
    }
    let kdf = KdfParams { memory_kib: document.memory_kib, iterations: document.iterations, parallelism: document.parallelism };
    let wrapping = derive_transfer_key(passphrase, &document.salt, kdf)?;
    let mut sealed = Zeroizing::new(document.ciphertext.clone());
    let opened = encryption_key(&wrapping)?
        .open_in_place(aead::Nonce::assume_unique_for_key(document.nonce), aead::Aad::from(TRANSFER_FORMAT), &mut sealed)
        .map_err(|_| WRONG_PASSPHRASE.to_string())?;
    let key = <[u8; 32]>::try_from(&*opened).map_err(|_| DAMAGED_TRANSFER.to_string())?;
    Ok(Zeroizing::new(key))
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
        .map_err(|_| NO_RANDOMNESS.to_string())?;
    Ok(bytes)
}

fn encryption_key(key: &[u8; 32]) -> Result<aead::LessSafeKey, String> {
    aead::UnboundKey::new(&aead::AES_256_GCM, key)
        .map(aead::LessSafeKey::new)
        .map_err(|_| INVALID_KEY.to_string())
}

fn storage_error(error: std::io::Error) -> String {
    format!("{STORAGE_FAILED} {error}")
}

/// Writes `bytes` to `path`, replacing it, and flushes them to disk.
fn write_synced(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .map_err(storage_error)?;
    file.write_all(bytes).map_err(storage_error)?;
    file.sync_all().map_err(storage_error)
}

fn publish(path: &Path, bytes: &[u8], replace: bool) -> Result<(), String> {
    let suffix: String = random_bytes::<12>()?
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let name = path
        .file_name()
        .ok_or(INVALID_PATH)?
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

    /// Cheap Argon2id for the tests that only follow the files; one test uses `TRANSFER_KDF`.
    const TEST_KDF: KdfParams = KdfParams { memory_kib: 256, iterations: 1, parallelism: 1 };

    impl Vault {
        fn with_transfer_kdf(mut self, kdf: KdfParams) -> Self {
            self.kdf = kdf;
            self
        }
    }

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

        /// The plain key of the portable store, as every version before DPAPI wrote it.
        fn vault(&self) -> Vault {
            Vault::with_key_store(&self.0, KeyStore::Portable)
        }

        /// The key wrapped by Windows for this user.
        fn protected(&self) -> Vault {
            Vault::with_key_store(&self.0, KeyStore::Dpapi).with_transfer_kdf(TEST_KDF)
        }

        fn path(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }

        fn exists(&self, name: &str) -> bool {
            self.path(name).exists()
        }

        fn read(&self, name: &str) -> Vec<u8> {
            fs::read(self.path(name)).unwrap()
        }

        /// Every file under the data folder with its bytes, but the lock file.
        fn files(&self) -> BTreeMap<String, Vec<u8>> {
            fn walk(root: &Path, folder: &Path, files: &mut BTreeMap<String, Vec<u8>>) {
                for entry in fs::read_dir(folder).unwrap() {
                    let path = entry.unwrap().path();
                    if path.is_dir() {
                        walk(root, &path, files);
                    } else if path.file_name().unwrap() != "credentials.lock" {
                        let name = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
                        files.insert(name, fs::read(&path).unwrap());
                    }
                }
            }
            let mut files = BTreeMap::new();
            walk(&self.0, &self.0, &mut files);
            files
        }

        fn set_aside_folders(&self) -> Vec<String> {
            fs::read_dir(&self.0)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .filter(|name| name.starts_with(SET_ASIDE_PREFIX))
                .collect()
        }

        fn plain_key(&self) -> [u8; 32] {
            let document: KeyDocument = serde_json::from_slice(&self.read(PLAIN_KEY_FILE)).unwrap();
            document.key
        }

        fn protected_document(&self) -> ProtectedKeyDocument {
            serde_json::from_slice(&self.read(PROTECTED_KEY_FILE)).unwrap()
        }

        /// What another Windows user, or this one on another computer, finds: a well-formed file
        /// whose blob names a DPAPI master key this user does not have. Bytes 24..40 of a DPAPI
        /// blob are the GUID of its master key.
        fn as_if_from_another_user(&self) {
            let mut document = self.protected_document();
            document.blob[24] ^= 0xFF;
            fs::write(self.path(PROTECTED_KEY_FILE), serde_json::to_vec(&document).unwrap()).unwrap();
            assert!(super::super::dpapi::unprotect(&document.blob).is_err());
        }

        /// The data an earlier version left: values sealed under a plain `credentials.key`, and
        /// files of mail and settings that the vault must never touch.
        fn earlier_install(&self) -> [u8; 32] {
            self.vault().set("gmail:old@example.test", "old-refresh-token").unwrap();
            self.vault().set(AI_KEY, "old-ai-key").unwrap();
            self.other_data();
            self.plain_key()
        }

        fn other_data(&self) {
            fs::write(self.path("store.json"), b"{\"settings\":\"kept\"}").unwrap();
            fs::create_dir_all(self.path("mail")).unwrap();
            fs::write(self.path("mail/1.eml"), b"Subject: kept").unwrap();
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

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|window| window == needle)
    }

    fn assert_other_data_kept(workspace: &Workspace) {
        assert_eq!(workspace.read("store.json"), b"{\"settings\":\"kept\"}");
        assert_eq!(workspace.read("mail/1.eml"), b"Subject: kept");
    }

    #[test]
    fn vault_key_moves_to_dpapi_once_and_stays_readable() {
        let workspace = Workspace::new();
        let plain = workspace.earlier_install();
        let vault = workspace.protected();
        assert_eq!(vault.get("gmail:old@example.test").unwrap(), "old-refresh-token");
        assert!(!workspace.exists(PLAIN_KEY_FILE), "the plain key goes once the wrapped one reads back");
        assert!(!workspace.exists(PROTECTED_KEY_TEMPORARY));
        let wrapped = workspace.read(PROTECTED_KEY_FILE);
        assert!(!contains(&wrapped, &plain));
        assert!(!contains(&wrapped, serde_json::to_string(&plain).unwrap().as_bytes()));
        let document = workspace.protected_document();
        assert_eq!(*Zeroizing::new(super::super::dpapi::unprotect(&document.blob).unwrap()), plain);
        assert_eq!(document.check, key_check(&plain));
        // Later loads read the wrapped key and rewrite nothing.
        assert_eq!(vault.get(AI_KEY).unwrap(), "old-ai-key");
        vault.set("contacts:1", "new-token").unwrap();
        assert_eq!(workspace.read(PROTECTED_KEY_FILE), wrapped);
        assert_eq!(vault.get("contacts:1").unwrap(), "new-token");
        assert_eq!(vault.get("gmail:old@example.test").unwrap(), "old-refresh-token");
        assert!(!workspace.exists(PLAIN_KEY_FILE));
        assert_eq!(vault.status(), VaultStatus::Ready { protected: true, transfer: false });
        assert_other_data_kept(&workspace);
    }

    #[test]
    fn vault_key_interrupted_after_rename_is_finished_at_the_next_load() {
        let workspace = Workspace::new();
        let plain = Zeroizing::new(workspace.earlier_install());
        // Steps 1 to 3 done, the plain key not yet removed.
        workspace.protected().publish_protected(&plain, false, None).unwrap();
        let wrapped = workspace.read(PROTECTED_KEY_FILE);
        assert!(workspace.exists(PLAIN_KEY_FILE));
        assert_eq!(workspace.protected().get("gmail:old@example.test").unwrap(), "old-refresh-token");
        assert!(!workspace.exists(PLAIN_KEY_FILE));
        assert_eq!(workspace.read(PROTECTED_KEY_FILE), wrapped);
        assert_other_data_kept(&workspace);
    }

    #[test]
    fn vault_key_interrupted_before_rename_keeps_the_plain_key() {
        for complete in [false, true] {
            let workspace = Workspace::new();
            let plain = Zeroizing::new(workspace.earlier_install());
            // Step 1 under way (half of the file) or done (the whole file), never renamed.
            let protected = workspace.protected();
            protected.publish_protected(&plain, false, None).unwrap();
            let whole = workspace.read(PROTECTED_KEY_FILE);
            fs::remove_file(workspace.path(PROTECTED_KEY_FILE)).unwrap();
            let written = if complete { whole.clone() } else { whole[..whole.len() / 2].to_vec() };
            fs::write(workspace.path(PROTECTED_KEY_TEMPORARY), &written).unwrap();
            assert_eq!(protected.get("gmail:old@example.test").unwrap(), "old-refresh-token");
            // The temporary file was written again and moved into place; the plain key went after.
            assert!(!workspace.exists(PROTECTED_KEY_TEMPORARY));
            assert!(!workspace.exists(PLAIN_KEY_FILE));
            assert_eq!(protected.status(), VaultStatus::Ready { protected: true, transfer: false });
            assert_eq!(protected.get(AI_KEY).unwrap(), "old-ai-key");
        }
    }

    #[test]
    fn vault_key_left_only_in_the_temporary_file_is_used() {
        let workspace = Workspace::new();
        let plain = Zeroizing::new(workspace.earlier_install());
        let protected = workspace.protected();
        protected.publish_protected(&plain, false, None).unwrap();
        fs::rename(workspace.path(PROTECTED_KEY_FILE), workspace.path(PROTECTED_KEY_TEMPORARY)).unwrap();
        fs::remove_file(workspace.path(PLAIN_KEY_FILE)).unwrap();
        assert_eq!(protected.get("gmail:old@example.test").unwrap(), "old-refresh-token");
        assert!(workspace.exists(PROTECTED_KEY_FILE));
        assert!(!workspace.exists(PROTECTED_KEY_TEMPORARY));
    }

    #[test]
    fn vault_key_from_another_user_is_refused_and_nothing_is_deleted() {
        let workspace = Workspace::new();
        workspace.earlier_install();
        let protected = workspace.protected();
        assert_eq!(protected.get(AI_KEY).unwrap(), "old-ai-key");
        workspace.as_if_from_another_user();
        let before = workspace.files();
        assert_eq!(protected.get("gmail:old@example.test").unwrap_err(), FOREIGN_KEY);
        assert_eq!(protected.status(), VaultStatus::Locked { transfer: false });
        // Signing out of an account that cannot be read here changes nothing.
        protected.set("gmail:old@example.test", "").unwrap();
        assert_eq!(protected.unlock_with_transfer_passphrase("any passphrase").unwrap_err(), NO_TRANSFER);
        assert_eq!(workspace.files(), before);
        assert_other_data_kept(&workspace);
    }

    #[test]
    fn vault_key_from_another_user_is_set_aside_when_the_person_signs_in_again() {
        let workspace = Workspace::new();
        workspace.earlier_install();
        let protected = workspace.protected();
        protected.get(AI_KEY).unwrap();
        workspace.as_if_from_another_user();
        let before = workspace.files();
        protected.set("gmail:new@example.test", "fresh-token").unwrap();
        assert_eq!(protected.get("gmail:new@example.test").unwrap(), "fresh-token");
        assert_eq!(protected.get("gmail:old@example.test").unwrap(), "");
        assert_eq!(protected.status(), VaultStatus::Ready { protected: true, transfer: false });
        // The old credentials and their key are in one folder, byte for byte.
        let folders = workspace.set_aside_folders();
        assert_eq!(folders.len(), 1);
        let after = workspace.files();
        for name in [VAULT_FILE, PROTECTED_KEY_FILE] {
            assert_eq!(after[&format!("{}/{name}", folders[0])], before[name], "{name}");
        }
        assert!(!workspace.exists(SET_ASIDE_MARKER));
        // The interface tells the person where the old ones went, once.
        assert_eq!(protected.set_aside_notice().as_deref(), Some(folders[0].as_str()));
        protected.dismiss_set_aside_notice().unwrap();
        assert_eq!(protected.set_aside_notice(), None);
        // The new credentials are this user's: the next sign-ins add to them, nothing more is set aside.
        protected.set("gmail:second@example.test", "second-token").unwrap();
        protected.set(AI_KEY, "new-ai-key").unwrap();
        assert_eq!(workspace.set_aside_folders().len(), 1);
        assert_eq!(protected.get("gmail:new@example.test").unwrap(), "fresh-token");
        assert_other_data_kept(&workspace);
    }

    #[test]
    fn vault_key_damaged_file_is_not_taken_for_another_user() {
        for damage in [&b"{damaged"[..], br#"{"version":2,"check":"00","blob":[1,2,3]}"#] {
            let workspace = Workspace::new();
            workspace.other_data();
            let protected = workspace.protected();
            protected.set(AI_KEY, "original").unwrap();
            fs::write(workspace.path(PROTECTED_KEY_FILE), damage).unwrap();
            let before = workspace.files();
            let error = protected.get(AI_KEY).unwrap_err();
            assert!(error.starts_with("The protected credentials key"), "{error}");
            assert!(matches!(protected.status(), VaultStatus::Damaged { .. }));
            // Unlike a key of another user, a damaged file never gives way to a new sign-in.
            assert!(protected.set(AI_KEY, "replacement").is_err());
            assert_eq!(workspace.files(), before);
            assert!(workspace.set_aside_folders().is_empty());
        }
    }

    #[test]
    fn vault_key_missing_dpapi_blob_is_an_error_without_a_replacement() {
        let workspace = Workspace::new();
        let protected = workspace.protected();
        protected.set(AI_KEY, "original").unwrap();
        fs::remove_file(workspace.path(PROTECTED_KEY_FILE)).unwrap();
        let before = workspace.files();
        assert_eq!(protected.get(AI_KEY).unwrap_err(), MISSING_PROTECTED_KEY);
        assert!(protected.set(AI_KEY, "replacement").is_err());
        assert_eq!(workspace.files(), before);
        assert!(!workspace.exists(PROTECTED_KEY_FILE));
    }

    #[test]
    fn vault_key_that_does_not_open_the_credentials_is_not_wrapped() {
        let first = Workspace::new();
        let second = Workspace::new();
        first.vault().set(AI_KEY, "first credential").unwrap();
        second.vault().set(AI_KEY, "second credential").unwrap();
        fs::copy(second.path(PLAIN_KEY_FILE), first.path(PLAIN_KEY_FILE)).unwrap();
        let before = first.files();
        assert!(first.protected().get(AI_KEY).is_err());
        assert!(first.protected().set(AI_KEY, "replacement").is_err());
        assert_eq!(first.files(), before);
    }

    #[test]
    fn vault_key_new_vault_never_writes_a_plain_key() {
        let workspace = Workspace::new();
        let protected = workspace.protected();
        assert_eq!(protected.status(), VaultStatus::Ready { protected: true, transfer: false });
        protected.set(AI_KEY, "secret").unwrap();
        let names: Vec<_> = workspace.files().into_keys().collect();
        assert_eq!(names, [VAULT_FILE, PROTECTED_KEY_FILE]);
        assert_eq!(protected.get(AI_KEY).unwrap(), "secret");
    }

    #[test]
    fn vault_key_copied_with_its_plain_key_is_wrapped_again_here() {
        let workspace = Workspace::new();
        let plain = Zeroizing::new(workspace.earlier_install());
        // A folder copied from another user in the middle of its migration.
        workspace.protected().publish_protected(&plain, false, None).unwrap();
        workspace.as_if_from_another_user();
        assert!(workspace.exists(PLAIN_KEY_FILE));
        let protected = workspace.protected();
        assert_eq!(protected.get("gmail:old@example.test").unwrap(), "old-refresh-token");
        assert!(super::super::dpapi::unprotect(&workspace.protected_document().blob).is_ok());
        assert!(!workspace.exists(PLAIN_KEY_FILE));
        assert_eq!(protected.status(), VaultStatus::Ready { protected: true, transfer: false });
    }

    #[test]
    fn vault_key_migrates_once_under_concurrent_loads() {
        let workspace = Workspace::new();
        workspace.earlier_install();
        let handles: Vec<_> = (0..4)
            .map(|worker| {
                let root = workspace.0.clone();
                std::thread::spawn(move || {
                    let vault = Vault::with_key_store(&root, KeyStore::Dpapi);
                    assert_eq!(vault.get("gmail:old@example.test").unwrap(), "old-refresh-token");
                    vault.set(&format!("worker:{worker}"), "value").unwrap();
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        let protected = workspace.protected();
        for worker in 0..4 {
            assert_eq!(protected.get(&format!("worker:{worker}")).unwrap(), "value");
        }
        assert!(!workspace.exists(PLAIN_KEY_FILE));
        assert!(!workspace.exists(PROTECTED_KEY_TEMPORARY));
    }

    #[test]
    fn vault_key_transfer_passphrase_opens_the_credentials_for_another_user() {
        let workspace = Workspace::new();
        let plain = workspace.earlier_install();
        let protected = workspace.protected();
        let passphrase = "correct horse battery staple";
        assert_eq!(
            protected.set_transfer_passphrase(passphrase).unwrap(),
            VaultStatus::Ready { protected: true, transfer: true }
        );
        let copy = workspace.read(TRANSFER_KEY_FILE);
        assert!(!contains(&copy, passphrase.as_bytes()));
        assert!(!contains(&copy, &plain));
        workspace.as_if_from_another_user();
        assert_eq!(protected.status(), VaultStatus::Locked { transfer: true });
        let before = workspace.files();
        assert_eq!(protected.get(AI_KEY).unwrap_err(), LOCKED_WITH_TRANSFER);
        // With a passphrase to try, a new sign-in does not throw the credentials away.
        assert_eq!(protected.set("gmail:new@example.test", "fresh").unwrap_err(), LOCKED_WITH_TRANSFER);
        assert_eq!(protected.unlock_with_transfer_passphrase("wrong horse battery staple").unwrap_err(), WRONG_PASSPHRASE);
        assert_eq!(workspace.files(), before);
        assert_eq!(
            protected.unlock_with_transfer_passphrase(passphrase).unwrap(),
            VaultStatus::Ready { protected: true, transfer: true }
        );
        // Wrapped again for this user; the transfer copy is unchanged.
        assert!(super::super::dpapi::unprotect(&workspace.protected_document().blob).is_ok());
        assert_eq!(workspace.read(TRANSFER_KEY_FILE), copy);
        assert_eq!(protected.get("gmail:old@example.test").unwrap(), "old-refresh-token");
        assert_eq!(protected.get(AI_KEY).unwrap(), "old-ai-key");
        assert_other_data_kept(&workspace);
    }

    #[test]
    fn vault_key_transfer_passphrase_needs_the_key_open_here() {
        let workspace = Workspace::new();
        workspace.earlier_install();
        let protected = workspace.protected();
        protected.set_transfer_passphrase("first passphrase").unwrap();
        workspace.as_if_from_another_user();
        let before = workspace.files();
        assert_eq!(protected.set_transfer_passphrase("second passphrase").unwrap_err(), NEEDS_UNLOCKED);
        assert_eq!(protected.remove_transfer_passphrase().unwrap_err(), NEEDS_UNLOCKED);
        assert_eq!(workspace.files(), before);
        // Once open, both work.
        protected.unlock_with_transfer_passphrase("first passphrase").unwrap();
        protected.set_transfer_passphrase("second passphrase").unwrap();
        workspace.as_if_from_another_user();
        assert_eq!(protected.unlock_with_transfer_passphrase("first passphrase").unwrap_err(), WRONG_PASSPHRASE);
        protected.unlock_with_transfer_passphrase("second passphrase").unwrap();
        assert_eq!(
            protected.remove_transfer_passphrase().unwrap(),
            VaultStatus::Ready { protected: true, transfer: false }
        );
        assert!(!workspace.exists(TRANSFER_KEY_FILE));
        assert_eq!(protected.get(AI_KEY).unwrap(), "old-ai-key");
        // Without it, another user is back to signing in again.
        workspace.as_if_from_another_user();
        assert_eq!(protected.status(), VaultStatus::Locked { transfer: false });
        assert_eq!(protected.unlock_with_transfer_passphrase("second passphrase").unwrap_err(), NO_TRANSFER);
    }

    #[test]
    fn vault_key_transfer_passphrase_is_validated_and_normalized() {
        let workspace = Workspace::new();
        let protected = workspace.protected();
        protected.set(AI_KEY, "secret").unwrap();
        assert_eq!(protected.set_transfer_passphrase("short").unwrap_err(), SHORT_PASSPHRASE);
        assert_eq!(protected.set_transfer_passphrase("   seven   ").unwrap_err(), SHORT_PASSPHRASE);
        // Spaces between the words do not count either.
        assert_eq!(protected.set_transfer_passphrase("a b c d e f g").unwrap_err(), SHORT_PASSPHRASE);
        assert_eq!(protected.set_transfer_passphrase(&"x".repeat(1025)).unwrap_err(), LONG_PASSPHRASE);
        assert!(!workspace.exists(TRANSFER_KEY_FILE));
        // "è" written as one code point here and as e + combining grave on the other keyboard.
        protected.set_transfer_passphrase("caff\u{e8} del mattino").unwrap();
        workspace.as_if_from_another_user();
        protected.unlock_with_transfer_passphrase("caffe\u{300} del mattino").unwrap();
        assert_eq!(protected.get(AI_KEY).unwrap(), "secret");
    }

    #[test]
    fn vault_key_transfer_copy_is_argon2id_with_its_parameters() {
        let workspace = Workspace::new();
        let protected = Vault::with_key_store(&workspace.0, KeyStore::Dpapi);
        protected.set(AI_KEY, "secret").unwrap();
        protected.set_transfer_passphrase("a passphrase for the move").unwrap();
        let copy: serde_json::Value = serde_json::from_slice(&workspace.read(TRANSFER_KEY_FILE)).unwrap();
        assert_eq!(copy["kdf"], "argon2id");
        assert_eq!(copy["argon2_version"], 0x13);
        assert_eq!(copy["memory_kib"], 64 * 1024);
        assert_eq!(copy["iterations"], 3);
        assert_eq!(copy["parallelism"], 4);
        assert_eq!(copy["salt"].as_array().unwrap().len(), 16);
        // A copy naming parameters beyond 256 MiB or 10 passes is damaged, not tried.
        assert_eq!((MAX_MEMORY_KIB, MAX_ITERATIONS), (256 * 1024, 10));
        for (memory_kib, iterations) in [(MAX_MEMORY_KIB + 1, 3), (64 * 1024, MAX_ITERATIONS + 1)] {
            let mut document: TransferKeyDocument = serde_json::from_slice(&workspace.read(TRANSFER_KEY_FILE)).unwrap();
            document.memory_kib = memory_kib;
            document.iterations = iterations;
            assert_eq!(unwrap_transfer(&document, "a passphrase for the move").unwrap_err(), DAMAGED_TRANSFER);
        }
    }

    #[test]
    fn vault_key_start_over_sets_everything_aside_only_when_locked_or_damaged() {
        let workspace = Workspace::new();
        workspace.earlier_install();
        let protected = workspace.protected();
        protected.set_transfer_passphrase("forgotten passphrase").unwrap();
        let open = workspace.files();
        assert_eq!(protected.start_over(false).unwrap_err(), NOTHING_TO_START_OVER);
        assert_eq!(workspace.files(), open);
        workspace.as_if_from_another_user();
        let before = workspace.files();
        let folder = protected.start_over(false).unwrap();
        assert!(folder.starts_with(SET_ASIDE_PREFIX));
        assert_eq!(protected.set_aside_notice(), None, "the person asked for it and saw the folder");
        let after = workspace.files();
        for name in [VAULT_FILE, PROTECTED_KEY_FILE, TRANSFER_KEY_FILE] {
            assert!(!workspace.exists(name));
            assert_eq!(after[&format!("{folder}/{name}")], before[name], "{name}");
        }
        assert_eq!(protected.status(), VaultStatus::Ready { protected: true, transfer: false });
        assert_other_data_kept(&workspace);
    }

    #[test]
    fn vault_key_start_over_refuses_files_it_cannot_read_now() {
        let workspace = Workspace::new();
        let protected = workspace.protected();
        protected.set(AI_KEY, "kept").unwrap();
        let vault = workspace.read(VAULT_FILE);
        // Credentials that cannot be read at this moment (here: a folder in their place).
        fs::rename(workspace.path(VAULT_FILE), workspace.path("held.json")).unwrap();
        fs::create_dir(workspace.path(VAULT_FILE)).unwrap();
        let error = protected.start_over(false).unwrap_err();
        assert!(error.starts_with(STORAGE_FAILED), "{error}");
        assert!(workspace.set_aside_folders().is_empty());
        fs::remove_dir(workspace.path(VAULT_FILE)).unwrap();
        fs::rename(workspace.path("held.json"), workspace.path(VAULT_FILE)).unwrap();
        assert_eq!(workspace.read(VAULT_FILE), vault);
        assert_eq!(protected.get(AI_KEY).unwrap(), "kept");
    }

    #[test]
    fn vault_key_set_aside_interrupted_is_finished_at_the_next_load() {
        let workspace = Workspace::new();
        workspace.earlier_install();
        let protected = workspace.protected();
        protected.get(AI_KEY).unwrap();
        workspace.as_if_from_another_user();
        let before = workspace.files();
        // Crash after the marker and the first move.
        let folder = format!("{SET_ASIDE_PREFIX}20261121-101500");
        fs::write(workspace.path(SET_ASIDE_MARKER), &folder).unwrap();
        fs::create_dir(workspace.path(&folder)).unwrap();
        fs::rename(workspace.path(VAULT_FILE), workspace.path(&format!("{folder}/{VAULT_FILE}"))).unwrap();
        assert_eq!(protected.get(AI_KEY).unwrap(), "");
        assert!(!workspace.exists(SET_ASIDE_MARKER));
        let after = workspace.files();
        for name in [VAULT_FILE, PROTECTED_KEY_FILE] {
            assert_eq!(after[&format!("{folder}/{name}")], before[name], "{name}");
        }
        // A damaged marker stops the vault instead of moving anything anywhere.
        fs::write(workspace.path(SET_ASIDE_MARKER), "../elsewhere").unwrap();
        assert_eq!(protected.get(AI_KEY).unwrap_err(), DAMAGED_SET_ASIDE_MARKER);
        assert_other_data_kept(&workspace);
    }

    #[test]
    fn vault_key_stays_plain_in_the_portable_store() {
        let workspace = Workspace::new();
        let vault = workspace.vault();
        vault.set(AI_KEY, "portable").unwrap();
        assert!(workspace.exists(PLAIN_KEY_FILE));
        assert!(!workspace.exists(PROTECTED_KEY_FILE));
        assert_eq!(vault.get(AI_KEY).unwrap(), "portable");
        assert_eq!(vault.status(), VaultStatus::Ready { protected: false, transfer: false });
        assert_eq!(vault.set_transfer_passphrase("a long passphrase").unwrap_err(), TRANSFER_NEEDS_DPAPI);
        let names: Vec<_> = workspace.files().into_keys().collect();
        assert_eq!(names, [VAULT_FILE, PLAIN_KEY_FILE]);
    }

    #[test]
    fn vault_key_wrapping_that_fails_keeps_the_plain_key_and_tries_again() {
        let workspace = Workspace::new();
        let plain = workspace.earlier_install();
        // The temporary file cannot be written: step 1 fails.
        fs::create_dir(workspace.path(PROTECTED_KEY_TEMPORARY)).unwrap();
        let protected = workspace.protected();
        assert_eq!(protected.get("gmail:old@example.test").unwrap(), "old-refresh-token");
        assert_eq!(workspace.plain_key(), plain);
        assert!(!workspace.exists(PROTECTED_KEY_FILE));
        assert_eq!(protected.status(), VaultStatus::Ready { protected: false, transfer: false });
        protected.set("contacts:1", "saved meanwhile").unwrap();
        fs::remove_dir(workspace.path(PROTECTED_KEY_TEMPORARY)).unwrap();
        assert_eq!(protected.get("contacts:1").unwrap(), "saved meanwhile");
        assert!(!workspace.exists(PLAIN_KEY_FILE));
        assert_eq!(protected.status(), VaultStatus::Ready { protected: true, transfer: false });
    }

    #[test]
    fn vault_key_plain_key_that_is_not_the_wrapped_one_is_never_removed() {
        let workspace = Workspace::new();
        let protected = workspace.protected();
        protected.set(AI_KEY, "protected").unwrap();
        // A plain key of some other vault, restored by hand beside the wrapped key.
        let other = Workspace::new();
        other.vault().set(AI_KEY, "other").unwrap();
        fs::copy(other.path(PLAIN_KEY_FILE), workspace.path(PLAIN_KEY_FILE)).unwrap();
        let before = workspace.files();
        assert_eq!(protected.get(AI_KEY).unwrap(), "protected");
        assert_eq!(workspace.files(), before);
    }

    /// A wrapped file that opens nothing here, beside the plain key that opens the credentials:
    /// it is kept byte for byte under another name, and the plain key is wrapped in its place.
    fn assert_stray_moved_aside_and_plain_key_wrapped(workspace: &Workspace, stray: &[u8], plain: [u8; 32]) {
        let protected = workspace.protected();
        assert_eq!(protected.get(AI_KEY).unwrap(), "old-ai-key");
        assert_eq!(protected.status(), VaultStatus::Ready { protected: true, transfer: false });
        let unused: Vec<_> = workspace
            .files()
            .into_iter()
            .filter(|(name, _)| name.starts_with(UNUSED_PROTECTED_KEY_PREFIX))
            .collect();
        assert_eq!(unused.len(), 1, "{unused:?}");
        assert_eq!(unused[0].1, stray, "the stray file is kept as it was");
        let document = workspace.protected_document();
        assert_eq!(*Zeroizing::new(super::super::dpapi::unprotect(&document.blob).unwrap()), plain);
        assert!(!workspace.exists(PLAIN_KEY_FILE));
        assert_other_data_kept(workspace);
    }

    #[test]
    fn vault_key_wrapped_for_another_vault_is_moved_aside_and_the_plain_key_wrapped() {
        let workspace = Workspace::new();
        let plain = workspace.earlier_install();
        // A wrapped key of another vault beside the plain key that opens these credentials.
        let other = Workspace::new();
        other.protected().set(AI_KEY, "other").unwrap();
        fs::copy(other.path(PROTECTED_KEY_FILE), workspace.path(PROTECTED_KEY_FILE)).unwrap();
        let stray = workspace.read(PROTECTED_KEY_FILE);
        assert_stray_moved_aside_and_plain_key_wrapped(&workspace, &stray, plain);
    }

    #[test]
    fn vault_key_of_another_user_for_another_vault_is_moved_aside_never_replaced() {
        let workspace = Workspace::new();
        let plain = workspace.earlier_install();
        // Another user's wrapped key of some other vault, beside the plain key that opens these.
        let other = Workspace::new();
        other.protected().set(AI_KEY, "other").unwrap();
        other.as_if_from_another_user();
        fs::copy(other.path(PROTECTED_KEY_FILE), workspace.path(PROTECTED_KEY_FILE)).unwrap();
        let stray = workspace.read(PROTECTED_KEY_FILE);
        assert_stray_moved_aside_and_plain_key_wrapped(&workspace, &stray, plain);
    }

    #[test]
    fn vault_key_damaged_wrapped_key_beside_a_plain_key_that_opens_is_moved_aside() {
        let workspace = Workspace::new();
        let plain = workspace.earlier_install();
        fs::write(workspace.path(PROTECTED_KEY_FILE), b"{damaged").unwrap();
        assert_stray_moved_aside_and_plain_key_wrapped(&workspace, b"{damaged", plain);
    }

    #[test]
    fn vault_key_blob_that_windows_cannot_open_now_is_retried_never_set_aside() {
        let workspace = Workspace::new();
        workspace.earlier_install();
        let protected = workspace.protected();
        protected.get(AI_KEY).unwrap();
        // Not another user's master key: a flipped bit of the data (or a profile not loaded).
        let mut document = workspace.protected_document();
        let last = document.blob.len() - 1;
        document.blob[last] ^= 0x01;
        fs::write(workspace.path(PROTECTED_KEY_FILE), serde_json::to_vec(&document).unwrap()).unwrap();
        let before = workspace.files();
        let error = protected.get(AI_KEY).unwrap_err();
        assert!(error.starts_with(UNAVAILABLE_PREFIX) && error.contains("(error 0x8007000D)"), "{error}");
        assert_eq!(error_code(&error), vault_codes::UNAVAILABLE);
        assert!(matches!(protected.status(), VaultStatus::Damaged { code: vault_codes::UNAVAILABLE, .. }));
        // A sign-in and a start-over are both refused: good credentials may be behind it.
        assert_eq!(error_code(&protected.set("gmail:new@example.test", "fresh").unwrap_err()), vault_codes::UNAVAILABLE);
        assert!(protected.check_writable().is_err());
        assert_eq!(error_code(&protected.start_over(false).unwrap_err()), vault_codes::UNAVAILABLE);
        assert_eq!(workspace.files(), before);
        assert!(workspace.set_aside_folders().is_empty());
    }

    #[test]
    fn vault_key_blob_that_stays_unopenable_is_set_aside_when_the_person_confirms() {
        let workspace = Workspace::new();
        workspace.earlier_install();
        let protected = workspace.protected();
        protected.get(AI_KEY).unwrap();
        let mut document = workspace.protected_document();
        let last = document.blob.len() - 1;
        document.blob[last] ^= 0x01;
        fs::write(workspace.path(PROTECTED_KEY_FILE), serde_json::to_vec(&document).unwrap()).unwrap();
        let before = workspace.files();
        // «Try again» failed, and the person confirmed signing in to the accounts again.
        let folder = protected.start_over(true).unwrap();
        let after = workspace.files();
        for name in [VAULT_FILE, PROTECTED_KEY_FILE] {
            assert_eq!(after[&format!("{folder}/{name}")], before[name], "{name}: moved, never deleted");
        }
        assert_eq!(protected.status(), VaultStatus::Ready { protected: true, transfer: false });
        protected.set("gmail:new@example.test", "fresh").unwrap();
        assert_eq!(protected.get("gmail:new@example.test").unwrap(), "fresh");
        assert_other_data_kept(&workspace);
    }

    #[test]
    fn vault_key_nothing_is_set_aside_while_windows_cannot_protect_a_new_key() {
        let workspace = Workspace::new();
        workspace.earlier_install();
        workspace.protected().get(AI_KEY).unwrap();
        workspace.as_if_from_another_user();
        let mut broken = workspace.protected();
        broken.broken_dpapi = Some(0x8009_0345);
        let before = workspace.files();
        // A sign-in would set the old credentials aside and then fail to write the new key.
        let error = broken.set("gmail:new@example.test", "fresh").unwrap_err();
        assert_eq!(error, unavailable_message(0x8009_0345));
        assert_eq!(error_code(&error), vault_codes::UNAVAILABLE);
        assert_eq!(error_code(&broken.start_over(true).unwrap_err()), vault_codes::UNAVAILABLE);
        assert_eq!(workspace.files(), before);
        assert!(workspace.set_aside_folders().is_empty());
        // Once Windows protects again, the sign-in goes on.
        workspace.protected().set("gmail:new@example.test", "fresh").unwrap();
        assert_eq!(workspace.set_aside_folders().len(), 1);
    }

    #[test]
    fn vault_key_damaged_transfer_copy_is_reported_never_taken_for_no_passphrase() {
        let workspace = Workspace::new();
        workspace.earlier_install();
        let protected = workspace.protected();
        protected.set_transfer_passphrase("a passphrase for the move").unwrap();
        workspace.as_if_from_another_user();
        fs::write(workspace.path(TRANSFER_KEY_FILE), b"{damaged").unwrap();
        let before = workspace.files();
        assert_eq!(protected.get(AI_KEY).unwrap_err(), DAMAGED_TRANSFER);
        assert!(matches!(protected.status(), VaultStatus::Damaged { code: vault_codes::DAMAGED, .. }));
        assert!(protected.set("gmail:new@example.test", "fresh").is_err());
        assert!(protected.check_writable().is_err());
        assert_eq!(workspace.files(), before);
        assert!(workspace.set_aside_folders().is_empty());
    }

    #[test]
    fn vault_key_check_before_a_sign_in() {
        let workspace = Workspace::new();
        workspace.earlier_install();
        let protected = workspace.protected();
        protected.check_writable().unwrap();
        // Another user's, no passphrase: the sign-in may go on, its save sets the old ones aside.
        workspace.as_if_from_another_user();
        let before = workspace.files();
        protected.check_writable().unwrap();
        assert_eq!(workspace.files(), before, "the check itself changes nothing");
        // With a passphrase to enter first, no sign-in is started.
        let workspace = Workspace::new();
        workspace.earlier_install();
        let protected = workspace.protected();
        protected.set_transfer_passphrase("a passphrase for the move").unwrap();
        workspace.as_if_from_another_user();
        let error = protected.check_writable().unwrap_err();
        assert_eq!((error.as_str(), error_code(&error)), (LOCKED_WITH_TRANSFER, vault_codes::LOCKED));
    }

    #[test]
    fn vault_key_every_message_has_its_code() {
        for (message, code) in [
            (FOREIGN_KEY, vault_codes::FOREIGN),
            (LOCKED_WITH_TRANSFER, vault_codes::LOCKED),
            (WRONG_PASSPHRASE, vault_codes::WRONG_PASSPHRASE),
            (NO_TRANSFER, vault_codes::NO_TRANSFER),
            (NEEDS_UNLOCKED, vault_codes::NEEDS_UNLOCK),
            (SHORT_PASSPHRASE, vault_codes::PASSPHRASE),
            (LONG_PASSPHRASE, vault_codes::PASSPHRASE),
            (NOTHING_TO_START_OVER, vault_codes::NOTHING_TO_START_OVER),
            (TRANSFER_NEEDS_DPAPI, vault_codes::NEEDS_DPAPI),
            (NO_MEMORY_FOR_TRANSFER, vault_codes::UNAVAILABLE),
            (DAMAGED_TRANSFER, vault_codes::DAMAGED),
            (MISSING_PROTECTED_KEY, vault_codes::DAMAGED),
            (VAULT_NOT_AUTHENTIC, vault_codes::DAMAGED),
        ] {
            assert_eq!(error_code(message), code, "{message}");
        }
        assert_eq!(error_code(&unavailable_message(0x8007_000D)), vault_codes::UNAVAILABLE);
        assert_eq!(error_code(&storage_error(std::io::Error::other("held"))), vault_codes::STORAGE);
    }

    #[test]
    fn vault_key_protected_store_keeps_the_existing_vault_guarantees() {
        // The tests above with the plain key, with the key wrapped by Windows.
        let workspace = Workspace::new();
        let protected = workspace.protected();
        protected.set(AI_KEY, "original").unwrap();
        let key = workspace.read(PROTECTED_KEY_FILE);
        let mut document: serde_json::Value = serde_json::from_slice(&workspace.read(VAULT_FILE)).unwrap();
        let first = document["ciphertext"][0].as_u64().unwrap();
        document["ciphertext"][0] = serde_json::json!(first ^ 1);
        let damaged = serde_json::to_vec(&document).unwrap();
        fs::write(workspace.path(VAULT_FILE), &damaged).unwrap();
        assert!(protected.get(AI_KEY).is_err());
        assert!(protected.set("other", "replacement").is_err());
        assert!(protected.start_over(false).is_ok(), "a damaged vault can be set aside on request");
        assert_eq!(protected.get(AI_KEY).unwrap(), "");
        let folder = &workspace.set_aside_folders()[0];
        assert_eq!(workspace.read(&format!("{folder}/{VAULT_FILE}")), damaged);
        assert_eq!(workspace.read(&format!("{folder}/{PROTECTED_KEY_FILE}")), key);
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
                        Vault::with_key_store(&root, KeyStore::Portable)
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
