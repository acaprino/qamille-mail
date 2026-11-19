# Qamille desktop

Tauri 2 + React 19 interface (the «Compatto» look, with unlumen UI components on Tailwind CSS v4, Motion and Ark UI) backed by the shared Rust engine in `../app`.

Meeting recorder and transcription work uses [Meetily as an upstream reference](../reference/README.md). The reference register tracks the last reviewed revision, relevant modules and the update review procedure.

The meeting workflow is documented in the [functional and technical specification](../docs/meetings-specifica.md) and [implementation report](../docs/meetings-implementazione.md). The recorder, deferred local/remote transcription, versioned notes and summaries, export, search and MCP adapters are implemented. Real-device capture, Italian accuracy and two-hour acceptance tests remain open.

Recording uses independently verified WAV chunks of at most one second. Damaged
indexes and partial recordings can be recovered without discarding healthy audio;
the UI keeps incomplete-audio warnings visible. See the [crash recovery report](../docs/meetings-recupero-crash.md) for process-kill tests and the limits of this protection.

Finished AI results have separate recovery checkpoints: reopening a meeting or
restarting the app can confirm a previously finished result without calling the
provider again. Run `pwsh -NoProfile -File checks/meetings/verify-durability.ps1`
from the project root; add `-Endurance` for the synthetic two-hour audio test.

The [online mail and local index specification](../docs/posta-online-specifica.md) describes the storage roadmap. Per-account limits, explicit offline downloads and a text-free semantic index are implemented; the separate SQLite migration and provider-hosted draft workflow remain planned.

## Language

**Settings → General → Language** chooses Italian, English or Automatic. Automatic
follows the regional format. The choice applies immediately, is stored on this device
and is independent from date and number formats. Text written by the backend is
translated for display by `src/lib/backend-text.ts`; add new backend messages there.

## Google sign-in

**Settings → Accounts → Sign in with Google** opens Google's official sign-in page
in the system browser, with a progress dialog in Qamille. Account passwords are
entered only on Google. **How to set up Google sign-in** opens a five-step guide
inside the app with direct links to project creation, Google APIs, consent setup
and desktop OAuth clients. **Enter credentials** reveals the developer settings.
The same guide is available when sign-in fails.

The app distributor configures a Google Desktop OAuth client once; regular users
then only sign in and approve access. Builds without an app client can use the
personal developer configuration. See the [Google setup guide](../docs/oauth-google.md)
for build configuration, test users and public distribution requirements.
Run `node tests/google-sign-in.mjs` against a local Vite server to verify this flow
with synthetic sign-in responses.

## Meeting reminders

Shortly before an online meeting, a chip appears at the top centre of the screen,
above other windows, with the meeting title, its time and **Join**, **Join and
record** and **×**. **Join and record** opens the link and starts a recording with
the default microphone and system audio. Turn reminders off or choose 1, 2, 5 or
10 minutes ahead in **Settings → General**. Qamille must be running, even
minimized.

Local events and the primary Google calendar of each account with calendar access
are checked every 15 seconds; Google is read at most once a minute, read-only.
The chip window (`reminder.html`) only names a chip; the engine opens the link.

Each window can call only the commands its capability grants
(`src-tauri/capabilities/`). The chip gets its three `reminder_*` commands; the
main window gets every other command through the `main-window-commands` set,
which `src-tauri/build.rs` regenerates in `src-tauri/permissions/main-window.toml`
from `generate_handler!` in `src-tauri/src/main.rs`. A new command therefore
needs no permission edit, unless only the chip should call it.

Behaviour and limits: [specification](../docs/specs/2026-11-03-promemoria-riunioni-design.md),
[ADR-0004](../docs/adr/ADR-0004-promemoria-riunioni.md).

## Storage for each account

Open **Settings → Local storage**, select an account and choose **Limited** with a
decimal GB value or **Unlimited**. New accounts start at 2 GB. Existing data files
without a per-account policy keep Unlimited until the user chooses a limit, so
installing the update does not silently remove their local mail.

The quota counts serialized mail/account data, drafts, referenced attachments and
semantic index records. The shared model is separate. Ten percent of a limited
account's budget is reserved for pending commits. This remains accounting over the
JSON compatibility repository, not an operating-system filesystem quota; shared
JSON formatting and temporary replacement files are not charged precisely to each
account. Windows disk-space checks protect writes, including Unlimited accounts.

When space is required, the oldest replaceable mail in that account is removed
locally. Other accounts and Gmail originals stay unchanged. Task references, local
messages without a verified remote locator, drafts and pending actions are protected.
Reducing a limit previews cleanup and requires an explicit **Apply limit** action.

Normal Gmail sync retrieves headers and attachment descriptions. Opening an online
message verifies its saved Gmail location first and retrieves only its text MIME part,
without transferring or extracting attachments. The reader reuses a dedicated read-only
Gmail connection; a stale connection reconnects on demand. When a message has an HTML
alternative, the reader fetches it together with the plain text in one request and displays
it sanitized in a sandboxed frame; remote images stay blocked until **Show images**, and
links open in the system browser. The HTML stays in memory and is never written to the
offline repository. Attachment previews retrieve and extract content on request.
The first five visible rows are prefetched after a short delay; hovering or focusing a
row prioritizes it. One background worker shares in-flight downloads with clicks, yields
to foreground reads, and replaces queued work when the visible list changes. Prefetch
uses PEEK, leaves read flags unchanged, and never writes bodies to the offline repository.
Recently opened and prefetched online messages share a bounded in-memory
session cache (20 messages, at most 16 MiB of message and extracted attachment text);
reopening them preserves current local flags and does not write bodies to disk.
The clicked row and reading pane show the pending selection immediately; conversation
details load after the message. **Keep all mail offline** instead
requests resumable body/attachment downloads from Gmail's exposed folders, with
64 MiB of new MIME content per pass. Missing folders, oversized messages or an
insufficient quota leave a visible partial state. Protected offline contents are
never removed to make room; use **Sync mail** and **Refresh storage status** to check
progress.

Semantic indexing uses up to 10,000 known messages from the last 90 days, with a
bounded temporary body download and no automatic attachment extraction. New index
files store vectors and source references, not passage text. Rebuilding replaces
the old index format; existing text-bearing indexes are preserved until that explicit
operation or a requested local cleanup.

## Compute and editions

Settings → Compute chooses where indexing, semantic search and transcription run: **No server** (default: mail and keyword search only; nothing leaves the machine) or a **Qamille server** (`server/`, even your own PC with Docker, see `docs/server-sul-pc.md`). The guided form probes the address with `GET /v1/info`, asks consent once per address before sending credentials, signs in (temporary password → new password, device limit → disable one) and keeps one session in the vault (`server-session`); the password is never stored. Connected, the section lists devices and offers change password, change server (self-hosted only), sign out and delete account.

Two builds of the same app (`app/src/edition.rs`):

- **Self-hosted**: `build-release.bat` → `release\`, installed into `D:\Software\Qamille`; the server address is written by hand.
- **Professional**: `set QAMILLE_SERVER=https://…` then `build-release.bat` → `release\`, installed into `D:\Software\Qamille-Professional`, plus the per-user NSIS installer in `release\installer\` (`src-tauri/tauri.professional.conf.json`). `QAMILLE_PRESET_SERVER` is validated by `app/build.rs`: an invalid value fails the build. The first start shows «Sign in to Qamille» with «Continue without an account», which opens the app as a mail client.

Preview scenarios: set `window.__previewServer = { edition: 'professional' | 'self-hosted', probe: 'found' | 'unreachable' | 'not_qamille' | 'incompatible', login: 'ok' | 'wrong' | 'temporary' | 'limit', offline: true, unlinked: true, mismatch: true }` before the app loads (`unlinked`: the server answers `401` after the sign-in until «Sign in again»; `mismatch`: it serves another model than the index until «Reindex», compute wave 3). Tests: `node --test tests/server-connect-logic.mjs`, and against the preview `tests/server-connect.mjs`, `tests/compute-settings.mjs`, `tests/compute-remote.mjs`, `tests/onboarding-professional.mjs`.

## Run

The main desktop window is borderless. Drag the header or Qamille logo to move
it; double-click the header to maximize or restore. The header's minimize,
maximize and close buttons follow the selected theme. Close still checks unsaved
settings and saves open drafts and meeting notes. Run `node tests/window-native.mjs`
after a standalone debug build to verify the controls with isolated sample data.

On Windows, double-click **build-release.bat** in the project root to install
the locked frontend dependencies (when `node_modules` is missing), build the interface
and compile Rust in Release mode. The ready-to-run app is written to **release/Qamille.exe**
and installed into **D:\Software\Qamille** (its `data` folder stays).
Node.js, Rust and the Visual Studio C++ build tools must be installed. The first
build downloads dependencies. Errors remain visible in the build window.
Use `build-release.bat --no-install` to stop at `release\`. `Start-Qamille.cmd`
prefers this Release build when present; it falls back to the development build.

From the project directory, double-click `Start-Qamille.cmd` after a standalone
build. Double-click `Preview-Qamille.cmd` for a separate sample workspace.

Qamille is portable by default. All app-owned data lives in **data/** beside
the executable: mail, drafts and copied attachments, meetings, search indexes,
downloaded models, credentials and WebView preferences. Copy the entire Qamille
folder to move it. The folder must be writable; the app never falls back to the
Windows profile. Credentials use a local encrypted vault and its local key;
anyone with the complete folder can use the saved credentials. Keep the folder
private and include it in backups. `QAMILLE_DATA` remains an explicit workspace
override for tests and external tools. The normal launcher clears inherited
workspace/demo overrides; the preview launcher uses **preview/data/** beside the
executable. Existing Windows-profile workspaces are not imported automatically.

Normal startup creates an empty workspace: connect an account in **Settings →
Accounts** to load real mail. Synthetic accounts and mail are seeded only when
`QAMILLE_DEMO=1`, as set by `Preview-Qamille.cmd`. Preview and normal desktop
windows have separate single-instance identities, so opening the normal app does
not focus an already-open preview. Existing saved workspaces are preserved,
including demo data saved by older versions; rebuilding does not reset local data.
After a Release build, `npm run test:startup:native` checks empty startup, restart,
account settings and the separate demo window with disposable workspaces. Close
existing Qamille windows before running it.

Development:

```powershell
cd desktop
npm ci
npm run desktop
```

Standalone debug build (no development server required):

```powershell
cd desktop
npm run build
cd src-tauri
cargo build --locked --features custom-protocol
```

The executable is `app/target/debug/qamille-desktop.exe`. The first build needs
Rust, Microsoft's C++ build tools, WebView2, and the existing ONNX runtime download.
This is a debug build, not a performance or installer release.

## Implemented in Tauri

Gmail accounts use a persistent IMAP IDLE notification connection while the desktop
is open. A mailbox change triggers an inbox download and refreshes the interface
without waiting for the periodic timer. Startup and reconnect perform catch-up
sync; failures retry with backoff from 1 to 60 seconds, sessions renew after 50
minutes, and a five-minute recovery sync handles missed notifications. Newly
connected accounts begin listening within five seconds. The manual Sync mail
button remains available. This uses existing Gmail sign-in; no Google Cloud
Pub/Sub setup is required.

After connecting an account or requesting a manual sync, a small overlay shows
preparation and completion for three seconds, then closes automatically. It does
not move the page or require dismissal. Routine background sync shows only the
single activity indicator; the Sync mail button stays clickable and carries no
spinner of its own. Errors appear for six seconds with **Try again**;
afterwards the toolbar retains the error details and remains available to retry.
The empty inbox no longer repeats the sync phase: the indicator is the only
persistent representation while a download runs. First
sync starts when the account is discovered (within five seconds), even if the
IDLE notification connection is unavailable. Status is restored from the running
service when the interface opens; no estimated percentage is shown.
Regression checks: `npm run test:sync` against the local preview server (set
`QAMILLE_PREVIEW_URL` for a port other than 5187), plus the `sync_feedback`
service tests and `application::mail_sync` unit tests in `app`.
`npm run test:sync:native` verifies the packaged background worker and retry UI
using a disposable account without credentials; it never connects to Google.

Incoming sync updates read/unread, starred and replied flags in both directions
while preserving local tasks and pending Undo operations. Normal online sync covers
the account's latest Inbox window (200 messages by default). Explicit offline mode
also scans the exposed Gmail folder roles and reports incomplete coverage when
necessary. Google Calendar remains local-only.

**Settings → Local storage** is the only control for how much mail stays on this
computer ([ADR-0005](../docs/adr/ADR-0005-archivio-unico-per-account.md)); see
[Storage for each account](#storage-for-each-account). The former global
**Offline mail** age and size limits were removed. Data files that still contain
them open normally, evict nothing and drop the fields on the next save.

- **Emails / Threads** at the top of the message list switches between individual messages and
  account-scoped conversations. Thread counts and bulk selection include messages
  matching the current folder and search; the reading pane lets you open any
  locally synced member, including replies in other folders. Reader actions apply
  to the displayed message. Conversations are grouped before pagination.
- **Collapse sidebar** keeps account avatars, folders, labels, tools and settings
  accessible as icons with tooltips. **Expand sidebar** restores their names.
  Sidebar size and mail display are remembered on this device.
- Smart Inbox, account filter, keyword search, pagination, message reading and
  attachment previews. Mail bodies are fetched only when opened.
- Archive, trash, read/unread, pins, done, snooze and local task views. Gmail actions
  use the existing Rust IMAP implementation. Provider failures remain visible.
- Rich-text drafts with autosave, explicit save on close, signatures, attachments,
  reply/forward, sending and scheduling while the app is running.
- Multiple draft tabs in the footer. Open several messages with **New message**
  or the footer **+**; click a tab to switch, and use **−** or **Esc** to collapse
  its editor. The inbox remains usable. **×** saves and closes just that tab.
  Each draft keeps its own text, formatting and autosave while collapsed. Closing
  the desktop waits for every open draft to save; a failed save keeps it open.
- Settings, Google sign-in, app passwords, local calendar, existing meeting notes,
  assistant questions and local search-model management.
- Light/dark/automatic appearance and keyboard shortcuts: Ctrl+K, N, R, E, arrows. **Settings → General → Appearance** selects Light, Dark or Automatic; Automatic follows the operating-system theme immediately. The choice is stored on this device, also applies to the meeting reminder window and adapts sandboxed HTML email content without changing the saved message. The toolbar button switches between explicit Light and Dark.
- Email actions (including Done, Send, scheduling and discarding drafts) show an
  Undo notification at the bottom left with a countdown and progress bar. The
  default is 5 seconds; Settings → General → Undo time offers 5–30 seconds.
  Ctrl+Z outside the editor undoes the latest available action.

Pending actions update the displayed mailbox immediately, but provider changes
and delivery wait until the countdown ends. Each action has its own Undo button.
Several changes to one message are applied together after their deadlines, so a
sequence of moves uses only the final destination. Keep the app open to apply
pending actions; they are saved and resume on reopening. An interrupted operation
whose server result is unknown is shown for review and never automatically retried.
Done messages appear in Archive as well as being archived on the original account.
- IBM Plex Sans is the interface font, bundled locally for offline use, including
  its variable weights. Font files and the SIL Open Font License are in
  `public/fonts/ibm-plex-sans/`.

## Attachment previews

Clicking a PDF, DOCX, XLSX or PPTX attachment opens a visual preview in the reading
pane. The pages are rendered by the ArtCraft engines (PdfCraft, WordCraft, GridCraft,
DeckCraft) compiled to WebAssembly and **embedded in the app**: nothing is installed or
downloaded, and no external program is started. Each document runs in its own Web
Worker; the WebAssembly module only sees the bytes it is given, with no access to
files, network or the desktop bridge. Macro-enabled files (`.xlsm`, `.pptm`) and legacy
Office formats are not previewed.

Flow: `attachment_preview_source` returns the document as one raw IPC frame
(`[u32 length][JSON {kind}][bytes]`, 25 MiB limit; online and nested attachments are
retrieved on demand like the text preview). `src/lib/embedded-preview/` decodes it and
the worker loads the engine for that kind, renders PNG pages at the chosen zoom and is
terminated when the panel closes, the attachment changes or an operation exceeds 60
seconds. Unsupported types, encrypted PDFs and rejected documents show the error and
offer the extracted text. The browser sample preview never renders real documents.

The engines come from the `artcraft-web` repository (public, `acaprino/artcraft-web`).
`npm run artcraft:sync` (run automatically before `dev` and `build`) takes the latest
GitHub release of it (plain HTTPS, no sign-in; `scripts/artcraft-github.mjs` checks each package
against the release's SHA-256 and caches it in `.cache/artcraft-web`), used only when its tag
is on artcraft-web's `main`, or `dist/` of `ARTCRAFT_WEB_DIR` when set (engines not released
yet); without the network the existing copy stays. It
copies it into `src/vendor/artcraft/` and the licences into `public/artcraft-licenses/`,
after checking every file against its `provenance.json`. Qamille always takes the latest
release of artcraft-web's main, with no pin of its own. Both folders
are generated and ignored by git. The app CSP allows `'wasm-unsafe-eval'` and
`connect-src 'self'` for this; `'unsafe-eval'` stays off.

Checks: `npm run test:attachment-preview`, `npm run test:attachment-panel`,
`npm run test:embedded-preview` (real engines under the app CSP), and in `app`:

```powershell
cargo test --offline --no-default-features --test services document_preview
```
## Print and export email

Open a message, choose **More actions** in the toolbar, then **Print email**,
**Print thread**, or **Download .eml**.
Printing opens a preview with sender, recipients, full date, plain-text body and
attachment names. **Print / Save PDF** opens the system print dialog. Each message
starts on a new page; attachment contents are included in downloads, not printed.

Select messages with the checkboxes in the message list. **Print selected** and
**Download .mbox** then appear above the list.
**Select loaded** selects the messages currently loaded in the active list/filter;
use **Load more messages** to include older mail. Selection resets when changing
folder, account, search or inbox tab.

Received Gmail exports download original MIME bytes, including HTML and attachments,
without marking messages read. MBOX uses mboxrd escaping. Local/sample messages are
rebuilt from their saved content and available attachments. Missing originals or
attachments stop the export before any output is saved. The native Save dialog
chooses the destination; canceling it saves nothing.

**Print thread** retrieves the Gmail conversation across All Mail (including Sent
and Archive), Trash and Junk when those folders are exposed over IMAP. It requires
an internet connection and All Mail access, and reports failures instead of silently
printing only the inbox copy. Sample conversations use account-scoped local links.

Checks: `node tests/mail-export.mjs` against the local preview server and
`cargo test --no-default-features --test mail_export --test services --test layers`
in `app`. The browser test uses only synthetic mail and verifies downloads, selection,
print CSS, thread membership and PDF generation. Gmail network behavior still needs
a live-account integration check.

## Migration boundaries

The old egui app remains available with `cargo run --manifest-path app/Cargo.toml`.
Do not run the old UI and the Tauri UI against the same data simultaneously: the
legacy UI still keeps a full in-memory snapshot. Tauri and MCP share transaction
locking; the legacy UI does not yet merge concurrent edits.

Meeting recording/transcription controls are now in Tauri. Windows captures the
selected microphone and output device separately; recording requires an explicit
click. Remote transcription uses **Settings → Assistant → OpenAI-compatible**,
its endpoint and API key, plus the transcription model selected in Meeting.
The endpoint must expose `/audio/transcriptions`. Transcription with the Qamille server's Whisper engine
runs on the server; without a server the recording is saved and waits.
Both modes process saved audio after recording ends. Summary generation uses the
separate AI text model already configured in Settings.

The complete AI writing toolbar and some advanced mail actions remain in the native
UI. Once the meeting archive exists, the updated legacy UI keeps meetings read-only.
An operating-system lock prevents both UI processes opening the same workspace.
Calendar is local, as before; this migration does not add Google Calendar sync.
MCP retains its old tool dispatcher and permission checks; its persistence now uses
the shared repository. It is not yet fully routed through the new use-case services.

Scheduled sends are claimed on disk before SMTP is contacted. After a failed or
interrupted attempt, the message remains a draft for explicit review instead of an
automatic retry. This does not provide exactly-once SMTP delivery.

The browser preview uses sample data in memory, cannot send mail and does not
access local user data. Its behavior is only for visual interaction checks.

## Checks

```powershell
cd app
cargo test --no-default-features --test services
# Set QAMILLE_DATA to a scratch directory before running the existing suite.
cargo test --no-default-features --lib
cd ../desktop
npm run build
# Start npm run dev in another terminal first.
npm test
npm run test:undo
npm run test:composers
npm run test:attachment-preview
npm run test:document-preview
```

Native test scripts select English labels: start them on sample data whose regional
format is en-US. Set `WEBVIEW2_USER_DATA_FOLDER` to a test folder, because otherwise the
test window shares browser storage, including language and sidebar preferences, with the
real app.

`tests/smoke.mjs` and `tests/composer-tabs.mjs` also accept `QAMILLE_CDP=http://127.0.0.1:9223` to exercise a
Tauri test window with WebView2 remote debugging enabled. Use only a sample data
directory for this test: its send check uses a Sample account. The script refuses
to proceed if any connected account is not a sample. Set `QAMILLE_TEST_DATA_DIR`
to that same directory to also verify draft persistence after closing the window.

`npm audit` reports no vulnerabilities for the current dependency set (React 19, Ark UI for
React, Tailwind CSS v4, Motion).
