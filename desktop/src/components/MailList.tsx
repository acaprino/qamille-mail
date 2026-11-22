import { AnimatePresence, motion, useIsPresent, useReducedMotion, type Variants } from '../ui/motion';
import { useEffect, useLayoutEffect, useRef, useState, type DragEvent, type KeyboardEvent, type MouseEvent } from 'react';
import Button from '../ds/primitives/Button';
import Checkbox from '../ds/primitives/Checkbox';
import Count from '../ds/primitives/Count';
import Icon from '../ds/primitives/Icon';
import Tabs from '../ds/primitives/Tabs';
import ThreadReplies from './ThreadReplies';
import { call, fullDate, shortDate } from '../lib/api';
import { markedParts } from '../lib/search';
import { startMailDrag } from '../lib/mail-drag';
import {
  conversationOf,
  groupState,
  hasUnreadReplies,
  isThreadOpen,
  reconcileThreads,
  repliesOf,
  threadState,
} from '../lib/groups';
import type { PersonIndex } from '../lib/people';
import { useStore } from '../lib/store';
import { visibleReplies } from '../lib/thread-tree';
import { backendText, fixedBackendText, folderLabel, sectionLabel, tr as t } from '../lib/i18n';
import { cn } from '../lib/utils';
import type { Account, ConversationReply, EmailSummary, Folder, MailPage } from '../lib/types';

export type MailListProps = {
  page: MailPage;
  view: Folder;
  selected: number | null;
  sectionLocks: Record<number, string>;
  /** The rows in the order the arrow keys walk them (was `bind:visibleOrder`). */
  visibleOrder?: number[];
  onVisibleOrderChange?: (value: number[]) => void;
  setThreads: (value: boolean) => void;
  search: string;
  loading: boolean;
  open: (id: number) => void;
  prefetch: (id: number) => void;
  action: (id: number, action: string) => void;
  loadMore: () => void;
  selectionScope: string;
  outputBusy: boolean;
  printEmails: (ids: number[]) => void;
  exportEmails: (ids: number[]) => void;
  threads: boolean;
  busyIds: Set<number>;
  searchError?: string;
  accounts?: Account[];
  /** The name known for each address, for the "to ..." beside a reply's sender. */
  people?: PersonIndex;
  /** Spam or Trash is being fetched from the server for this view (W3.1). */
  fetchingFolder?: boolean;
  /**
   * A right-click, Shift+F10 or the Menu key on a row: the native menu (lib/row-menu.ts) at `at`, in
   * CSS pixels of the page, for `rows`: every ticked row when the row is one of them, else the row.
   */
  rowMenu?: (id: number, at: { x: number; y: number }, rows: number[]) => void;
};

const order = ['Priority', 'New', 'Notifications', 'Newsletters', 'Pins', 'Seen'];
const noAccounts: Account[] = [];
const noPeople: PersonIndex = new Map();
// More rows than this arriving or leaving at once is a new list, not a change in it: no motion.
const BULK = 5;
// Past this many rows the list does not animate positions (each animated row is measured).
const LAYOUT_LIMIT = 200;

// A row comes in by opening to its height, on a critically damped spring (a height never
// overshoots below zero). A row that goes is lifted out of the flow at once (AnimatePresence
// `popLayout`): the rows under it close the gap straight away, as they always did, so a control the
// reader is reaching for is where it was at once, and the row fades out where it was. A whole list
// that is replaced (`instant`) goes without motion.
const openSpring = { type: 'spring', stiffness: 700, damping: 53, restDelta: 1 } as const;
const rowVariants: Variants = {
  enter: { height: 0, opacity: 0, overflow: 'hidden' },
  shown: {
    height: 'auto',
    opacity: 1,
    transition: { height: openSpring, opacity: { duration: 0.15 } },
    transitionEnd: { overflow: 'visible' },
  },
  gone: (exit: { instant: boolean } | undefined) =>
    exit?.instant
      ? { opacity: 0, transition: { duration: 0 } }
      : { opacity: 0, overflow: 'hidden', transition: { opacity: { duration: 0.16, ease: 'easeOut' } } },
};

// The time at the right of the first line. It is always the short form (the full one is its title):
// the arrow of a conversation sits just left of it, and the row knows how wide the time is in
// characters (--time-ch) so the arrow can find its place without measuring.
function whenText(email: EmailSummary) {
  return email.date_missing ? t('Data non disponibile', 'Date unavailable') : shortDate(email.date);
}
function rowDescription(email: EmailSummary) {
  const date = email.date_missing ? t('Data non disponibile', 'Date unavailable') : fullDate(email.date);
  const parts = [date];
  if (!email.read) parts.push(t('Non letto', 'Unread'));
  if (email.pinned) parts.push(t('Fissato', 'Pinned'));
  if (email.attachment_count)
    parts.push(
      `${email.attachment_count} ${email.attachment_count === 1 ? t('allegato', 'attachment') : t('allegati', 'attachments')}`,
    );
  return parts.join(', ');
}
// A conversation keeps its row when new mail makes another message its main one: the row is the
// conversation, not the message, and it changes in place instead of leaving and coming back.
const rowKey = (email: EmailSummary) => (email.conversation_id ? `c${email.conversation_id}` : `m${email.id}`);
function tabLabel(tab: string) {
  return tab === 'Smart' ? t('Intelligente', 'Smart') : tab === 'All' ? t('Tutti', 'All') : t('Non letti', 'Unread');
}
function marked(text: string, terms: string[]) {
  return markedParts(text, terms).map((part, i) => (part.match ? <mark key={i}>{part.text}</mark> : part.text));
}

// The heading of an Inbox group. It comes and goes with the rows: when its last row leaves, it
// closes the same way instead of jumping away after them.
function GroupHeading({
  title,
  count,
  unread,
  collapsed,
  toggle,
  moving,
  quiet,
}: {
  title: string;
  count: number;
  unread: boolean;
  collapsed: boolean;
  toggle: (title: string) => void;
  moving: boolean;
  quiet: boolean;
}) {
  const present = useIsPresent();
  const own = {
    className: cn('group-heading', unread && 'has-unread'),
    onClick: () => toggle(title),
    'aria-expanded': !collapsed,
    'aria-hidden': present ? undefined : true,
    inert: !present,
  };
  const content = (
    <>
      <span className={cn(title === 'Priority' && 'priority-label')}>
        {title === 'Priority' && <span className="priority-dot"></span>}
        {sectionLabel(title)}
      </span>
      <Count value={count} className="group-count" />
      <span className={cn(collapsed && 'collapsed')}>
        <Icon name="down" size={14} />
      </span>
    </>
  );
  return moving ? (
    <motion.button {...own} variants={rowVariants} initial={quiet ? false : 'enter'} animate="shown" exit="gone">
      {content}
    </motion.button>
  ) : (
    <button {...own}>{content}</button>
  );
}

type RowProps = {
  email: EmailSummary;
  /** Motion is on (no reduced motion): the row is a motion element. */
  moving: boolean;
  /** The row came with many others at once: it shows at once. */
  quiet: boolean;
  layoutKey: string | undefined;
  selected: number | null;
  checked: boolean;
  tabRow: number | null;
  threads: boolean;
  search: string;
  loading: boolean;
  outputBusy: boolean;
  busyIds: Set<number>;
  accounts: Account[];
  people: PersonIndex;
  self: Set<string>;
  view: Folder;
  check: (id: number) => void;
  dragRow: (event: DragEvent, item: EmailSummary) => void;
  open: (id: number) => void;
  prefetch: (id: number) => void;
  action: (id: number, action: string) => void;
  openRowMenu: (event: MouseEvent<HTMLElement>, item: EmailSummary) => void;
};

// One row of the list and, when its conversation is open, the replies under it. It stays plain
// markup (no Ark). While it animates out it is no longer selected, hidden from assistive
// technology and inert, so nothing finds or reaches a message that has left.
function MailRow({
  email,
  moving,
  quiet,
  layoutKey,
  selected,
  checked,
  tabRow,
  threads,
  search,
  loading,
  outputBusy,
  busyIds,
  accounts,
  people,
  self,
  view,
  check,
  dragRow,
  open,
  prefetch,
  action,
  openRowMenu,
}: RowProps) {
  useStore(threadState);
  const present = useIsPresent();
  const element = useRef<HTMLDivElement>(null);
  // A focused control in a row that is going away would keep the focus out of reach: let it go,
  // so the app puts it back on the list.
  useLayoutEffect(() => {
    if (present) return;
    const active = document.activeElement;
    if (active instanceof HTMLElement && element.current?.contains(active)) active.blur();
  }, [present]);

  const when = whenText(email);
  const isSelected = present && selected === email.id;
  const threadOpen = threads && isThreadOpen(email);
  const pinLabel = email.pinned
    ? threads
      ? t('Rimuovi fissaggio dalla conversazione', 'Unpin thread')
      : t('Rimuovi fissaggio dal messaggio', 'Unpin message')
    : threads
      ? t('Fissa conversazione', 'Pin thread')
      : t('Fissa messaggio', 'Pin message');
  const terms = search ? email.match_terms || [] : [];
  const attachments = `${email.attachment_count} ${email.attachment_count === 1 ? t('allegato', 'attachment') : t('allegati', 'attachments')}`;
  const tab = tabRow === email.id ? 0 : -1;

  // Right opens the conversation of the row under the focus, Left closes it.
  function toggleThread() {
    const key = conversationOf(email);
    threadState.open[key] = !threadState.open[key];
  }
  function threadKey(event: KeyboardEvent) {
    if (!threads || !email.replies?.length) return;
    if (event.key === 'ArrowRight' && !isThreadOpen(email)) toggleThread();
    else if (event.key === 'ArrowLeft' && isThreadOpen(email)) toggleThread();
    else return;
    event.preventDefault();
  }
  // The view is part of the question: the Trash and Spam views hold their own members.
  const readReplies = (offset: number): Promise<ConversationReply[]> =>
    call<ConversationReply[]>('read_replies', { mainId: email.id, folder: view, offset, limit: 30 });

  const rowProps = {
    ref: element,
    className: cn(
      'mail-row',
      isSelected && 'selected',
      !email.read && 'unread',
      threads && !!email.replies?.length && 'has-thread',
    ),
    style: { '--time-ch': when.length } as React.CSSProperties,
    'data-section': email.section.toLowerCase(),
    'data-row-id': email.id,
    'aria-hidden': present ? undefined : true,
    inert: !present,
    onContextMenu: (event: MouseEvent<HTMLElement>) => openRowMenu(event, email),
  };
  const content = (
    <>
      <label className="mail-check">
        <input
          className="mail-checkbox"
          type="checkbox"
          tabIndex={tab}
          aria-label={`${t('Seleziona', 'Select')} ${email.subject || t('(Senza oggetto)', '(No subject)')}`}
          checked={checked}
          disabled={outputBusy || loading}
          onChange={() => check(email.id)}
        />
      </label>{' '}
      <span className="visually-hidden" id={`mail-row-${email.id}`}>
        {rowDescription(email)}
      </span>{' '}
      <button
        className="mail-open"
        tabIndex={tab}
        draggable={!outputBusy && !loading}
        onDragStart={(event) => dragRow(event, email)}
        onClick={() => open(email.id)}
        onPointerEnter={() => prefetch(email.id)}
        onFocus={() => prefetch(email.id)}
        onKeyDown={threadKey}
        aria-describedby={`mail-row-${email.id}`}
        aria-pressed={selected === email.id}
      >
        <span className="visually-hidden">{t('Apri', 'Open')}</span>{' '}
        <span className="mail-row-content">
          <span className="mail-meta">
            <strong>{email.from_name || email.from_addr}</strong>
            {!!email.attachment_count && (
              <span className="attachment-count" title={attachments}>
                <Icon name="clip" size={14} />
                {email.attachment_count > 1 && email.attachment_count}
              </span>
            )}
            <time title={email.date_missing ? t('Data non disponibile', 'Date unavailable') : fullDate(email.date)}>
              {when}
            </time>
          </span>
          <span className="mail-line">
            <span className="mail-subject">
              <span className="mail-subject-text">{marked(email.subject || t('(Senza oggetto)', '(No subject)'), terms)}</span>
              {threads && email.message_ids.length > 1 && !email.replies?.length && (
                <span
                  className="thread-count"
                  title={`${email.message_ids.length} ${t('messaggi corrispondenti nella conversazione', 'matching messages in this thread')}`}
                >
                  {email.message_ids.length}
                </span>
              )}
            </span>
            <span className="mail-preview">
              <span className="mail-dash" aria-hidden="true">
                —
              </span>
              <span className="mail-snippet">{marked(fixedBackendText(email.snippet), terms)}</span>
              {!!email.labels.length && (
                <span className="row-labels">
                  {email.labels.slice(0, 2).map((label) => (
                    <span key={label} title={label}>
                      {label}
                    </span>
                  ))}
                </span>
              )}
            </span>
          </span>
          {!!search && (
            <span className="search-match-context">
              {accounts.find((account) => account.id === email.account_id)?.email || ''}
              {email.folder ? ` · ${folderLabel(email.folder)}` : ''}
              {email.match_reason && <small>{backendText(email.match_reason)}</small>}
            </span>
          )}
        </span>{' '}
        {!email.read && <span className="unread-dot" aria-hidden="true"></span>}
      </button>{' '}
      <button
        className={cn('row-pin', email.pinned && 'pinned')}
        tabIndex={tab}
        title={pinLabel}
        aria-label={pinLabel}
        disabled={email.message_ids.some((id) => busyIds.has(id))}
        onClick={() => action(email.id, email.pinned ? 'unpin' : 'pin')}
      >
        <Icon name="pin" size={14} />
      </button>{' '}
      {threads && !!email.replies?.length && (
        // The total of messages in the conversation. Closed, it turns blue when one of the
        // replies it hides is unread.
        <button
          className={cn('thread-toggle', !isThreadOpen(email) && hasUnreadReplies(email) && 'has-unread')}
          tabIndex={tab}
          aria-expanded={isThreadOpen(email)}
          aria-label={t('Conversazione', 'Conversation')}
          title={`${1 + email.replies.length + (email.replies_more ?? 0)} ${t('messaggi nella conversazione', 'messages in this thread')}${!isThreadOpen(email) && hasUnreadReplies(email) ? t(', con messaggi non letti', ', with unread messages') : ''}`}
          onClick={toggleThread}
        >
          <Icon name="down" size={14} />
          <span className="thread-total">{1 + email.replies.length + (email.replies_more ?? 0)}</span>
        </button>
      )}
    </>
  );

  return (
    <>
      {moving ? (
        <motion.div
          {...rowProps}
          variants={rowVariants}
          initial={quiet ? false : 'enter'}
          animate="shown"
          exit="gone"
          layout={layoutKey !== undefined ? 'position' : undefined}
          layoutDependency={layoutKey}
        >
          {content}
        </motion.div>
      ) : (
        <div {...rowProps}>{content}</div>
      )}
      {present && threadOpen && (
        <ThreadReplies
          row={email}
          index={people}
          self={self}
          selectedId={selected}
          tabId={tabRow}
          open={open}
          more={readReplies}
        />
      )}
    </>
  );
}

export default function MailList({
  page,
  view,
  selected,
  sectionLocks,
  onVisibleOrderChange,
  setThreads,
  search,
  loading,
  open,
  prefetch,
  action,
  loadMore,
  selectionScope,
  outputBusy,
  printEmails,
  exportEmails,
  threads,
  busyIds,
  searchError = '',
  accounts = noAccounts,
  people = noPeople,
  fetchingFolder = false,
  rowMenu,
}: MailListProps) {
  useStore(groupState);
  useStore(threadState);
  const still = useReducedMotion();
  // The reader's own addresses: one person, named "you".
  const self = new Set(accounts.map((account) => account.email.trim().toLowerCase()));
  const [mode, setMode] = useState('Smart');
  const [checked, setChecked] = useState<number[]>([]);
  useEffect(() => {
    setChecked([]);
  }, [selectionScope, mode]);
  const stableItems = page.items.map((e) =>
    sectionLocks[e.id] && !e.pinned ? { ...e, section: sectionLocks[e.id] } : e,
  );
  const items =
    view === 'Inbox' && !search && mode === 'Unread'
      ? stableItems.filter((e) => !e.read || sectionLocks[e.id])
      : stableItems;
  const checkedItems = checked.filter((id) => items.some((e) => e.id === id));
  const allChecked = items.length > 0 && checkedItems.length === items.length;
  const checkedMessageIds = items.filter((item) => checkedItems.includes(item.id)).flatMap((item) => item.message_ids);
  function dragRow(event: DragEvent, item: EmailSummary) {
    // Dragging a checked row carries the whole selection, like the selection actions.
    const ids = checkedItems.includes(item.id) ? [...checkedMessageIds] : [...item.message_ids];
    startMailDrag(
      event.nativeEvent,
      ids,
      ids.length === 1 ? t('1 email', '1 email') : t(`${ids.length} email`, `${ids.length} emails`),
    );
  }
  // Drop the ticks of rows that left the list. An updater, so that it works on the latest list: the
  // reset above (a new scope or tab) runs in the same commit, and a stale `checkedItems` written after
  // it brought the old selection back.
  useEffect(() => {
    if (checked.length === checkedItems.length) return;
    setChecked((list) => {
      const kept = list.filter((id) => items.some((e) => e.id === id));
      return kept.length === list.length ? list : kept;
    });
  });
  function check(id: number) {
    setChecked((list) => (list.includes(id) ? list.filter((value) => value !== id) : [...list, id]));
  }
  // The native menu of a row (docs/aspetto-nativo.md, rule 33). The row takes the event, so neither
  // WebView2's menu nor the system menu opens on it. Shift+F10 and the Menu key send the same event
  // without a button: Chromium says `pointerType` 'mouse' and `button` -1 (the specification, an
  // empty `pointerType`). The menu then opens below the row, 16 px in, not at the pointer.
  function openRowMenu(event: MouseEvent<HTMLElement>, item: EmailSummary) {
    if (!rowMenu) return;
    event.preventDefault();
    const pointer = event.nativeEvent as PointerEvent;
    const fromKeyboard = !pointer.pointerType || pointer.button < 0;
    const box = event.currentTarget.getBoundingClientRect();
    const at = fromKeyboard ? { x: box.left + 16, y: box.bottom } : { x: event.clientX, y: event.clientY };
    rowMenu(item.id, at, checkedItems.includes(item.id) ? [...checkedItems] : [item.id]);
  }
  const grouped = view === 'Inbox' && mode === 'Smart' && !search;
  const groups = grouped
    ? order.map((title) => ({ title, items: items.filter((e) => e.section === title) })).filter((g) => g.items.length)
    : [{ title: search ? 'Search results' : 'Messages', items }];
  const shownItems = groups.flatMap((g) => (groupState.collapsed.includes(g.title) ? [] : g.items));
  // What the reader needs, not how much mail there is. Only the Inbox carries the counts.
  const headline =
    !search && page.unread_total != null ? { unread: page.unread_total, priority: page.priority_total ?? 0 } : null;
  // The rows in the order the arrow keys walk them: each row, then the replies on screen under it
  // (an open conversation, minus what a folded reply hides).
  const rowOrder = shownItems.flatMap((item) => [
    item.id,
    ...(threads && isThreadOpen(item)
      ? visibleReplies(repliesOf(item), threadState.folded[conversationOf(item)] ?? []).map((shown) => shown.reply.id)
      : []),
  ]);
  const rowOrderKey = rowOrder.join(',');
  const orderChange = useRef(onVisibleOrderChange);
  orderChange.current = onVisibleOrderChange;
  // Told during the commit, before the browser paints: the app reads the order right after a tick.
  useLayoutEffect(() => {
    orderChange.current?.(rowOrderKey ? rowOrderKey.split(',').map(Number) : []);
  }, [rowOrderKey]);
  // The list is one Tab stop: only the current row's controls join the tab order, and the
  // arrow keys move between rows.
  const tabRow = selected !== null && rowOrder.includes(selected) ? selected : (rowOrder[0] ?? null);
  useEffect(() => {
    if (selected === null) return;
    const row = document.querySelector('.mail-row.selected, .reply-row.selected');
    if (!row) return;
    // Only a row out of sight is scrolled to: Chromium also moves the Tab starting point to the
    // element scrolled into view, which must not happen for a row that is already on screen.
    const area = row.closest('.mail-scroll')?.getBoundingClientRect();
    const box = row.getBoundingClientRect();
    if (area && box.top >= area.top && box.bottom <= area.bottom) return;
    row.scrollIntoView({ block: 'nearest' });
  }, [selected]);
  function toggle(title: string) {
    groupState.collapsed = groupState.collapsed.includes(title)
      ? groupState.collapsed.filter((x) => x !== title)
      : [...groupState.collapsed, title];
  }
  // What "Show more" loaded follows the list: see reconcileThreads.
  useEffect(() => {
    reconcileThreads(page.items);
  }, [page.items]);

  // Which rows arrived and left since the last commit: a few are new mail or mail sent away and
  // move; many at once are a new list (another folder, a search, "Load more") and do not.
  const shownIds = shownItems.map(rowKey);
  const committed = useRef<Set<string>>(new Set());
  const before = committed.current;
  const now = new Set(shownIds);
  const arrived = shownIds.filter((id) => !before.has(id)).length;
  let left = 0;
  for (const id of before) if (!now.has(id)) left += 1;
  const quietArrival = before.size === 0 || arrived > BULK;
  // A group the reader opens or closes shows or hides its rows at once: that is not mail moving.
  const collapsedBefore = useRef(groupState.collapsed);
  const opened = new Set(collapsedBefore.current.filter((title) => !groupState.collapsed.includes(title)));
  const closed = groupState.collapsed.some((title) => !collapsedBefore.current.includes(title));
  // Rows still on their way out read this again on later renders: it keeps the last removal's answer.
  const lastExit = useRef(false);
  const instantExit = left > 0 ? closed || left > BULK || left === before.size : lastExit.current;
  useEffect(() => {
    committed.current = now;
    collapsedBefore.current = groupState.collapsed;
    lastExit.current = instantExit;
  });
  const moving = !still;
  // Rows that move (a pinned row that changes group, new mail above) slide to their place. A
  // removal is left out: the rows close the gap at once (see rowVariants).
  const lastLayout = useRef('');
  const layoutKey =
    moving && shownItems.length <= LAYOUT_LIMIT ? (left > 0 ? lastLayout.current : shownIds.join(',')) : undefined;
  useEffect(() => {
    lastLayout.current = layoutKey ?? '';
  });

  const rowProps = {
    moving,
    quiet: quietArrival,
    layoutKey,
    selected,
    tabRow,
    threads,
    search,
    loading,
    outputBusy,
    busyIds,
    accounts,
    people,
    self,
    view,
    check,
    dragRow,
    open,
    prefetch,
    action,
    openRowMenu,
  };
  const rows = groups.flatMap((group) => [
    ...(grouped
      ? [
          <GroupHeading
            key={`group:${group.title}`}
            title={group.title}
            count={group.items.length}
            unread={group.items.some((e) => !e.read)}
            collapsed={groupState.collapsed.includes(group.title)}
            toggle={toggle}
            moving={moving}
            quiet={quietArrival}
          />,
        ]
      : []),
    ...(groupState.collapsed.includes(group.title)
      ? []
      : group.items.map((email) => (
          <MailRow
            key={rowKey(email)}
            email={email}
            checked={checkedItems.includes(email.id)}
            {...rowProps}
            quiet={quietArrival || opened.has(group.title)}
          />
        ))),
  ]);

  const word = threads
    ? page.total === 1
      ? t('conversazione', 'thread')
      : t('conversazioni', 'threads')
    : page.total === 1
      ? t('messaggio', 'message')
      : t('messaggi', 'messages');
  const available =
    search && page.search?.coverage !== 'complete'
      ? page.total === 1
        ? t(' disponibile', ' available')
        : t(' disponibili', ' available')
      : '';

  // «All caught up» is the news of the views that ask for work (the inbox, the to-do list); elsewhere an empty view
  // simply holds no messages.
  const caughtUp = view === 'Inbox' || view === 'Tasks';
  return (
    <section
      className="mail-list"
      aria-label={threads ? t('Elenco conversazioni', 'Thread list') : t('Elenco messaggi', 'Message list')}
      aria-busy={loading}
    >
      <div className="list-heading">
        <div className="list-title">
          <h1 tabIndex={-1}>{search ? t('Risultati di ricerca', 'Search results') : folderLabel(view)}</h1>{' '}
          <span className="muted" aria-live="polite">
            {headline ? (
              headline.unread === 0 ? (
                t('Tutto letto', 'All read')
              ) : (
                <>
                  {headline.priority > 0 && (
                    <>
                      <b className="glance-priority">
                        {headline.priority} {t('priorità', 'priority')}
                      </b>
                      {' · '}
                    </>
                  )}
                  <b>{headline.unread}</b> {t('da leggere', 'to read')}
                </>
              )
            ) : (
              `${search ? `${page.items.length} / ` : ''}${page.total} ${word}${available}`
            )}
          </span>
        </div>{' '}
        <div className="mail-display" role="group" aria-label={t('Visualizzazione della posta', 'Mail display')}>
          <Button
            variant="icon"
            className={cn(!threads && 'active')}
            aria-label={t('Email', 'Emails')}
            title={t('Mostra le singole email', 'Show individual emails')}
            aria-pressed={!threads}
            onClick={() => setThreads(false)}
          >
            <Icon name="mail" size={17} />
          </Button>
          <Button
            variant="icon"
            className={cn(threads && 'active')}
            aria-label={t('Conversazioni', 'Threads')}
            title={t('Raggruppa le email in conversazioni', 'Group emails into threads')}
            aria-pressed={threads}
            onClick={() => setThreads(true)}
          >
            <Icon name="threads" size={17} />
          </Button>
        </div>
      </div>{' '}
      {fetchingFolder && !search && (
        <p className="list-fetching" role="status">
          {t(
            'Sto recuperando i messaggi di questa cartella dal server',
            'Fetching the messages of this folder from the server',
          )}
        </p>
      )}
      {view === 'Inbox' && !search && (
        <div className="mail-tabs">
          <Tabs
            value={mode}
            onChange={setMode}
            aria-label={t('Vista di Posta in arrivo', 'Inbox view')}
            items={['Smart', 'All', 'Unread'].map((tab) => ({ value: tab, label: tabLabel(tab) }))}
          />
        </div>
      )}
      {/* The bar waits for the first tick, so a list nobody is selecting from stays quiet. */}
      {!!checkedItems.length && (
        <div className="selection-toolbar">
          <Checkbox
            aria-label={t('Seleziona i messaggi caricati', 'Select loaded messages')}
            checked={allChecked}
            indeterminate={checkedItems.length > 0 && !allChecked}
            disabled={loading || !items.length || outputBusy}
            onChange={() => setChecked(allChecked ? [] : items.map((e) => e.id))}
          >
            {`${checkedItems.length} ${checkedItems.length === 1 ? t('selezionato', 'selected') : t('selezionati', 'selected')}${threads ? ` · ${checkedMessageIds.length} ${checkedMessageIds.length === 1 ? 'email' : t('email', 'emails')}` : ''}`}
          </Checkbox>{' '}
          <Button variant="text" disabled={outputBusy} onClick={() => setChecked([])}>
            {t('Cancella', 'Clear')}
          </Button>{' '}
          <div className="selection-actions">
            <Button
              variant="secondary"
              disabled={!checkedItems.length || outputBusy || loading}
              onClick={() => printEmails([...checkedMessageIds])}
            >
              <Icon name="print" size={15} />
              {t('Stampa selezionati', 'Print selected')}
            </Button>{' '}
            <Button
              variant="secondary"
              disabled={!checkedItems.length || outputBusy || loading}
              onClick={() => exportEmails([...checkedMessageIds])}
            >
              <Icon name="download" size={15} />
              {t('Scarica .mbox', 'Download .mbox')}
            </Button>
          </div>{' '}
          {outputBusy && <span role="status">{t('Preparazione messaggi…', 'Preparing messages…')}</span>}
        </div>
      )}
      {/* Positioned, so that a row on its way out stays in place inside the scrolled list. */}
      <div className="mail-scroll" style={moving ? { position: 'relative' } : undefined}>
        {loading && !page.items.length && (
          <div className="loading-lines">
            <span></span>
            <span></span>
            <span></span>
          </div>
        )}
        {moving ? (
          // A change of view, account, mode or display is a new list: it starts over, without motion.
          <AnimatePresence
            key={`${selectionScope}:${mode}`}
            initial={false}
            mode="popLayout"
            custom={{ instant: instantExit }}
          >
            {rows}
          </AnimatePresence>
        ) : (
          rows
        )}
        {!loading && !items.length && (
          <div className="empty-list">
            <Icon name={search ? 'search' : caughtUp ? 'check' : 'mail'} size={30} />
            <h3>
              {searchError
                ? t('Ricerca da correggere', 'Search needs correction')
                : search
                  ? page.search?.coverage !== 'complete'
                    ? t('Nessun risultato nella posta disponibile', 'No results in available mail')
                    : t('Nessuna email corrisponde ai filtri', 'No email matches these filters')
                  : caughtUp
                    ? t('Sei in pari', 'You’re all caught up')
                    : t('Nessun messaggio', 'No messages')}
            </h3>
            <p>
              {search
                ? searchError
                  ? t('Correggi il filtro indicato sopra.', 'Correct the filter shown above.')
                  : t('Premi “Non la trovo” per provare una modifica.', 'Choose “I can’t find it” to try one change.')
                : t('Qui compariranno i messaggi di questa vista.', 'Messages for this view will appear here.')}
            </p>
          </div>
        )}
        {page.next_offset !== null && (
          <button className="load-more" onClick={loadMore} disabled={loading}>
            {t('Mostra altri messaggi', 'Load more messages')}
          </button>
        )}
      </div>
    </section>
  );
}
