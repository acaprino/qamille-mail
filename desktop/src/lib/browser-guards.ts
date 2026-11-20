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

// What a click on a link follows: `a`, an image map's `area`, and an SVG `a`, which has `href` or the
// older `xlink:href` (the HTML parser puts that one in the XLink namespace; `*|href` takes both).
const LINK = 'a[*|href], area[*|href]';
const XLINK = 'http://www.w3.org/1999/xlink';

/** The link `element` is, or is inside, as written (`href`, else `xlink:href`); null outside a link. */
export function linkHref(element: Element): string | null {
  const link = closest(element, LINK);
  return link && (link.getAttribute('href') ?? link.getAttributeNS(XLINK, 'href'));
}

/** The address of the link `element` is, or is inside, resolved against its document: `a[href]`,
 *  `area[href]`, an SVG `a`. Null outside a link, or when the address is not one. */
export function linkTarget(element: Element): string | null {
  const href = linkHref(element);
  if (href === null) return null;
  try {
    return new URL(href, element.ownerDocument.baseURI).href;
  } catch {
    return null;
  }
}

/** `target`: the page's window, or the message frame's (MessageHtml.tsx), which the page's own
 *  listeners never see. */
export function installBrowserGuards(target: Window = window): () => void {
  const history = (event: MouseEvent) => {
    if (isHistoryButton(event.button)) event.preventDefault();
  };
  const link = (event: MouseEvent) => {
    const onLink = linkTarget(event.target as Element) !== null;
    if (opensLinkElsewhere(event, onLink)) event.preventDefault();
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
