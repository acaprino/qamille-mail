//! Windows DPAPI for the vault key (`secret.rs`): `CryptProtectData` for the current Windows
//! user, with a fixed entropy of the app and no prompt. Only this user on this computer can
//! unprotect what `protect` returns; another user, another computer or damaged bytes fail.
use windows_sys::Win32::{
    Foundation::{GetLastError, LocalFree},
    Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    },
};

/// Mixed into every blob: another program of the same user cannot unprotect the key by
/// calling DPAPI without it.
const ENTROPY: &[u8] = b"Qamille credentials key (DPAPI) v1";

/// `NTE_BAD_KEY_STATE`, «Key not valid for use in specified state»: the master key the blob
/// names is not one of this user's. That is what a blob written by another Windows user, or by
/// this user on another computer, gives. It is the only code taken to mean «another user or
/// computer»; damaged data gives `ERROR_INVALID_DATA` and the like. A master key of this very
/// user that is missing (a roaming profile not loaded) gives the same code: phase 3, Task 9,
/// scenario K checks the real case on a second Windows user.
pub const NTE_BAD_KEY_STATE: u32 = 0x8009_000B;

/// A DPAPI call that failed, with the Windows error code (never the data).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DpapiError {
    pub code: u32,
    protect: bool,
}

impl DpapiError {
    /// The blob belongs to another Windows user or computer (`NTE_BAD_KEY_STATE`).
    pub fn foreign(&self) -> bool {
        !self.protect && self.code == NTE_BAD_KEY_STATE
    }
}

impl std::fmt::Display for DpapiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let action = if self.protect { "protect" } else { "open" };
        write!(f, "Windows could not {action} the credentials key (error 0x{:08X}).", self.code)
    }
}

/// Wraps `bytes` for the current Windows user.
pub fn protect(bytes: &[u8]) -> Result<Vec<u8>, DpapiError> {
    run(bytes, Direction::Protect)
}

/// Unwraps a blob written by `protect` for this user on this computer.
pub fn unprotect(bytes: &[u8]) -> Result<Vec<u8>, DpapiError> {
    run(bytes, Direction::Unprotect)
}

#[derive(Clone, Copy)]
enum Direction {
    Protect,
    Unprotect,
}

fn run(bytes: &[u8], direction: Direction) -> Result<Vec<u8>, DpapiError> {
    let protect = matches!(direction, Direction::Protect);
    // ERROR_INVALID_PARAMETER: a blob of more than 4 GiB is no blob of ours.
    let length = u32::try_from(bytes.len()).map_err(|_| DpapiError { code: 0x8007_0057, protect })?;
    // DPAPI only reads its inputs; the pointers are `*mut` in the binding.
    let input = CRYPT_INTEGER_BLOB { cbData: length, pbData: bytes.as_ptr().cast_mut() };
    let entropy = CRYPT_INTEGER_BLOB { cbData: ENTROPY.len() as u32, pbData: ENTROPY.as_ptr().cast_mut() };
    let mut output = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
    let done = unsafe {
        match direction {
            Direction::Protect => CryptProtectData(
                &input,
                std::ptr::null(),
                &entropy,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            ),
            Direction::Unprotect => CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                &entropy,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            ),
        }
    };
    if done == 0 {
        // GetLastError gives a Win32 code (13) or an HRESULT (0x8009000B): Win32 codes are
        // shown as HRESULTs, so one form names each failure.
        let code = unsafe { GetLastError() };
        let code = if code != 0 && code <= 0xFFFF { 0x8007_0000 | code } else { code };
        return Err(DpapiError { code, protect });
    }
    // The output is allocated by DPAPI with LocalAlloc: copy it, wipe it, free it.
    let result = if output.pbData.is_null() {
        Vec::new()
    } else {
        let slice = unsafe { std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize) };
        let copy = slice.to_vec();
        zeroize::Zeroize::zeroize(slice);
        unsafe { LocalFree(output.pbData.cast()) };
        copy
    };
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dpapi_round_trips_for_this_user_and_hides_the_bytes() {
        let secret = [0x5Au8; 32];
        let blob = protect(&secret).unwrap();
        assert!(!blob.windows(secret.len()).any(|window| window == secret));
        assert_eq!(unprotect(&blob).unwrap(), secret);
        // Each call uses a fresh salt: the same bytes give another blob.
        assert_ne!(protect(&secret).unwrap(), blob);
    }

    #[test]
    fn dpapi_tells_a_key_of_another_user_from_a_damaged_blob() {
        let blob = protect(&[7u8; 32]).unwrap();
        // Bytes 24..40 name the master key: another user's or computer's blob names one this user
        // does not have.
        let mut foreign = blob.clone();
        foreign[24] ^= 0xFF;
        let error = unprotect(&foreign).unwrap_err();
        assert_eq!(error.code, NTE_BAD_KEY_STATE, "{error}");
        assert!(error.foreign());
        // A flipped bit in the data or its signature is damage, not another user.
        let mut damaged = blob.clone();
        let last = damaged.len() - 1;
        damaged[last] ^= 0x01;
        let error = unprotect(&damaged).unwrap_err();
        assert!(!error.foreign(), "{error}");
        assert!(error.to_string().contains(&format!("0x{:08X}", error.code)));
    }

    #[test]
    fn dpapi_refuses_a_damaged_blob() {
        let mut blob = protect(b"vault key").unwrap();
        let last = blob.len() - 1;
        blob[last] ^= 0x01;
        let error = unprotect(&blob).unwrap_err();
        assert!(error.to_string().contains("error 0x"), "{error}");
        assert!(unprotect(b"").is_err());
    }
}
