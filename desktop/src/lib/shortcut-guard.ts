// The single-key shortcuts of App.svelte (N, R, E, the arrows, Ctrl+Z) belong to the page, not to
// a control that is typing or choosing. A key pressed inside a text field, an editable area or a
// part of an Ark widget that reads keys itself (the listbox of a Select or Combobox, a tags input,
// a date picker, the thumb of a slider) is the widget's: first-letter search in a list must never archive the message.
const OWN_KEYS =
  'input,textarea,select,[contenteditable]:not([contenteditable="false"]),' +
  '[data-scope="select"],[data-scope="combobox"],[data-scope="tags-input"],[data-scope="date-picker"],' +
  '[data-scope="slider"],[role="slider"]';

export function shortcutIgnored(target: EventTarget | null): boolean {
  return target instanceof Element && target.closest(OWN_KEYS) !== null;
}

// A dialog, popover or menu owns the keys while it holds the focus, and an open modal dialog owns
// them wherever the focus is: E in a confirmation must not archive the message behind it.
const LAYERS = '[role="dialog"],[role="alertdialog"],[role="menu"]';
const OPEN_MODAL = '[aria-modal="true"][data-state="open"]';

export function shortcutCoveredByLayer(target: EventTarget | null, doc: Document = document): boolean {
  if (target instanceof Element && target.closest(LAYERS)) return true;
  return doc.querySelector(OPEN_MODAL) !== null;
}

// Zoom is the app's, not the browser's (WebView2's own shortcuts are off, docs/aspetto-nativo.md
// rule 8): Ctrl with = or + zooms in, Ctrl with - zooms out, Ctrl+0 goes back to the default. Main
// keys and numeric keypad, with or without Shift (on an Italian keyboard + is a key of its own).
export type ZoomKey = 'in' | 'out' | 'reset';

export function zoomKey(event: Pick<KeyboardEvent, 'key' | 'code' | 'ctrlKey' | 'metaKey' | 'altKey'>): ZoomKey | null {
  if (!(event.ctrlKey || event.metaKey) || event.altKey) return null;
  if (event.key === '=' || event.key === '+' || event.code === 'NumpadAdd') return 'in';
  if (event.key === '-' || event.key === '_' || event.code === 'NumpadSubtract') return 'out';
  // Not `code === 'Numpad0'`: with Num Lock off that key is Insert, and Ctrl+Ins is copy.
  if (event.key === '0' || event.code === 'Digit0') return 'reset';
  return null;
}
