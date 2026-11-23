// Qamille's look as Sigrid's parameters: «Compatto», the only look the app has (the user chooses light, dark or the
// system's). Sigrid's theme engine computes every colour from this definition; Qamille fixes the parameters and writes
// no colour of its own for what the engine owns. `npm run theme:css` turns them into src/theme/compatto.css, which
// tests/tokens-compatto.mjs keeps equal to the engine's output.
//
// The plain `import type` leaves nothing at run time: the module loads in Node (scripts/theme-css.mjs, the tests).
import type { DensityOverride, ThemeDef, ThemeTone } from '@acaprino/sigrid';

/**
 * The definition. The grays take their hue and chroma from Compatto's page, lines and ink; the accent is the OKLCH
 * saturation and hue of #4f46e5. `surfaces` hold Compatto's own page, sheet, lines, ink, accent text and fill in each
 * scheme; the sidebar of `surfaces` is the light lane, because `sidebar: 'scura'` makes the engine draw the dark column
 * itself (a dark colour here would push ink, lines and red to black and white). `radii` are Compatto's corners (4, 6, 8 and
 * 12 px), a parameter of the theme since Sigrid 3.2.1.
 */
export const COMPATTO: ThemeDef = {
  id: 'compatto',
  label: 'Compatto',
  hue: 262,
  chroma: 0.012,
  accent: [0.234, 277],
  sidebar: 'scura',
  palette: 'crisp',
  radii: { sm: 4, md: 6, lg: 8, xl: 12 },
  surfaces: {
    light: { paper: '#f6f7f9', sidebar: '#eef0f3', raised: '#ffffff', line: '#e3e6ea', ink: '#111827', accent: '#4338ca', fill: '#4f46e5' },
    dark: { paper: '#12161e', sidebar: '#161b25', raised: '#1a202c', line: '#2a3240', ink: '#e8ecf3', accent: '#aeb0fa', fill: '#9a9cf8' },
  },
};

/** As designed: the default tone (Vivid) would move the indigo to a more saturated, darker one. */
export const COMPATTO_TONE: ThemeTone = { intensity: 'theme', light: 'theme' };

/**
 * Qamille's density: Sigrid's `compact` (32/28/24 px controls) with the two smaller heights at 32, because every
 * control of Qamille is 32 px high today.
 */
export const QAMILLE_DENSITY: DensityOverride = { id: 'compatto', label: 'Compatto', control: { sm: 32, xs: 32 } };
