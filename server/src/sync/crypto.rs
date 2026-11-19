//! Cifratura a riposo degli elementi di sincronizzazione: ChaCha20-Poly1305 con la chiave del
//! server (`QAMILLE_SYNC_KEY`), un nonce casuale a ogni scrittura e come dati associati
//! `user_id ‖ 0x00 ‖ item_id`: un testo cifrato spostato su un altro utente o un altro id non
//! si apre. Nessun messaggio d'errore contiene la chiave, il testo o i dati associati.

use std::fmt;

use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng, Payload, rand_core::RngCore};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use zeroize::Zeroize;

/// Cifre esadecimali di una chiave (32 byte).
pub const KEY_HEX_LEN: usize = 64;
/// Lunghezza del nonce di ChaCha20-Poly1305.
pub const NONCE_LEN: usize = 12;
/// Cifre diverse minime: una chiave di tutti zeri o ripetuta non è una chiave scelta a caso.
const MIN_DISTINCT_DIGITS: usize = 8;

/// Perché un testo non è una chiave valida. Mai il testo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyError {
    Format,
    Weak,
}

impl KeyError {
    pub fn reason(self) -> &'static str {
        match self {
            KeyError::Format => {
                "servono 64 cifre esadecimali (per esempio da `qamille-server sync keygen`)"
            }
            KeyError::Weak => "la chiave è troppo regolare: usa `qamille-server sync keygen`",
        }
    }
}

/// La chiave del server. Il `Debug` non la stampa; i byte si azzerano alla fine.
#[derive(Clone)]
pub struct SyncKey([u8; 32]);

impl Drop for SyncKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl fmt::Debug for SyncKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SyncKey(<nascosta>)")
    }
}

/// Il risultato di una cifratura: nonce e testo cifrato (con il tag in coda).
pub struct Sealed {
    pub nonce: [u8; NONCE_LEN],
    pub ciphertext: Vec<u8>,
}

/// Una cifratura che non si apre. Nessun dettaglio: né il motivo né i byte.
#[derive(Debug, PartialEq, Eq)]
pub struct OpenError;

fn associated_data(user_id: &str, item_id: &str) -> Vec<u8> {
    let mut aad = Vec::with_capacity(user_id.len() + 1 + item_id.len());
    aad.extend_from_slice(user_id.as_bytes());
    aad.push(0);
    aad.extend_from_slice(item_id.as_bytes());
    aad
}

impl SyncKey {
    /// 64 cifre esadecimali (maiuscole o minuscole), spazi ai lati ignorati, almeno 8 cifre
    /// diverse.
    pub fn parse(value: &str) -> Result<SyncKey, KeyError> {
        let value = value.trim();
        if value.len() != KEY_HEX_LEN || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(KeyError::Format);
        }
        let mut distinct = [false; 16];
        for b in value.bytes() {
            distinct[(b as char).to_digit(16).unwrap_or(0) as usize] = true;
        }
        if distinct.iter().filter(|seen| **seen).count() < MIN_DISTINCT_DIGITS {
            return Err(KeyError::Weak);
        }
        let mut bytes = [0u8; 32];
        hex::decode_to_slice(value, &mut bytes).map_err(|_| KeyError::Format)?;
        Ok(SyncKey(bytes))
    }

    /// Una chiave casuale (mai debole).
    pub fn generate() -> SyncKey {
        loop {
            let mut bytes = [0u8; 32];
            OsRng.fill_bytes(&mut bytes);
            let mut hex = hex::encode(bytes);
            bytes.zeroize();
            let parsed = SyncKey::parse(&hex);
            hex.zeroize();
            if let Ok(key) = parsed {
                return key;
            }
        }
    }

    /// Stessa chiave, con un confronto a tempo costante.
    pub fn same_as(&self, other: &SyncKey) -> bool {
        use subtle::ConstantTimeEq;
        self.0.ct_eq(&other.0).into()
    }

    /// Le 64 cifre, solo per `sync keygen`.
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    fn cipher(&self) -> ChaCha20Poly1305 {
        ChaCha20Poly1305::new(Key::from_slice(&self.0))
    }

    /// Cifra `plaintext` per `user_id` e `item_id`, con un nonce nuovo.
    pub fn seal(&self, user_id: &str, item_id: &str, plaintext: &[u8]) -> Sealed {
        let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
        let aad = associated_data(user_id, item_id);
        let ciphertext = self
            .cipher()
            .encrypt(&nonce, Payload { msg: plaintext, aad: &aad })
            // ChaCha20-Poly1305 non fallisce nel cifrare un testo che sta in memoria.
            .expect("cifratura in memoria");
        let mut bytes = [0u8; NONCE_LEN];
        bytes.copy_from_slice(&nonce);
        Sealed { nonce: bytes, ciphertext }
    }

    /// Apre un testo cifrato per `user_id` e `item_id`.
    pub fn open(
        &self,
        user_id: &str,
        item_id: &str,
        nonce: &[u8],
        ciphertext: &[u8],
    ) -> Result<Vec<u8>, OpenError> {
        if nonce.len() != NONCE_LEN {
            return Err(OpenError);
        }
        let aad = associated_data(user_id, item_id);
        self.cipher()
            .decrypt(Nonce::from_slice(nonce), Payload { msg: ciphertext, aad: &aad })
            .map_err(|_| OpenError)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn key() -> SyncKey {
        SyncKey::parse(GOOD).unwrap()
    }

    #[test]
    fn a_key_is_64_hex_digits_in_either_case() {
        assert!(SyncKey::parse(GOOD).is_ok());
        let upper = SyncKey::parse(&GOOD.to_uppercase()).unwrap();
        let sealed = key().seal("u", "i", b"{}");
        assert_eq!(upper.open("u", "i", &sealed.nonce, &sealed.ciphertext).unwrap(), b"{}");
        assert!(SyncKey::parse(&format!("  {GOOD}\n")).is_ok());
        for bad in [&GOOD[..63], &format!("{GOOD}0"), &"zz".repeat(32), ""] {
            assert_eq!(SyncKey::parse(bad).unwrap_err(), KeyError::Format, "{bad:?}");
        }
    }

    #[test]
    fn a_regular_key_is_weak() {
        for weak in ["0".repeat(64), "ab".repeat(32), "0123".repeat(16)] {
            assert_eq!(SyncKey::parse(&weak).unwrap_err(), KeyError::Weak, "{weak}");
        }
    }

    #[test]
    fn generate_gives_a_valid_key_and_two_different_ones() {
        let (a, b) = (SyncKey::generate(), SyncKey::generate());
        assert!(SyncKey::parse(&a.to_hex()).is_ok());
        assert_ne!(a.to_hex(), b.to_hex());
    }

    #[test]
    fn the_debug_of_a_key_is_hidden() {
        let k = key();
        assert_eq!(format!("{k:?}"), "SyncKey(<nascosta>)");
        assert_eq!(format!("{k:#?}"), "SyncKey(<nascosta>)");
        assert!(!format!("{k:?}").contains(&GOOD[..16]));
    }

    #[test]
    fn seal_then_open_gives_the_plaintext() {
        let k = key();
        for text in [b"{\"a\":1}".to_vec(), vec![b'x'; 64 * 1024], Vec::new()] {
            let sealed = k.seal("user", "item", &text);
            assert_eq!(sealed.ciphertext.len(), text.len() + 16);
            assert_eq!(k.open("user", "item", &sealed.nonce, &sealed.ciphertext).unwrap(), text);
        }
    }

    #[test]
    fn two_seals_of_the_same_text_have_different_nonces_and_ciphertexts() {
        let k = key();
        let (a, b) = (k.seal("u", "i", b"{}"), k.seal("u", "i", b"{}"));
        assert_ne!(a.nonce, b.nonce);
        assert_ne!(a.ciphertext, b.ciphertext);
    }

    #[test]
    fn an_item_moved_to_another_user_or_id_does_not_open() {
        let k = key();
        let sealed = k.seal("u1", "i1", b"segreto");
        assert!(k.open("u1", "i1", &sealed.nonce, &sealed.ciphertext).is_ok());
        assert_eq!(k.open("u2", "i1", &sealed.nonce, &sealed.ciphertext), Err(OpenError));
        assert_eq!(k.open("u1", "i2", &sealed.nonce, &sealed.ciphertext), Err(OpenError));
        // Il separatore: «ab» + «c» non è «a» + «bc».
        let sealed = k.seal("ab", "c", b"x");
        assert!(k.open("a", "bc", &sealed.nonce, &sealed.ciphertext).is_err());
    }

    #[test]
    fn a_wrong_key_a_changed_byte_or_a_short_nonce_do_not_open() {
        let k = key();
        let sealed = k.seal("u", "i", b"segreto");
        let other = SyncKey::parse(&"fedcba9876543210".repeat(4)).unwrap();
        assert!(other.open("u", "i", &sealed.nonce, &sealed.ciphertext).is_err());
        for at in [0, sealed.ciphertext.len() - 1] {
            let mut bad = sealed.ciphertext.clone();
            bad[at] ^= 1;
            assert!(k.open("u", "i", &sealed.nonce, &bad).is_err());
        }
        let mut nonce = sealed.nonce;
        nonce[0] ^= 1;
        assert!(k.open("u", "i", &nonce, &sealed.ciphertext).is_err());
        assert!(k.open("u", "i", &sealed.nonce[..11], &sealed.ciphertext).is_err());
        assert!(k.open("u", "i", &sealed.nonce, &sealed.ciphertext[..5]).is_err());
    }

    #[test]
    fn the_open_error_says_nothing() {
        assert_eq!(format!("{:?}", OpenError), "OpenError");
    }
}

#[cfg(test)]
mod same_as_tests {
    use super::*;

    #[test]
    fn same_as_compares_the_bytes() {
        let a = SyncKey::parse(&"0123456789abcdef".repeat(4)).unwrap();
        let b = SyncKey::parse(&"0123456789ABCDEF".repeat(4)).unwrap();
        let c = SyncKey::parse(&"fedcba9876543210".repeat(4)).unwrap();
        assert!(a.same_as(&b));
        assert!(!a.same_as(&c));
    }
}
