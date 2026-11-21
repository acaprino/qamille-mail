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

/// Wraps `bytes` for the current Windows user.
pub fn protect(bytes: &[u8]) -> Result<Vec<u8>, String> {
    run(bytes, Direction::Protect)
}

/// Unwraps a blob written by `protect` for this user on this computer. The error carries the
/// Windows error code, never the data.
pub fn unprotect(bytes: &[u8]) -> Result<Vec<u8>, String> {
    run(bytes, Direction::Unprotect)
}

#[derive(Clone, Copy)]
enum Direction {
    Protect,
    Unprotect,
}

fn run(bytes: &[u8], direction: Direction) -> Result<Vec<u8>, String> {
    let length = u32::try_from(bytes.len()).map_err(|_| "DPAPI input is too large.".to_string())?;
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
        let code = unsafe { GetLastError() };
        let action = match direction {
            Direction::Protect => "protect",
            Direction::Unprotect => "unprotect",
        };
        return Err(format!("Windows could not {action} the credentials key (error 0x{code:08X})."));
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
    fn dpapi_refuses_a_damaged_blob() {
        let mut blob = protect(b"vault key").unwrap();
        let last = blob.len() - 1;
        blob[last] ^= 0x01;
        let error = unprotect(&blob).unwrap_err();
        assert!(error.contains("error 0x"), "{error}");
        assert!(unprotect(b"").is_err());
    }
}
