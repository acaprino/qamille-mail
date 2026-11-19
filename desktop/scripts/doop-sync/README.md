# Doop sync

Keeps the Qamille app and the Doop canvas "Qamille" (`https://doop.design/c/APLIvnJwhl`) aligned.
Doop has no built-in canvas-to-code sync: its "Live app" snippet only pushes app -> canvas. Everything else goes through an agent that talks to Doop over MCP and edits the code.

These scripts live under git in `desktop/scripts/doop-sync/` (ADR-0011). Commands below run from the repository root. The token file they read and write is `desktop/src/ds/tokens/theme.css`. The scripts never call Doop: the agent does, with the files they produce.

## Design tokens

`desktop/src/ds/tokens/theme.css` <-> the Doop style guide `qamille-design-system`. `tokens.lock.json` is the common base; `tokens.mjs` does a three-way comparison against it.

App changed (theme.css):
1. `node desktop/scripts/doop-sync/tokens.mjs export --out guideline.md`
2. Agent: `set_guidelines` (canvas `APLIvnJwhl`, name `qamille-design-system`) with that file.
3. `node desktop/scripts/doop-sync/tokens.mjs mark-synced guideline.md`

Doop changed (someone edited the style guide):
1. Agent: `get_guidelines`, save the markdown to a file.
2. `node desktop/scripts/doop-sync/tokens.mjs plan file.md` lists Doop-only, app-only and conflicting tokens.
3. `node desktop/scripts/doop-sync/tokens.mjs pull file.md --write` applies Doop-only changes. Conflicts need `--prefer=doop|app`; new Doop tokens need `--add`.
4. Run `npm run check` in `desktop/`, then do the "App changed" steps.

## Screens

Frames on the canvas are real HTML of the app's browser preview (`npm run dev`, sample data, Italian), with the prototype wiring (`data-doop-interactions`) on the real elements. `frames.json` maps each state to its Doop frame id.

App -> Doop:
1. `node desktop/scripts/doop-sync/extract.mjs --out <dir> --verify <vdir>` (optional `--only inbox-maria,calendar`). It starts its own Vite server, fixes locale (it-IT), time zone (Europe/Rome) and clock (2026-11-03), captures each state and writes `<dir>/<state>.html`.
2. The same run renders each file on its own and diffs it against the live app: `python desktop/scripts/doop-sync/diff.py <vdir>/<state>.real.png <vdir>/<state>.html.png`. The expected result is 0 differing pixels.
3. Agent: `set_frame_html` for each changed state with the file's content. Frame ids do not change, so the prototype links stay valid.

Doop -> app:
1. Someone edits a frame in Doop and marks it "ready for dev".
2. Agent: `get_canvas(approval_status="ready_for_dev")`, `get_frame`, compare with the last pushed file, edit the Svelte components, run the checks.
3. App -> Doop again, so the frame matches the code.

Pushing a changed HTML frame without resending 30–65 KB: `get_frame` the current frame (a large result lands in a file), then `python desktop/scripts/doop-sync/syncframe.py <get_frame file> <out>/<state>.html <plan dir>` with `GAP=150` in the environment. It reuses the frame's icon symbol ids (the extractor renumbers them whenever an icon is added, which changes nothing on screen), writes `<state>.target.html` and `<state>.edits.json`, and checks that the edits reproduce the target exactly. Apply the edits in order with `edit_frame_html`, then `get_frame` again and compare with the target.

Screenshot frames: `node desktop/scripts/doop-sync/shots.mjs <dir>` captures inbox-luca, inbox-sofia, inbox-github, composer-reply, calendar-new-event, meetings-recorder and settings-agent with the same setup as `extract.mjs`, measures the hotspots on the live DOM and writes `<state>.png` and `<state>.frame.html`. Upload each PNG, put its URL in place of `IMAGE_URL`, push with `set_frame_html`. Hotspots must be measured again whenever the sidebar or a layout changes.

States exported as HTML: inbox-maria, composer-new, calendar, meetings, assistant, settings. The other frames in `frames.json` (other messages, reply, new event, recorder, agent access) are screenshots of the same app and are replaced when their state is added to `STATES` in `extract.mjs`.

Notes:
- Doop's own thumbnails (`/i/<frame>.png`) rasterise text slightly differently from Windows. Geometry matches; text edges differ. Judge fidelity in the desktop app or the browser, not in the thumbnail.
- Do not export the Assistant answer: the browser preview shows an error ("needs the desktop app").
