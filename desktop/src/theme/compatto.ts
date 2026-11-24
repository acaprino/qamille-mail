// Qamille's look as Sigrid's parameters: «Compatto», the only look the app has (the user chooses light, dark or the
// system's). Qamille picks one colour, the accent; Sigrid's `colorTheme` gives the grays, the sidebar, the surfaces and
// the corners, in light and in dark, and its theme engine computes every token from them. Qamille writes no colour of its
// own for what the engine owns. `npm run theme:css` turns the pick into src/theme/compatto.css, which
// tests/theme-css.mjs keeps equal to the engine's output.
//
// The plain `import type` leaves nothing at run time: the module loads in Node (scripts/theme-css.mjs, the tests).
import type { ColorThemePick, DensityOverride, ThemeTone } from '@acaprino/sigrid';

/** Qamille's colour: only the accent (an indigo). */
export const COMPATTO_PICK: ColorThemePick = { id: 'compatto', label: 'Compatto', color: '#4f46e5' };

/** As designed: the default tone (Vivid) would move the indigo to a more saturated, darker one. */
export const COMPATTO_TONE: ThemeTone = { intensity: 'theme', light: 'theme' };

/**
 * Qamille's density: Sigrid's `compact` (32/28/24 px controls) with the two smaller heights at 32, because every
 * control of Qamille is 32 px high today.
 */
export const QAMILLE_DENSITY: DensityOverride = { id: 'compatto', label: 'Compatto', control: { sm: 32, xs: 32 } };
