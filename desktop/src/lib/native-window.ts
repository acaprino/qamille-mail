// Keeps the native window's background on the page's `--ground` (what html, body and
// .app-shell paint), so resizing or switching theme never shows a strip of another
// colour behind the page (docs/aspetto-nativo.md). Outside Tauri (browser tests) it does nothing.
export function windowColor(value: string): [number, number, number] | null {
  const hex = value.trim().replace(/^#/, '');
  const full = hex.length === 3 ? [...hex].map((c) => c + c).join('') : hex;
  if (!/^[0-9a-f]{6}$/i.test(full)) return null;
  return [0, 2, 4].map((at) => parseInt(full.slice(at, at + 2), 16)) as [number, number, number];
}

export async function paintWindowBackground(): Promise<void> {
  if (!('__TAURI_INTERNALS__' in window)) return;
  const color = windowColor(getComputedStyle(document.documentElement).getPropertyValue('--ground'));
  if (!color) return;
  try {
    const { getCurrentWindow } = await import('@tauri-apps/api/window');
    await getCurrentWindow().setBackgroundColor(color);
  } catch {
    // Cosmetic only: the page itself is already painted.
  }
}
