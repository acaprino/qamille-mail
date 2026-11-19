// Mouse gestures a browser answers and a Windows app does not (docs/aspetto-nativo.md, rule 23):
// the mouse's back and forward buttons navigate the page's history, a middle click or Ctrl+click
// on a link opens it in a new window. The keyboard ones (Alt+←, F5) are off in WebView2 already
// (native_feel.rs); these need the page. Every window installs them from its entry point.

/** Back (3) and forward (4) mouse buttons. */
export function isHistoryButton(button: number): boolean {
  return button === 3 || button === 4;
}

/** A middle click, or a modified click, on a link: the browser would open it in a new window. */
export function opensLinkElsewhere(
  event: Pick<MouseEvent, 'button' | 'ctrlKey' | 'shiftKey' | 'metaKey'>,
  onLink: boolean,
): boolean {
  return onLink && (event.button === 1 || (event.button === 0 && (event.ctrlKey || event.shiftKey || event.metaKey)));
}

/** A file, or a link dragged from another app, that no drop zone took: the browser would open it,
 *  navigating the page (or the message frame) to it. A link dropped into a field is its text. */
export function isStrayDrop(types: readonly string[], intoField: boolean): boolean {
  return types.includes('Files') || (types.includes('text/uri-list') && !intoField);
}

// Not `instanceof Element`: in the message frame the elements belong to the frame's own realm.
const closest = (target: EventTarget | null, selector: string) =>
  (target as Element | null)?.closest?.(selector) ?? null;
const FIELD = 'input, textarea, [contenteditable]:not([contenteditable="false"])';

/** `target`: the page's window, or the message frame's (MessageHtml.tsx), which the page's own
 *  listeners never see. */
export function installBrowserGuards(target: Window = window): () => void {
  const history = (event: MouseEvent) => {
    if (isHistoryButton(event.button)) event.preventDefault();
  };
  const link = (event: MouseEvent) => {
    if (opensLinkElsewhere(event, closest(event.target, 'a[href]') !== null)) event.preventDefault();
  };
  // Drop zones (mail rows, attachments) take their drops first and mark them handled.
  const drop = (event: DragEvent) => {
    if (event.defaultPrevented || !event.dataTransfer) return;
    if (!isStrayDrop([...event.dataTransfer.types], closest(event.target, FIELD) !== null)) return;
    event.preventDefault();
    if (event.type === 'dragover') event.dataTransfer.dropEffect = 'none';
  };
  target.addEventListener('mouseup', history);
  target.addEventListener('auxclick', link);
  target.addEventListener('click', link, true);
  target.addEventListener('dragover', drop);
  target.addEventListener('drop', drop);
  return () => {
    target.removeEventListener('mouseup', history);
    target.removeEventListener('auxclick', link);
    target.removeEventListener('click', link, true);
    target.removeEventListener('dragover', drop);
    target.removeEventListener('drop', drop);
  };
}
