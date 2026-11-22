// The single-key shortcuts of App.svelte (N, R, E, the arrows, Ctrl+Z) belong to the page, not to
// a control that is typing or choosing. A key pressed inside a text field, an editable area or a
// part of an Ark widget that reads keys itself (the listbox of a Select or Combobox, a tags input,
// a date picker, the thumb of a slider) is the widget's: first-letter search in a list must never archive the message.
const OWN_KEYS =
  'input,textarea,select,[contenteditable]:not([contenteditable="false"]),' +
  '[role="listbox"],[data-slot^="select"],[data-scope="combobox"],[data-scope="tags-input"],[data-scope="date-picker"],' +
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

// The up and down arrows step through the messages only from the page itself, the mail list or the
// reader. In the sidebar, on a top-bar button or in the activity panel they belong to that control's
// own navigation: they must not open the next message, which also marks it read. On a menu button
// (the reader's "More actions") they open its menu, as on every menu button.
const MAIL_AREAS = '.mail-list,.reader';
const MENU_BUTTON = '[aria-haspopup="menu"]';

export function arrowsMoveMail(target: EventTarget | null): boolean {
  if (!(target instanceof Element) || target.closest(MENU_BUTTON)) return false;
  return target.localName === 'body' || target.closest(MAIL_AREAS) !== null;
}

// A chord of the app (Ctrl+K, Ctrl+Z) is Ctrl or Meta without Alt. AltGr, the key that types @ [ ] {
// and # on an Italian layout, reaches the page as Ctrl+Alt on Windows: it is a character, not a command.
export function isAppChord(event: { ctrlKey: boolean; metaKey: boolean; altKey: boolean }): boolean {
  return (event.ctrlKey || event.metaKey) && !event.altKey;
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
