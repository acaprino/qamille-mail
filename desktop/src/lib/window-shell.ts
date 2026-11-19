// What the custom title bar must hand to Windows (docs/aspetto-nativo.md, audit F4 F7 F9 F12):
// the system menu on a right-click of the bar, where the maximize button is (snap layouts), the
// app zoom and the appearance. Every call goes to src-tauri/src/window_shell.rs; outside Tauri
// (the browser preview and its tests) nothing here does anything.
import { call, desktop } from './api';
import type { Appearance } from './appearance';
import type { ZoomKey } from './shortcut-guard';

/** A right-click on a drag region (the title bar, the sidebar heading) is a right-click on a
 *  title bar: the Windows system menu, not WebView2's (empty) context menu. */
export function dragRegionTarget(target: EventTarget | null): boolean {
  return target instanceof Element && target.hasAttribute('data-tauri-drag-region') &&
    target.getAttribute('data-tauri-drag-region') !== 'false';
}

export function installWindowShell(): () => void {
  if (!desktop) return () => {};
  const onContextMenu = (event: MouseEvent) => {
    if (event.defaultPrevented || !dragRegionTarget(event.target)) return;
    event.preventDefault();
    const scale = window.devicePixelRatio || 1;
    void call('show_system_menu', { x: event.clientX * scale, y: event.clientY * scale }).catch(() => {});
  };
  // The Windows text size can change while the app is in the background.
  const onFocus = () => void zoomWindow(null);
  window.addEventListener('contextmenu', onContextMenu);
  window.addEventListener('focus', onFocus);
  return () => {
    window.removeEventListener('contextmenu', onContextMenu);
    window.removeEventListener('focus', onFocus);
  };
}

/** Ctrl+= / Ctrl+- / Ctrl+0, or `null` to pick up a new Windows text size. */
export async function zoomWindow(step: ZoomKey | null): Promise<void> {
  if (!desktop) return;
  try {
    await call('window_zoom', step ? { step } : {});
  } catch {
    // The page stays at its zoom.
  }
}

/** Title bar, WebView2 colour scheme (context menu, scrollbars) and the next start's first frame;
 *  the language of what Rust shows when the page cannot (system menu entry, recovery dialogs). */
export async function windowAppearance(appearance: Appearance, italian: boolean): Promise<void> {
  if (!desktop) return;
  try {
    await call('window_appearance', { appearance, italian });
  } catch {
    // Cosmetic only.
  }
}

/** The maximize button's place in client pixels, so Windows 11 shows snap layouts over it.
 *  `null` when it is gone. */
export function reportSnapButton(button: HTMLElement | null): void {
  if (!desktop) return;
  let rect: { x: number; y: number; width: number; height: number } | null = null;
  if (button?.isConnected) {
    const box = button.getBoundingClientRect();
    const scale = window.devicePixelRatio || 1;
    if (box.width > 0 && box.height > 0)
      rect = { x: box.left * scale, y: box.top * scale, width: box.width * scale, height: box.height * scale };
  }
  void call('window_snap_button', { rect }).catch(() => {});
}
