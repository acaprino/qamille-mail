# unlumen UI components

Copied from the unlumen UI registry (https://ui.unlumen.com, free components, MIT-style registry code; forked from animate-ui).
Edits from upstream: the `"use client"` line removed (no Next.js), apple-switch is gone (Switch is Sigrid's) — the `app` tone (Compatto colours: --muted off, --accent on, --on-accent thumb) and the `xs` size (the Compatto switch's 32×18 track), and reads Qamille tokens.

- Sidebar 001 (not copied): ds/primitives/NavList.tsx reimplements its hover highlight and sliding active bar.
- Animated List (not copied): the lists use Motion's AnimatePresence directly, keeping their own markup.
