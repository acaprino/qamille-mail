// Runs before the stylesheet paints, so a dark workspace never starts on a white page.
// Same rule as resolvedAppearance() in src/lib/appearance.ts: no saved choice means
// "system" (docs/aspetto-nativo.md).
let saved = null;
try {
  saved = localStorage.getItem('qamille:appearance') ?? localStorage.getItem('qemille:appearance') ?? localStorage.getItem('sparklone:appearance');
} catch {
  // Without storage the system theme decides.
}
const dark = saved === 'dark' || (saved !== 'light' && window.matchMedia('(prefers-color-scheme: dark)').matches);
document.documentElement.dataset.theme = dark ? 'dark' : 'light';
