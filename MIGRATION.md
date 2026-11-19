# Tauri migration

1. [x] Separate the Rust domain, application services and infrastructure.
2. [x] Add the Tauri desktop adapter and Svelte interface.
3. [x] Connect inbox, drafts, settings and the existing Rust capabilities.
4. [x] Test service behavior, verify both interfaces and inspect the new UI.

The existing native interface stays available during migration. User data keeps its
existing format and location. Development and UI checks use isolated sample data.

## Verified first increment

- 70 existing Rust unit tests passed; 3 existing opt-in tests remain ignored.
- 15 application/repository tests and 2 architecture-dependency tests passed.
- 6 browser interaction checks and 8 checks in the standalone Tauri app passed.
- Frontend type/accessibility checks: 0 errors, 0 warnings.
- Native checks include sample sending and immediate window-close draft saving.
- `Preview-Qamille.cmd` opens a separate sample workspace.
- `Start-Qamille.cmd` opens the existing user workspace.

This is the first working migration increment, not full feature parity. Remaining
native-only features and compatibility limits are listed in `desktop/README.md`.
Release-mode performance comparisons and installer packaging are not completed.
