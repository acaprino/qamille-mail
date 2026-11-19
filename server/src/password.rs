//! Password: regole di lunghezza, hash argon2id, verifica a lavoro costante,
//! password provvisorie.

use std::sync::OnceLock;

use argon2::password_hash::rand_core::{OsRng, RngCore};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};

pub const MIN_CHARS: usize = 10;
pub const MAX_CHARS: usize = 256;
/// Alfabeto delle password provvisorie: niente `i`, `l`, `o`, `0`, `1`.
pub const TEMPORARY_ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
pub const TEMPORARY_LEN: usize = 20;

/// Parametri argon2id: 19 MiB, 2 passate, 1 filo.
pub const ARGON2_M_KIB: u32 = 19_456;
pub const ARGON2_T: u32 = 2;
pub const ARGON2_P: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PasswordPolicyError {
    #[error("La password deve avere almeno 10 caratteri.")]
    TooShort,
    #[error("La password può avere al più 256 caratteri.")]
    TooLong,
    #[error("Questa password è tra quelle più usate: scegline un'altra.")]
    TooCommon,
}

/// Password lunghe abbastanza da passare il controllo di lunghezza, ma tra le prime di ogni
/// elenco di password rubate: si confrontano in minuscolo, senza spazi ai lati.
const COMMON: &[&str] = &[
    "0000000000", "00000000000", "000000000000", "0123456789", "01234567890", "012345678901",
    "1111111111", "11111111111", "111111111111", "1234512345", "1234567890", "12345678901",
    "123456789012", "1234567890123", "12345678910", "123456789a", "1q2w3e4r5t", "1q2w3e4r5t6y",
    "1qaz2wsx3edc", "1qazxsw23edc", "a1b2c3d4e5", "abc1234567", "abc123456789",
    "abcd123456", "abcdefghij", "abcdefghijk", "administrator", "adminadmin1", "adminadmin12",
    "asdasdasdasd", "asdf123456", "asdfghjkl1", "asdfghjkl123", "asdfghjklq", "asdfghjklzxcvbnm",
    "baseball123", "changeme123", "changeme12", "changeme1234", "ciao123456", "ciaociaociao",
    "ciaociao12", "dragon1234", "football123", "forzaitalia", "iloveyou12", "iloveyou123",
    "iloveyou1234", "italia1234", "juventus123", "letmein1234", "letmein123", "master1234",
    "monkey1234", "mypassword1", "mypassword12", "mypassword123", "p@ssw0rd123", "p@ssword123",
    "p@ssw0rd1234", "pass123456", "pass1234567", "passw0rd123", "passw0rd!!", "passw0rd1234",
    "password00", "password01", "password1!", "password10", "password11", "password12",
    "password123", "password1234", "password12345", "password123456", "password!1", "password!!",
    "princess123", "qazwsxedcrfv", "qamille123", "qamille1234", "qwerty1234", "qwerty12345",
    "qwerty123456", "qwertyuiop", "qwertyuiop1", "qwertyuiop12", "qwertyuiopasdfghjkl",
    "qwertyuiop123", "superman123", "sunshine123", "trustno1234", "welcome123", "welcome1234",
    "welcome12345", "zaq12wsxcde3", "zxcvbnm123", "zxcvbnmasdf",
];

fn is_common(password: &str) -> bool {
    COMMON.contains(&password.trim().to_lowercase().as_str())
}

/// Controlla la lunghezza in caratteri (non in byte).
pub fn validate(password: &str) -> Result<(), PasswordPolicyError> {
    let chars = password.chars().count();
    if chars < MIN_CHARS {
        Err(PasswordPolicyError::TooShort)
    } else if chars > MAX_CHARS {
        Err(PasswordPolicyError::TooLong)
    } else if is_common(password) {
        Err(PasswordPolicyError::TooCommon)
    } else {
        Ok(())
    }
}

fn argon2() -> Argon2<'static> {
    let params = Params::new(ARGON2_M_KIB, ARGON2_T, ARGON2_P, None)
        .expect("parametri argon2 costanti e validi");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

/// Hash argon2id in formato PHC, con sale casuale.
pub fn hash(password: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    argon2()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| format!("argon2: {e}"))
}

fn dummy_hash() -> &'static str {
    static DUMMY: OnceLock<String> = OnceLock::new();
    DUMMY.get_or_init(|| {
        let mut seed = [0u8; 32];
        OsRng.fill_bytes(&mut seed);
        hash(&hex::encode(seed)).expect("hash fittizio")
    })
}

/// Verifica `password` contro `stored`. Senza hash (o con un hash illeggibile)
/// fa comunque una verifica vera contro un hash fittizio e restituisce `false`:
/// il lavoro è lo stesso in ogni caso.
pub fn verify(stored: Option<&str>, password: &str) -> bool {
    let parsed = stored.and_then(|s| PasswordHash::new(s).ok());
    match parsed {
        Some(parsed) => argon2()
            .verify_password(password.as_bytes(), &parsed)
            .is_ok(),
        None => {
            let dummy = PasswordHash::new(dummy_hash()).expect("hash fittizio leggibile");
            let _ = argon2().verify_password(password.as_bytes(), &dummy);
            false
        }
    }
}

/// Password provvisoria di 20 caratteri dall'alfabeto senza caratteri ambigui.
pub fn temporary() -> String {
    random_from_alphabet(TEMPORARY_LEN)
}

/// `len` simboli casuali da [`TEMPORARY_ALPHABET`] (anche per i codici per posta).
pub fn random_from_alphabet(len: usize) -> String {
    // Campionamento con rifiuto: nessuna distorsione verso i primi simboli.
    let n = TEMPORARY_ALPHABET.len() as u8;
    let limit = u8::MAX - (u8::MAX % n);
    let mut out = String::with_capacity(len);
    let mut buf = [0u8; 64];
    while out.len() < len {
        OsRng.fill_bytes(&mut buf);
        for &b in &buf {
            if b < limit {
                out.push(TEMPORARY_ALPHABET[(b % n) as usize] as char);
                if out.len() == len {
                    break;
                }
            }
        }
    }
    out
}

/// 32 byte casuali, per le chiavi dei dispositivi.
pub fn random_bytes_32() -> [u8; 32] {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes
}

/// 16 byte casuali, per i sali dei codici.
pub fn random_bytes_16() -> [u8; 16] {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    bytes
}
