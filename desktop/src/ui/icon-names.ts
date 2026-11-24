// The icon names Qamille has always used, each on the meaning Sigrid draws (its `ICON_NAMES`). The drawing is Sigrid's, in
// the pack in context (Lucide by default); Qamille keeps no path of its own. A name that is not here draws `mail`, as the old
// primitive did. `tests/sigrid-preconditions.mjs` (P5) checks that every target is in the synced copy's `ICON_NAMES`.
import type { IconName } from '@acaprino/sigrid/primitives';

export const ICON_MAP = {
  threads: 'threads',
  'sidebar-close': 'sidebarClose',
  'sidebar-open': 'sidebarOpen',
  print: 'print',
  download: 'download',
  upload: 'upload',
  underline: 'underline',
  'list-ordered': 'listOrdered',
  quote: 'quote',
  undo: 'undo',
  redo: 'redo',
  inbox: 'inbox',
  mail: 'mail',
  check: 'check',
  minus: 'minus',
  task: 'tasks',
  clock: 'clock',
  history: 'history',
  edit: 'rename',
  send: 'send',
  archive: 'archive',
  trash: 'delete',
  calendar: 'date',
  contacts: 'contacts',
  mic: 'mic',
  spark: 'ai',
  settings: 'settings',
  search: 'search',
  plus: 'plus',
  chevron: 'chevron',
  down: 'chevronDown',
  pin: 'pin',
  clip: 'attach',
  refresh: 'refresh',
  reply: 'reply',
  replyAll: 'replyAll',
  arrow: 'moveRight',
  close: 'close',
  minimize: 'windowMinimize',
  'window-maximize': 'windowMaximize',
  'window-restore': 'windowRestore',
  moon: 'moon',
  sun: 'sun',
  more: 'more',
  tag: 'tag',
  link: 'link',
  bold: 'bold',
  italic: 'italic',
  list: 'bulletList',
  info: 'info',
  logout: 'signOut',
  copy: 'copy',
  pause: 'pause',
  play: 'play',
  stop: 'stop',
  video: 'video',
  alert: 'alert',
} as const satisfies Record<string, IconName>;

export type QamilleIconName = keyof typeof ICON_MAP;

/** The meaning Sigrid draws for a name of Qamille's. */
export function sigridIcon(name: string): IconName {
  return Object.prototype.hasOwnProperty.call(ICON_MAP, name) ? ICON_MAP[name as QamilleIconName] : 'mail';
}
