## Notes (not synced)

- Type: Source Sans 3 (variable) for everything that is scanned (`--font-text`), Lexend (variable) only for titles and the brand (`--font-title`). UI base `--text-sm` 14px; scale 12/13/14/15/16/18/22/26px. No independent study shows that either font helps readers with ADHD: they are a choice of look and density.
- Layout, desktop 1440×900: sidebar 232px, top bar 48px, mail list 360px. The sidebar and the top bar share the `--ground` fill; the work sits on one white sheet (`--surface`, 1px `--border`, radius `--radius-sheet`, 8px from the right and bottom edge). Hairline borders, no resting shadow: `--shadow-sm` is `none`; floating surfaces (side panels, dialogs, popovers, toasts, composer) use `--shadow`.
- One accent. Primary actions are solid accent with `--on-accent` text, radius `--radius-md`. The sidebar compose button uses `--radius-md` too. Active sidebar items are a neutral pill, not blue.
- Icons: stroke 1.7, 18px, rounded caps and joins (Lucide-style).
- Logo: blue rounded square (`--accent`, radius `--radius-lg`) with a bolt in `--on-accent`, viewBox 21×25: `M12 1 2 14h8l-1 10 10-14h-8l1-9Z`.
- Label dots use `--label-1..5` in order; sender avatars use `--sender-1..4` by the first letter of the name. Priority is amber: `--priority-mark` for the 3px left edge and the dot, `--priority-wash` for the row fill.
- Every color, shadow, radius, layer, duration and font size comes from these tokens. `desktop/tests/style-tokens.mjs` fails on a literal value anywhere else (ADR-0009).
- Overlays (ADR-0008), three containers only: side panel (`--panel-narrow` 420 for the calendar event, `--panel-wide` 760 for attachments; slides from the right in `--motion-panel` over `--scrim-soft`, non-modal, Esc closes), centered modal (`--modal-sm/md/lg` over `--scrim`, border `--border`, radius `--radius-xl`, `--shadow`) and popover (menus and filters anchored to their button, radius `--radius-lg`). Composer stays a docked window with the same radius and shadow.
- The visual system lives in `desktop/src/ds/` (ADR-0011): `tokens/theme.css`, `styles/system.css`, `primitives/`, and the guide `README.md`.
