// Where the focus goes back to when a surface that the caller closes by unmounting it goes away.
// Unmounting gives Ark no chance to restore the focus, so the focus would fall to <body>: `Modal` and
// `SidePanel` both use this hook to put it back on the element that had it when they appeared.
import { useEffect, useState, type RefObject } from 'react';

/** The button that opened the popup (`aria-controls`, as Ark sets it) holding `element`, if any. */
function popupTrigger(element: HTMLElement | null): HTMLElement | null {
  for (let node = element?.parentElement; node; node = node.parentElement) {
    if (!node.id) continue;
    const trigger = document.querySelector<HTMLElement>(`[aria-controls="${CSS.escape(node.id)}"]`);
    if (trigger && !node.contains(trigger)) return trigger;
  }
  return null;
}

/**
 * Remembers the element that has the focus when the surface mounts and, when the surface unmounts with
 * the focus inside it or on <body>, puts the focus back there if it is still in the page. Opened from an
 * item of a menu or popover that closes meanwhile, the opener is gone: it goes back to that popup's
 * trigger instead. A focus that moved elsewhere on its own (a click on the page behind a non-modal
 * panel) is left alone.
 */
export function useReturnFocus(surface: RefObject<HTMLElement | null>) {
  const [opener] = useState(() =>
    document.activeElement instanceof HTMLElement ? document.activeElement : null,
  );
  const [fallback] = useState(() => popupTrigger(opener));
  useEffect(() => {
    // The node is read when the surface goes: the ref is already null by then.
    const element = surface.current;
    return () => {
      const lost = document.activeElement === document.body || !!element?.contains(document.activeElement);
      const target = opener?.isConnected ? opener : fallback?.isConnected ? fallback : null;
      if (lost && target) target.focus();
    };
  }, [surface, opener, fallback]);
}
