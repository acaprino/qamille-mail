//! The registry key of one test that writes the registry (`background`, `mail_handler`,
//! `notifications`): `HKCU\Software\Qamille-tests-<pid>-<module>-<test>`, straight under `Software`,
//! removed with everything in it when the test ends. The tests run in parallel, so no two share a
//! parent: `RegSetKeyValueW` creates a path one key at a time, and another test that deleted a shared
//! parent in between would make the write fail with `ERROR_KEY_DELETED`. The tests never touch
//! Windows' own keys, nor those of the owner's copies.

use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RegDeleteKeyW, RegDeleteTreeW};
use windows::core::HSTRING;

pub struct ScratchKey(String);

impl ScratchKey {
    /// `module` and `test` name the key, so that no two tests of one run share it.
    pub fn new(module: &str, test: &str) -> Self {
        Self(format!(r"Software\Qamille-tests-{}-{module}-{test}", std::process::id()))
    }

    /// The key, under `HKEY_CURRENT_USER`.
    pub fn path(&self) -> &str {
        &self.0
    }

    /// A key under it.
    pub fn under(&self, rest: &str) -> String {
        format!(r"{}\{rest}", self.0)
    }
}

impl Drop for ScratchKey {
    fn drop(&mut self) {
        let key = HSTRING::from(self.0.as_str());
        // SAFETY: the string lives across both calls. `RegDeleteTreeW` empties the key, `RegDeleteKeyW`
        // removes the key itself; only this test's own key, never a parent.
        unsafe {
            let _ = RegDeleteTreeW(HKEY_CURRENT_USER, &key);
            let _ = RegDeleteKeyW(HKEY_CURRENT_USER, &key);
        }
    }
}
