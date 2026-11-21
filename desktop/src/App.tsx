import { Suspense, lazy, useEffect, useLayoutEffect, useReducer, useRef, useState } from 'react';
import { useReducedMotion } from 'motion/react';
import Button from './ds/primitives/Button';
import Toaster, { notify as showNotice, type NoticeAction } from './ds/primitives/Toaster';
import { listen } from '@tauri-apps/api/event';
import Sidebar from './components/Sidebar';
import WindowControls from './components/WindowControls';
import MailList from './components/MailList';
import { imeComposing } from './ds/ime';
import {
  arrowsMoveMail,
  isAppChord,
  shortcutCoveredByLayer,
  shortcutIgnored,
  zoomKey,
} from './lib/shortcut-guard';
import { syncErrors, type MailSyncStatus } from './lib/mail-sync';
import SearchBar from './components/SearchBar';
import Reader from './components/Reader';
import PrintPreview from './components/PrintPreview';
import HistoryUnreadDialog from './components/HistoryUnreadDialog';
import Onboarding from './components/Onboarding';
import LegacyLocalNotice from './components/LegacyLocalNotice';
import type { AppEdition } from './components/ComputeSettings';
import Settings, { type SettingsHandle } from './components/Settings';
import UnsavedDialog from './components/UnsavedDialog';
import Assistant from './components/Assistant';
import Calendar from './components/Calendar';
import Contacts from './components/Contacts';
import Meetings from './components/Meetings';
import MeetingRecordingIndicator from './components/meetings/MeetingRecordingIndicator';
import { captureActive, flushMeetingDrafts, meetingCall, requestMeetingOpen, type CaptureSnapshot } from './lib/meetings';
import { installWindowCloseHandler, registerCloseGuard } from './lib/window-close';
import { paintWindowBackground } from './lib/native-window';
import { installWindowShell, windowAppearance, zoomWindow } from './lib/window-shell';
import Icon from './ds/primitives/Icon';
import UndoStack from './components/UndoStack';
import { refreshActivity } from './lib/activity-center';
import ActivityPage from './components/ActivityPage';
import { applyActivityEvent, type Activity } from './lib/activity';
import { call, desktop, errorText, shortDate } from './lib/api';
import { syncGoogleCalendars } from './lib/calendar';
import { readPreference, savePreference } from './lib/preferences';
import { appearancePreference, resolvedAppearance, saveAppearance, systemPrefersDark, type Appearance } from './lib/appearance';
import { configureRegional, configureRegionalWording, regionalLocale, regionalTimeZone } from './lib/regional';
import { backendText, folderLabel, isItalian, setLanguageLocale, tr, useLanguage } from './lib/i18n';
import { personIndex, type PersonIndex } from './lib/people';
import { replyAllRecipients } from './lib/reply-all';
import { installMailtoRouting, mailtoDraft, type MailtoFields } from './lib/mailto';
import { savedFileActions } from './lib/saved-file';
import { rowMenuAction, rowMenuEntries, showRowMenu } from './lib/row-menu';
import { patchExtraRead, repliesOf, threadState } from './lib/groups';
import { useStore } from './lib/store';
import { tick } from './lib/tick';
import { cn } from './lib/utils';
import {
  applyEvent,
  applyRows,
  carriesFlags,
  coalescing,
  parseAppEvent,
  readRowsMax,
  sameFilter,
} from './lib/events';
import type {
  Draft,
  Email,
  EmailSummary,
  Folder,
  ListFilter,
  MailPage,
  MailRow,
  SyncPhase,
  Workspace,
  UndoAction,
} from './lib/types';

// The composer dock is only needed once a draft is open: it is loaded on demand.
const ComposerDock = lazy(() => import('./components/ComposerDock'));

export type AppProps = Record<string, never>;

/** What "Undo" in the search bar puts back: the list, the reader and the scroll as they were. */
interface SearchUndo {
  query: string;
  page: MailPage;
  sort: string;
  accountId: number | null;
  selected: Email | null;
  threadEmails: Email[];
  scroll: number;
}

/** The root's state, the role of App.svelte's `$state` variables. */
interface AppState {
  workspace: Workspace | null;
  appearance: Appearance;
  systemDark: boolean;
  view: Folder;
  accountId: number | null;
  page: MailPage;
  selected: Email | null;
  openingId: number | null;
  threadEmails: Email[];
  // The name known for each address, for the reader. Replaced whole, never edited in place.
  people: PersonIndex;
  threads: boolean;
  sidebarCollapsed: boolean;
  // Rows opened in this view keep their group until the user leaves or syncs.
  sectionLocks: Record<number, string>;
  visibleOrder: number[];
  settingsDirty: boolean;
  leaveTarget: Folder | 'close' | null;
  composers: Draft[];
  activeComposerId: number | null;
  closingWindow: boolean;
  searchInput: string;
  search: string;
  displayedQuery: string;
  searchError: string;
  searchSort: string;
  remoteBusy: boolean;
  /** Where the server search stopped, for «Altri risultati dal server»; only for `remoteQuery`. */
  remoteNext: unknown;
  remoteQuery: string;
  remoteAccount: number | null;
  searchUndo: SearchUndo | null;
  loading: boolean;
  // Messages whose triage action the backend has not accepted yet (INV-006). Only the controls
  // of these messages are disabled; the set is replaced, never mutated, so views update.
  busyIds: Set<number>;
  manualSyncing: boolean;
  manualSyncError: string;
  syncStatus: MailSyncStatus;
  // The single activity registry (ADR-0007 decision 7, INV-009): fed only by the typed
  // events, rendered by the ActivityCenter in the sidebar footer.
  backendActivities: Activity[];
  outputBusy: boolean;
  printPreview: { emails: Email[]; title: string } | null;
  /** The history brought many unread messages into an account's Inbox (EX-9): asked once. */
  historyUnread: { accountId: number; count: number } | null;
  /** Accounts whose Spam or Trash, open now, is being fetched from the server (W3.1). */
  fetchingFolder: number[];
  /** The Professional build's first start (spec A8): shown before the main window. */
  onboarding: AppEdition | null;
  fatal: string;
  // A message opened as shown-only stays so until the view changes (see shownOnly).
  openedShownOnly: Set<number>;
}

/**
 * Always-current state for the root: `state.x` reads the latest value also after an `await`, in a
 * timer or a listener, and `state.x = y` re-renders. It is shallow on purpose: every value is
 * replaced, never edited in place, so the children get plain objects (a deep proxy would reach
 * them, and `structuredClone` of a proxy throws).
 */
function useLiveState<T extends object>(init: () => T): T {
  const [, rerender] = useReducer((n: number) => n + 1, 0);
  const [state] = useState(() => {
    const target = init();
    return new Proxy(target, {
      set(object, key, value) {
        if (Reflect.get(object, key) !== value) {
          Reflect.set(object, key, value);
          rerender();
        }
        return true;
      },
    });
  });
  return state;
}

const emptyPage = (): MailPage => ({ items: [], total: 0, next_offset: null });
const special = new Set(['Settings', 'Calendar', 'Contacts', 'Meetings', 'Assistant', 'Drafts', 'Activity']);
// The backend answers at most 100 rows per list_mail call (INV-007, mail.rs clamps the limit).
const mailCallMax = 100;
const leavesList = ['archive', 'moveToTrash', 'markAsDone', 'snooze'];

export default function App() {
  // The whole tree renders under the root: a language change re-renders every translated word.
  useLanguage();
  // `selectedInView` reads the replies loaded with "Show more".
  useStore(threadState);

  const s = useLiveState<AppState>(() => ({
    workspace: null,
    appearance: appearancePreference(),
    systemDark: systemPrefersDark(),
    view: 'Inbox',
    accountId: null,
    page: emptyPage(),
    selected: null,
    openingId: null,
    threadEmails: [],
    people: new Map(),
    threads: readPreference('mail-display') === 'threads',
    sidebarCollapsed: readPreference('sidebar-collapsed') === 'true',
    sectionLocks: {},
    visibleOrder: [],
    settingsDirty: false,
    leaveTarget: null,
    composers: [],
    activeComposerId: null,
    closingWindow: false,
    searchInput: '',
    search: '',
    displayedQuery: '',
    searchError: '',
    searchSort: '',
    remoteBusy: false,
    remoteNext: null,
    remoteQuery: '',
    remoteAccount: null,
    searchUndo: null,
    loading: false,
    busyIds: new Set(),
    manualSyncing: false,
    manualSyncError: '',
    syncStatus: { revision: 0, accounts: [] },
    backendActivities: [],
    outputBusy: false,
    printPreview: null,
    historyUnread: null,
    fetchingFolder: [],
    onboarding: null,
    fatal: '',
    openedShownOnly: new Set(),
  }));

  // Plain variables of the instance: they never show on screen, so changing them renders nothing.
  const [v] = useState(() => {
    configureRegionalWording(tr);
    return {
      // Account picked while unsaved settings held the user back; applied once they leave.
      leaveAccount: null as { id: number | null } | null,
      reportedSyncError: '',
      // Accounts whose typed `sync-progress` the UI has seen: a snapshot pulled at startup never
      // overrides them (see seedSyncActivities).
      syncEventAccounts: new Set<number>(),
      listRequest: 0,
      workspaceRequest: 0,
      readRequest: 0,
      rereadSeq: 0,
      // Messages shown as read before the backend has said so (INV-006): opening one marks it at once.
      // The value tells whether the Inbox counter was lowered for it, so that taking the mark back
      // puts back exactly that. While a mark waits, a snapshot of the backend, which still has the
      // message unread, must not undo it; the mark goes when the backend has answered.
      shownRead: new Map<number, boolean>(),
      // The open() that last marked a message answers for its mark: an older click for the same
      // message giving up later must not take it back.
      markOwner: new Map<number, number>(),
      // Marks made at the click, before read_mail has answered: id -> the open() that made it. When
      // the read is superseded they are taken back at once, so rows passed over while scanning do not
      // look read and the Inbox counter does not dip once per row.
      earlyMarks: new Map<number, number>(),
      // Workspace snapshots in flight: a patch from an event makes them stale (see onAppEvent).
      snapshotsRunning: 0,
      // The control that had the focus when each composer opened (by draft id): the place to go back to
      // when the composer closes and nothing in the list is selected.
      composerOpeners: new Map<number, HTMLElement>(),
      // The last workspace reload that a composer asked for (it saved or queued something): the lists it
      // rebuilds settle when this is done.
      workspaceSettling: Promise.resolve(),
      searchTimer: undefined as ReturnType<typeof setTimeout> | undefined,
      prefetchTimer: undefined as ReturnType<typeof setTimeout> | undefined,
      // Rows the events named, read together (see wantRows). `rowsLatest` remembers which read asked
      // for an id last, so an older answer never overwrites a newer one.
      pendingRows: new Set<number>(),
      rowsLatest: new Map<number, number>(),
      rowsTimer: undefined as ReturnType<typeof setTimeout> | undefined,
      rowsRequest: 0,
      refresh: null as null | (() => Promise<void>),
      refreshWorkspace: null as null | (() => Promise<void>),
    };
  });

  const settingsPanel = useRef<SettingsHandle>(null);
  const searchElement = useRef<HTMLInputElement>(null);
  const content = useRef<HTMLDivElement>(null);
  const reduceMotion = useReducedMotion();

  // The page has drawn: without this, src-tauri/src/recovery.rs offers to reload after 15 s. App's
  // first effect, not a frame after render() in main.tsx: a render that throws never commits, so an
  // empty window never passes for a ready one.
  useEffect(() => {
    if (desktop) void call('frontend_ready').catch(() => {});
  }, []);

  // Every function below reads the state through `s` and `v`, never a value of one render, so a
  // listener or timer that holds an older copy of a function still sees the current state.
  const dark = () => resolvedAppearance(s.appearance, s.systemDark);
  // The retained sync errors (pulled once per `sync-progress` that ends, see onSyncProgress): the
  // toolbar mark and the alert. The sync phase itself lives only in the indicator.
  const syncErrorList = () => syncErrors(s.workspace?.accounts || [], s.syncStatus, s.manualSyncError);
  const syncing = () => s.manualSyncing || s.backendActivities.some((activity) => activity.kind === 'sync');
  const isMailView = () => !special.has(s.view);

  async function loadPeople() {
    try {
      s.people = personIndex(
        await call<{ name: string; address: string }[]>('search_suggestions', { accountId: null }),
      );
    } catch {
      // Names are a convenience: without them the reader shows addresses.
    }
  }
  // A newer read supersedes the ones still waiting: returns the number of the new one.
  function supersedeReads() {
    for (const id of v.earlyMarks.keys()) {
      v.markOwner.delete(id);
      markReadLocally(id, false);
    }
    v.earlyMarks.clear();
    return ++v.readRequest;
  }
  // The list row that holds the message on screen, whatever is being opened next.
  function rowOfSelected() {
    if (!s.selected) return null;
    const current = s.selected;
    return (
      s.page.items.find(
        (item) =>
          item.message_ids.includes(current.id) ||
          (s.threads && item.message_ids.some((id) => s.threadEmails.some((email) => email.id === id))),
      )?.id ?? current.id
    );
  }
  // A message the list shows by itself (a row, or a reply under an open conversation) is the one
  // selected there; any other stands in its row.
  function selectedListId() {
    const openingId = s.openingId;
    return openingId !== null
      ? s.visibleOrder.includes(openingId)
        ? openingId
        : (s.page.items.find((item) => item.message_ids.includes(openingId))?.id ?? openingId)
      : s.selected && s.visibleOrder.includes(s.selected.id)
        ? s.selected.id
        : rowOfSelected();
  }
  // Another message is being read while the reader still shows the previous one.
  const openingOther = () => s.openingId !== null && s.openingId !== s.selected?.id;
  function prefetch(id?: number) {
    clearTimeout(v.prefetchTimer);
    const ids = isMailView()
      ? [...new Set([...(id === undefined ? [] : [id]), ...s.visibleOrder])].slice(0, 5)
      : [];
    v.prefetchTimer = setTimeout(() => {
      void call('prefetch_mail', { ids }).catch(() => {});
    }, 120);
  }
  function notify(message: string, error = false, actions: NoticeAction[] = []) {
    showNotice(message, error, actions);
  }
  async function printEmails(ids: number[], thread = false) {
    if (s.outputBusy) return;
    s.outputBusy = true;
    try {
      const emails = await call<Email[]>('prepare_print', { ids, thread });
      if (!emails.length) throw new Error(tr('Nessun messaggio da stampare', 'No messages to print'));
      s.printPreview = {
        emails,
        title: thread
          ? tr('Conversazione completa', 'Complete conversation')
          : emails.length === 1
            ? emails[0].subject
            : tr('Messaggi selezionati', 'Selected messages'),
      };
    } catch (error) {
      notify(errorText(error), true);
    } finally {
      s.outputBusy = false;
    }
  }
  async function exportEmails(ids: number[], format: 'eml' | 'mbox') {
    if (s.outputBusy) return;
    s.outputBusy = true;
    try {
      // The browser preview downloads instead of saving a file: it has no token and no actions.
      const saved = await call<{ count: number; path: string; token?: number } | null>('export_mail', {
        italian: isItalian(),
        ids,
        format,
      });
      if (saved)
        notify(
          saved.count === 1
            ? tr(`1 messaggio esportato: ${saved.path}`, `1 message exported: ${saved.path}`)
            : tr(
                `${saved.count} messaggi esportati: ${saved.path}`,
                `${saved.count} messages exported: ${saved.path}`,
              ),
          false,
          savedFileActions(saved.token, notify),
        );
    } catch (error) {
      notify(errorText(error), true);
    } finally {
      s.outputBusy = false;
    }
  }
  async function loadWorkspace() {
    const request = ++v.workspaceRequest;
    v.snapshotsRunning++;
    try {
      let result = await call<Workspace>('workspace_snapshot');
      if (request === v.workspaceRequest) {
        configureRegional(result.settings, result.regional);
        setLanguageLocale(regionalLocale());
        // The local counter already accounts for the marks waiting for the backend.
        if (s.workspace && [...v.shownRead.values()].some(Boolean))
          result = { ...result, counts: { ...result.counts, Inbox: s.workspace.counts.Inbox } };
        s.workspace = result;
      }
    } finally {
      v.snapshotsRunning--;
    }
  }
  // Re-reads the first `wanted` rows as sequential calls of at most mailCallMax rows. Returns
  // null if `superseded()` turns true while waiting, so the caller keeps the page it has.
  async function listLoaded(
    query: Record<string, unknown>,
    wanted: number,
    superseded: () => boolean,
  ): Promise<MailPage | null> {
    const seen = new Set<number>();
    const items: MailPage['items'] = [];
    let last: MailPage | null = null;
    for (let offset = 0; offset < wanted; offset += mailCallMax) {
      last = await call<MailPage>('list_mail', {
        query: { ...query, offset, limit: Math.min(mailCallMax, wanted - offset) },
      });
      if (superseded()) return null;
      // The list can shift between two calls; a row seen twice is kept once, at its first place.
      for (const item of last.items)
        if (!seen.has(item.id)) {
          seen.add(item.id);
          items.push(item);
        }
      if (last.next_offset === null) break;
    }
    return last && { ...last, items };
  }
  // keepLoaded: a background refresh re-reads every page already on screen, so it neither
  // drops the rows loaded with "Load more" nor moves the list under the reader.
  async function loadMail(append = false, keepLoaded = false) {
    if (!isMailView()) return;
    const request = ++v.listRequest;
    s.loading = true;
    const pageSize = s.search ? 20 : 60;
    const search = s.search;
    const query = { folder: s.view, account_id: s.accountId, search, threads: s.threads, sort: s.searchSort };
    try {
      const result =
        keepLoaded && !append
          ? await listLoaded(
              query,
              // next_offset counts the rows already requested, even when a refresh dropped some.
              Math.max(pageSize, s.page.next_offset ?? s.page.items.length),
              () => request !== v.listRequest,
            )
          : await call<MailPage>('list_mail', {
              query: { ...query, offset: append ? s.page.next_offset || 0 : 0, limit: pageSize },
            });
      if (!result || request !== v.listRequest) return;
      const shown = { ...result, items: result.items.map(rowShownAsRead) };
      s.page = append ? { ...shown, items: [...s.page.items, ...shown.items] } : shown;
      const selected = s.selected;
      if (
        !append &&
        s.search !== s.displayedQuery &&
        selected &&
        !result.items.some((item) => item.message_ids.includes(selected.id))
      ) {
        s.selected = null;
        s.threadEmails = [];
        s.openingId = null;
        supersedeReads();
      }
      s.displayedQuery = s.search;
      s.searchError = '';
    } catch (e) {
      if (request === v.listRequest) {
        if (s.search) s.searchError = errorText(e);
        else notify(errorText(e), true);
      }
    } finally {
      if (request === v.listRequest) s.loading = false;
    }
  }
  async function refreshOnce() {
    try {
      await loadWorkspace();
      await loadMail(false, true);
      if (s.threads && s.selected) {
        const id = s.selected.id;
        const request = v.readRequest;
        const result = await call<Email[]>('read_mail_thread', { id });
        if (request === v.readRequest && s.selected?.id === id) s.threadEmails = result.map(emailShownAsRead);
      }
    } catch (e) {
      notify(errorText(e), true);
    }
  }
  // At most one pass runs and one waits behind it. A caller gets the first pass that starts
  // after its call, so it waits for the running pass to end plus one more, however many events
  // keep arriving; callers that arrive in the same pass share it. Never rejects.
  v.refresh ??= coalescing(refreshOnce);
  // The same discipline for a snapshot of the workspace alone (counters, drafts).
  v.refreshWorkspace ??= coalescing(async () => {
    try {
      await loadWorkspace();
    } catch (e) {
      notify(errorText(e), true);
    }
  });
  const refresh = () => v.refresh!();
  const refreshWorkspace = () => v.refreshWorkspace!();
  const currentFilter = (): ListFilter => ({
    folder: s.view,
    account_id: s.accountId,
    search: s.search,
    threads: s.threads,
  });
  // Events from one write arrive one by one: the rows they name are read once, shortly after the
  // first of them, instead of once per event.
  function wantRows(ids: number[]) {
    for (const id of ids) v.pendingRows.add(id);
    v.rowsTimer ??= setTimeout(() => {
      v.rowsTimer = undefined;
      void flushRows();
    }, 25);
  }
  async function flushRows() {
    const ids = [...v.pendingRows];
    v.pendingRows.clear();
    if (!ids.length || !isMailView()) return;
    const filter = currentFilter();
    // The list may have become a search or a conversation list since the event: it is not
    // patched row by row.
    if (filter.search.trim() || filter.threads) {
      void refresh();
      return;
    }
    // A list load is under way, or the events together name more rows than one read answers: the
    // pass that follows reads the list itself, after the change.
    if (s.loading || ids.length > readRowsMax) {
      void refresh();
      return;
    }
    const seq = ++v.rowsRequest;
    for (const id of ids) v.rowsLatest.set(id, seq);
    let rows: unknown;
    try {
      rows = await call<MailRow[]>('read_rows', { ids });
    } catch {
      // The rows could not be read: show what the backend holds instead.
      for (const id of ids) if (v.rowsLatest.get(id) === seq) v.rowsLatest.delete(id);
      void refresh();
      return;
    }
    const mine = ids.filter((id) => v.rowsLatest.get(id) === seq);
    for (const id of mine) v.rowsLatest.delete(id);
    // A newer read of all these rows answers for them. And a list the user moved to meanwhile
    // (another view, account, search or display) has these rows already, in their new state.
    // "Load more" or a reload of the same list does not make the answer stale: the rows are
    // patched in whatever page is on screen now.
    if (!mine.length || !sameFilter(filter, currentFilter())) return;
    // Without the flags a snoozed or set-aside message would still match the Inbox.
    if (!carriesFlags(rows)) {
      console.warn('read_rows answered rows without snoozed_until, set_aside or reminder_at; reloading');
      void refresh();
      return;
    }
    s.page = applyRows(
      s.page,
      mine,
      rows.filter((row) => mine.includes(row.id)).map(rowShownAsRead),
      filter,
      Date.now() / 1000,
    );
  }
  // The backend's typed events (ADR-0007 decision 5, INV-004): each one changes what it names and
  // nothing else. Undo events are applied here; a changed message is one read_rows; the counters
  // and drafts are one snapshot; only conversations, a search or a bulk change reload the list.
  function onAppEvent(payload: unknown) {
    if (!s.workspace) return;
    const event = parseAppEvent(payload);
    if (!event) {
      // The payload no longer matches the TypeScript mirror of the backend's events: say so, and
      // show what the backend holds instead of going quiet.
      const kind = (payload as { kind?: unknown } | null)?.kind;
      console.warn('app-event not understood; reloading', kind);
      void refresh();
      return;
    }
    if (event.kind === 'notice') return notify(backendText(event.text));
    // The synchronization changed the mailboxes: take the snapshot again.
    if (event.kind === 'accounts-changed') return void refreshWorkspace();
    if (event.kind === 'history-unread')
      return void (s.historyUnread = { accountId: event.account_id, count: event.count });
    // The first batch after a request is written: the folder shows what the server has.
    if (event.kind === 'history-progress' && s.fetchingFolder.includes(event.account_id))
      s.fetchingFolder = s.fetchingFolder.filter((id) => id !== event.account_id);
    // The activity log changed (centro attività): the footer re-reads its few rows and nothing else.
    if (event.kind === 'activity-changed') return void refreshActivity();
    // Progress events move the single activity indicator and nothing else: the
    // default branch of applyEvent leaves the page and the workspace untouched.
    s.backendActivities = applyActivityEvent(s.backendActivities, event, Date.now());
    if (event.kind === 'sync-progress') onSyncProgress(event.account_id, event.phase);
    // Without a mail list on screen, the next visit loads it.
    if (event.kind === 'email-changed' && !isMailView()) return;
    const workspace = s.workspace;
    if (!workspace) return;
    const effects = applyEvent(s.page, workspace, event, currentFilter());
    if (effects.workspace !== workspace) {
      // A snapshot taken before this event would take the change back.
      ++v.workspaceRequest;
      s.workspace = effects.workspace;
      if (v.snapshotsRunning) void refreshWorkspace();
    }
    if (effects.page !== s.page) s.page = effects.page;
    if (effects.reload) void refresh();
    else {
      if (effects.snapshot) void refreshWorkspace();
      if (effects.needsRows.length) wantRows(effects.needsRows);
    }
  }
  // The phase went to the indicator through applyActivityEvent; what is left is the error text,
  // which the event does not carry. The backend updates its status before it emits, so one pull
  // when a sync ends holds the errors (and clears the last ones after a clean run).
  function onSyncProgress(accountId: number, phase: SyncPhase) {
    v.syncEventAccounts.add(accountId);
    if (phase !== 'error') s.manualSyncError = '';
    if (phase !== 'syncing') void refreshSyncStatus();
  }
  async function refreshSyncStatus(): Promise<MailSyncStatus | null> {
    if (!desktop) return null;
    try {
      const result = await call<MailSyncStatus>('mail_sync_status');
      // An earlier snapshot must not overwrite a newer sync event.
      if (result.revision >= s.syncStatus.revision) s.syncStatus = result;
      return result;
    } catch (error) {
      notify(errorText(error), true);
      return null;
    }
  }
  // At startup a sync may already be running (its events went out before the listener existed):
  // the retained status restores its activity, unless a typed event for that account got here first.
  async function seedSyncActivities() {
    const result = await refreshSyncStatus();
    if (!result) return;
    for (const state of result.accounts) {
      if (state.phase !== 'syncing' || v.syncEventAccounts.has(state.account_id)) continue;
      s.backendActivities = applyActivityEvent(
        s.backendActivities,
        { kind: 'sync-progress', account_id: state.account_id, phase: 'syncing', added: 0 },
        Date.now(),
      );
    }
  }
  async function refreshRegional() {
    if (!s.workspace) return;
    const previous = `${regionalLocale()}|${regionalTimeZone()}`;
    try {
      await loadWorkspace();
      if (previous !== `${regionalLocale()}|${regionalTimeZone()}`) await loadMail();
    } catch (error) {
      notify(errorText(error), true);
    }
  }
  async function init() {
    s.fatal = '';
    try {
      await loadWorkspace();
      await checkEdition();
      await loadMail();
      void loadPeople();
      await tick();
      const first = s.visibleOrder[0] ?? s.page.items[0]?.id;
      if (first !== undefined) await open(first);
    } catch (e) {
      s.fatal = errorText(e);
    }
  }
  // The Professional build drops a session of another server (EX-20) and, without a session,
  // shows its sign-in screen: the Professional app needs an account (owner, 2026-11-19).
  async function checkEdition() {
    const edition = await call<AppEdition>('app_edition').catch(() => null);
    if (edition?.kind !== 'professional') return;
    if (await call<boolean>('server_startup_check').catch(() => false)) await loadWorkspace();
    const status = await call<{ connected: boolean }>('server_status').catch(() => null);
    if (!status?.connected) s.onboarding = edition;
  }
  // A snapshot of the backend that predates a mark must not show its message unread again.
  function emailShownAsRead<T extends { id: number; read: boolean }>(email: T): T {
    return !email.read && v.shownRead.has(email.id) ? { ...email, read: true } : email;
  }
  // A row of one message follows that message; a conversation row reads as read once all its
  // messages do, and the ones the interface knows nothing about count as unread.
  function rowShownAsRead<T extends EmailSummary>(item: T): T {
    return repliesShownAsRead(rowReadByMarks(item));
  }
  function rowReadByMarks<T extends EmailSummary>(item: T): T {
    if (item.read || !item.message_ids.some((id) => v.shownRead.has(id))) return item;
    const read = item.message_ids.every(
      (id) => v.shownRead.has(id) || s.threadEmails.some((email) => email.id === id && email.read),
    );
    return read ? { ...item, read } : item;
  }
  // The replies a row carries follow the same marks: one the interface has shown read stays read
  // until the backend has said so too.
  function repliesShownAsRead<T extends EmailSummary>(item: T): T {
    if (!item.replies?.some((reply) => !reply.read && v.shownRead.has(reply.id))) return item;
    return {
      ...item,
      replies: item.replies.map((reply) =>
        !reply.read && v.shownRead.has(reply.id) ? { ...reply, read: true } : reply,
      ),
    };
  }
  // Whether the Inbox counter counts this message while it is unread: it sits in the Inbox, is
  // not done and not snoozed. A list row tells so only for a one-message row in the Inbox view.
  function countedInInbox(id: number) {
    const email = s.selected?.id === id ? s.selected : s.threadEmails.find((item) => item.id === id);
    if (email)
      return (
        email.folder === 'Inbox' &&
        !email.done &&
        !(email.snoozed_until && email.snoozed_until > Date.now() / 1000)
      );
    return s.view === 'Inbox' && !s.search && s.page.items.some((item) => item.message_ids.includes(id));
  }
  function setInboxCount(count: number) {
    const workspace = s.workspace;
    if (workspace) s.workspace = { ...workspace, counts: { ...workspace.counts, Inbox: count } };
  }
  // INV-006: the message reads as read before the backend has said so, in the list, on screen and
  // in the Inbox counter. `read = false` takes the mark back. Repeating a call changes nothing, so
  // a message is neither counted twice nor restored twice.
  function markReadLocally(id: number, read: boolean) {
    const lowered = v.shownRead.get(id);
    if (read && lowered === undefined) {
      const counted = countedInInbox(id);
      v.shownRead.set(id, counted);
      if (counted && s.workspace) setInboxCount(Math.max(0, (s.workspace.counts.Inbox ?? 0) - 1));
    } else if (!read) {
      // Only a mark made here can be taken back: a message that was read all along stays read.
      if (lowered === undefined) return;
      v.shownRead.delete(id);
      if (lowered && s.workspace) setInboxCount((s.workspace.counts.Inbox ?? 0) + 1);
    }
    if (s.selected?.id === id && s.selected.read !== read) s.selected = { ...s.selected, read };
    if (s.threadEmails.some((email) => email.id === id && email.read !== read))
      s.threadEmails = s.threadEmails.map((email) => (email.id === id ? { ...email, read } : email));
    if (s.page.items.some((item) => item.message_ids.includes(id)))
      s.page = {
        ...s.page,
        items: s.page.items.map((item) =>
          !item.message_ids.includes(id)
            ? item
            : read
              ? rowShownAsRead(item)
              : item.read
                ? { ...item, read: false }
                : item,
        ),
      };
    // A reply under a row is a message of its own: its dot and the blue mark of the arrow follow.
    if (s.page.items.some((item) => item.replies?.some((reply) => reply.id === id && reply.read !== read)))
      s.page = {
        ...s.page,
        items: s.page.items.map((item) =>
          item.replies?.some((reply) => reply.id === id && reply.read !== read)
            ? { ...item, replies: item.replies.map((reply) => (reply.id === id ? { ...reply, read } : reply)) }
            : item,
        ),
      };
    // So is one that "Show more" loaded after the list was read: it is a copy kept beside the page.
    patchExtraRead(id, read);
  }
  async function open(id: number) {
    if (s.openingId === id) return;
    if (shownOnly(id) && !s.openedShownOnly.has(id)) s.openedShownOnly = new Set([...s.openedShownOnly, id]);
    const request = supersedeReads();
    const row =
      s.page.items.find((item) => item.message_ids.includes(id)) ||
      s.page.items.find((item) => item.replies?.some((reply) => reply.id === id)) ||
      ((listed) => s.page.items.find((item) => item.id === listed))(selectedListId());
    const section = row ? (s.sectionLocks[row.id] ?? row.section) : undefined;
    // The reader keeps the message it shows until this one arrives (INV-006).
    s.openingId = id;
    if (section && row && s.sectionLocks[row.id] !== section)
      s.sectionLocks = { ...s.sectionLocks, [row.id]: section };
    // A row that stands for this one message tells at once that it is unread: show it read from
    // the click on. A conversation row cannot say which of its messages is unread; that waits
    // for the message itself.
    if (
      v.shownRead.has(id) ||
      s.page.items.some((item) => !item.read && item.message_ids.length === 1 && item.message_ids[0] === id) ||
      // A reply says for itself whether it is unread, so it reads as read from the click on too.
      s.page.items.some((item) => item.replies?.some((reply) => reply.id === id && !reply.read))
    ) {
      if (!v.shownRead.has(id)) v.earlyMarks.set(id, request);
      markReadLocally(id, true);
      v.markOwner.set(id, request);
    }
    let sent = false;
    let answered = false;
    try {
      const email = await call<Email>('read_mail', { id });
      // The answer settles the mark made at the click: from here it is the ordinary one.
      if (v.earlyMarks.get(id) === request) v.earlyMarks.delete(id);
      if (request !== v.readRequest) return;
      // Opening a message of the conversation on screen keeps the conversation.
      if (!s.threadEmails.some((item) => item.id === email.id)) s.threadEmails = [];
      s.selected = email;
      s.openingId = null;
      if (s.threads) {
        void call<Email[]>('read_mail_thread', { id })
          .then((conversation) => {
            if (request === v.readRequest && s.selected?.id === id)
              s.threadEmails = conversation.map(emailShownAsRead);
          })
          .catch((error) => {
            if (request === v.readRequest) notify(errorText(error), true);
          });
      }
      if (email.read) {
        // The backend had it read already, so the row that said otherwise was out of date.
        answered = true;
        if (v.shownRead.delete(id)) void refresh();
      } else {
        markReadLocally(id, true);
        v.markOwner.set(id, request);
        sent = true;
        await call('mail_action', { id, action: 'markAsSeen', value: null });
        answered = true;
        v.shownRead.delete(id);
        if (request !== v.readRequest) return;
        // A search is not run again, which could reorder it; the mark is already in its rows.
        if (s.search) await loadWorkspace();
        else await refresh();
      }
    } catch (e) {
      if (request === v.readRequest) notify(errorText(e), true);
      // The backend did not take the mark: show what it holds.
      if (sent) void refresh();
    } finally {
      if (v.earlyMarks.get(id) === request) v.earlyMarks.delete(id);
      if (v.markOwner.get(id) === request) {
        v.markOwner.delete(id);
        // Nothing was marked on the backend (older click, error): the message is still unread.
        if (!answered) markReadLocally(id, false);
      }
      if (request === v.readRequest) s.openingId = null;
    }
  }
  function navigate(next: Folder, account?: { id: number | null }) {
    if (s.view === 'Settings' && next !== 'Settings' && s.settingsDirty) {
      s.leaveTarget = next;
      v.leaveAccount = account ?? null;
      return;
    }
    if (account) s.accountId = account.id;
    goTo(next);
  }
  async function finishLeaving(save: boolean) {
    const target = s.leaveTarget;
    const account = v.leaveAccount;
    v.leaveAccount = null;
    if (save && !(await settingsPanel.current?.save())) {
      s.leaveTarget = null;
      return;
    }
    if (!save) settingsPanel.current?.discard();
    s.leaveTarget = null;
    s.settingsDirty = false;
    if (target === 'close') {
      const { getCurrentWindow } = await import('@tauri-apps/api/window');
      await getCurrentWindow().close();
    } else if (target) {
      if (account) s.accountId = account.id;
      goTo(target);
    }
  }
  function goTo(next: Folder) {
    clearTimeout(v.searchTimer);
    s.view = next;
    s.search = '';
    s.searchInput = '';
    s.searchError = '';
    s.displayedQuery = '';
    s.searchSort = '';
    s.searchUndo = null;
    s.selected = null;
    s.openingId = null;
    s.threadEmails = [];
    s.sectionLocks = {};
    supersedeReads();
    ++v.listRequest;
    s.page = emptyPage();
    s.fetchingFolder = [];
    void loadMail();
    fetchFolder(next);
  }
  // Gmail's Trash is not in All Mail: opening it asks the server for its newest messages, unless
  // the account already lists Spam and Trash (Settings → Search). Spam has no view of its own: a
  // search with in:spam reads it on the server.
  function fetchFolder(next: Folder) {
    if (!desktop || next !== 'Trash') return;
    const accounts = (s.workspace?.accounts ?? []).filter(
      (account) =>
        account.kind === 'Gmail' &&
        (s.accountId === null || account.id === s.accountId) &&
        !account.storage_policy.include_spam_trash,
    );
    s.fetchingFolder = accounts.map((account) => account.id);
    // A batch that fails sends no progress: the line does not stay forever.
    const shown = s.fetchingFolder;
    setTimeout(() => {
      if (s.fetchingFolder === shown) s.fetchingFolder = [];
    }, 60_000);
    for (const account of accounts)
      void call('history_fetch_folder', { accountId: account.id, folder: next }).catch((e) => {
        s.fetchingFolder = s.fetchingFolder.filter((id) => id !== account.id);
        notify(errorText(e), true);
      });
  }
  function selectAccount(id: number | null) {
    // Account buttons filter mail, so pages without mail hand over to the inbox.
    if (!isMailView()) return navigate('Inbox', { id });
    s.accountId = id;
    s.selected = null;
    s.openingId = null;
    s.threadEmails = [];
    s.sectionLocks = {};
    supersedeReads();
    s.page = emptyPage();
    void loadMail();
  }
  function setThreads(value: boolean) {
    if (s.threads === value) return;
    s.threads = value;
    savePreference('mail-display', value ? 'threads' : 'emails');
    s.selected = null;
    s.openingId = null;
    s.threadEmails = [];
    s.sectionLocks = {};
    supersedeReads();
    s.page = emptyPage();
    void loadMail();
  }
  function toggleSidebar() {
    s.sidebarCollapsed = !s.sidebarCollapsed;
    savePreference('sidebar-collapsed', String(s.sidebarCollapsed));
  }
  /** A click on an activity row opens the list or the settings page it belongs to. */
  function openActivity(activity: Activity) {
    switch (activity.kind) {
      case 'index':
      case 'history':
      case 'model':
        navigate('Settings');
        void tick().then(() => settingsPanel.current?.showSection('Search'));
        break;
      case 'contacts':
        navigate('Contacts');
        break;
      case 'sync':
        navigate('Inbox');
        break;
      case 'confirming':
        break;
    }
  }
  function find(immediate = false) {
    clearTimeout(v.searchTimer);
    ++v.listRequest;
    const run = () => {
      s.search = s.searchInput.trim();
      void loadMail();
    };
    if (immediate) run();
    else v.searchTimer = setTimeout(run, 250);
  }
  function applySearch(query: string, recovery = false) {
    if (recovery) {
      // The values are replaced, never edited in place, so keeping them is a snapshot.
      s.searchUndo = {
        query: s.searchInput,
        page: s.page,
        sort: s.searchSort,
        accountId: s.accountId,
        selected: s.selected,
        threadEmails: s.threadEmails,
        scroll: document.querySelector('.mail-scroll')?.scrollTop || 0,
      };
    }
    s.searchInput = query;
    find(true);
  }
  function undoSearch() {
    if (!s.searchUndo) return;
    clearTimeout(v.searchTimer);
    ++v.listRequest;
    supersedeReads();
    s.openingId = null;
    const previous = s.searchUndo;
    s.searchInput = s.search = s.displayedQuery = previous.query;
    s.searchSort = previous.sort;
    s.accountId = previous.accountId;
    s.page = previous.page;
    s.selected = previous.selected;
    s.threadEmails = previous.threadEmails;
    s.searchUndo = null;
    s.searchError = '';
    s.loading = false;
    requestAnimationFrame(() => {
      const list = document.querySelector('.mail-scroll');
      if (list) list.scrollTop = previous.scroll;
    });
  }
  async function searchRemote(more = false) {
    if (s.remoteBusy || !s.search) return;
    s.remoteBusy = true;
    const activeQuery = s.search;
    const cursor = more && s.remoteQuery === activeQuery && s.remoteAccount === s.accountId ? s.remoteNext : null;
    try {
      const result = await call<{
        added: number;
        errors: string[];
        next: unknown;
        local_only: boolean;
        notice: string | null;
      }>('search_remote_mail', {
        query: activeQuery,
        accountId: s.accountId,
        cursor,
      });
      if (result.errors.length) notify(backendText(result.errors.join('\n')), true);
      if (result.local_only && result.notice) notify(backendText(result.notice));
      s.remoteQuery = activeQuery;
      s.remoteAccount = s.accountId;
      s.remoteNext = result.next ?? null;
      await refresh();
    } catch (error) {
      notify(errorText(error), true);
    } finally {
      s.remoteBusy = false;
    }
  }
  async function cancelRemote() {
    try {
      await call('cancel_remote_search');
    } catch (error) {
      notify(errorText(error), true);
    }
  }
  function setBusy(ids: number[], on: boolean) {
    s.busyIds = on
      ? new Set([...s.busyIds, ...ids])
      : new Set([...s.busyIds].filter((id) => !ids.includes(id)));
  }
  // Messages with a queued action that takes them out of the list.
  function departingMessages() {
    const gone = new Set<number>();
    for (const pending of s.workspace?.undo_actions ?? [])
      if (
        pending.status !== 'failed' &&
        pending.operation.kind === 'mail' &&
        leavesList.includes(pending.operation.action)
      )
        gone.add(pending.operation.email_id);
    return gone;
  }
  // Rows whose every message has a queued action that takes it out of the list. The reload that
  // drops them may still be running, so triage must not step onto them.
  function departingRows() {
    const gone = departingMessages();
    return new Set(
      s.page.items
        .filter((item) => item.message_ids.length && item.message_ids.every((m) => gone.has(m)))
        .map((item) => item.id),
    );
  }
  // The ids are busy only while the backend accepts the action. Reloading the list, moving on
  // to the next message and re-reading the current one start afterwards and block nothing.
  // A message the list shows without holding it (the user's own reply in Sent, under an Inbox
  // conversation) can be read and answered, but nothing is done to it from this view (INV-002).
  // A message opened as shown-only stays so until the view changes, even once its conversation has
  // left the list (moved to another folder, a rule, another window): it still lives in Sent.
  function shownOnly(id: number) {
    return (
      !s.page.items.some((item) => item.message_ids.includes(id)) &&
      (s.openedShownOnly.has(id) || s.page.items.some((item) => repliesOf(item).some((reply) => reply.id === id)))
    );
  }
  const selectedInView = () => !s.selected || !shownOnly(s.selected.id);
  async function action(id: number, a: string, value?: string, ids: number[] = [id]) {
    if (ids.some((messageId) => s.busyIds.has(messageId) || shownOnly(messageId))) return;
    // The reader keeps a message that was just sent away until the next one arrives, with its
    // buttons live: sending it away twice would queue the action twice.
    if (
      openingOther() &&
      leavesList.includes(a) &&
      s.selected &&
      ids.includes(s.selected.id) &&
      departingMessages().has(s.selected.id)
    )
      return;
    const leaving = !!s.selected && ids.includes(s.selected.id) && leavesList.includes(a);
    // The list order and the position in it when the action was pressed.
    const orderBefore = [...s.visibleOrder];
    // The entry of the list the reader's message stands for: its own line when the list shows it by
    // itself (a reply under an open conversation), else the row that holds it. Triage goes on from
    // there, not from the row of the whole conversation.
    const leftEntry = s.selected && orderBefore.includes(s.selected.id) ? s.selected.id : rowOfSelected();
    const positionBefore = leftEntry === null ? -1 : orderBefore.indexOf(leftEntry);
    setBusy(ids, true);
    try {
      for (const messageId of ids) {
        const pending = await call<UndoAction>('queue_mail_action', {
          id: messageId,
          action: a,
          value: value || null,
        });
        queued(pending);
      }
    } catch (e) {
      notify(errorText(e), true);
      void refresh();
      return;
    } finally {
      setBusy(ids, false);
    }
    const reloaded = refresh();
    // The selection may have moved while the backend was answering.
    if (!s.selected || !ids.includes(s.selected.id)) return;
    try {
      if (leaving) {
        // Triage continues on the next message, so the reader never goes blank mid-flow. The next
        // row comes from the order before the reload; if it has vanished by now, open() reports
        // the failed read and the reload that follows drops the row.
        const departing = departingRows();
        const goneMessages = departingMessages();
        const isRow = (entryId: number) => s.page.items.some((item) => item.id === entryId);
        const next = [
          ...orderBefore.slice(positionBefore + 1),
          ...orderBefore.slice(0, Math.max(positionBefore, 0)).reverse(),
        ].find(
          (entryId) =>
            entryId !== leftEntry &&
            !departing.has(entryId) &&
            // A reply on its own line leaves with its message; and one that is only shown (the
            // user's own reply in Sent) cannot be triaged, so the reader would stall on it.
            !(!isRow(entryId) && (goneMessages.has(entryId) || shownOnly(entryId))) &&
            s.visibleOrder.includes(entryId),
        );
        let opened: Promise<void> | undefined;
        if (next !== undefined) opened = open(next);
        else {
          s.selected = null;
          s.threadEmails = [];
          supersedeReads();
        }
        // Focus is restored once the list has settled; rows can be rebuilt by the reload.
        await Promise.all([reloaded, opened]);
        // If the next message could not be opened, the one sent away must not stay on screen.
        if (s.openingId === null && s.selected && ids.includes(s.selected.id)) {
          s.selected = null;
          s.threadEmails = [];
        }
        await restoreListFocus();
      } else {
        const selectedId = s.selected.id;
        const request = v.readRequest;
        // Actions on one message are no longer serialised, so their re-reads overlap: only the
        // newest one may show its result. A separate counter, because ++readRequest would also
        // cancel an open() of another message.
        const seq = ++v.rereadSeq;
        const email = await call<Email>('read_mail', { id: selectedId });
        if (seq === v.rereadSeq && request === v.readRequest && s.selected?.id === selectedId)
          s.selected = emailShownAsRead(email);
      }
    } catch (e) {
      notify(errorText(e), true);
      await refresh();
    }
  }
  // The native menu of list rows (lib/row-menu.ts, docs/aspetto-nativo.md rule 33). `rowIds` are the
  // rows it is for: the one right-clicked, or every ticked one. It acts without opening the row,
  // through the paths the row and the reader use and with their locks; the focus is back on the row.
  async function openRowMenu(id: number, at: { x: number; y: number }, rowIds: number[]) {
    // The rows as the list holds them now, and the messages they stand for: a conversation row
    // acts on every message of it the view holds, as its pin button does.
    const listedRows = () =>
      rowIds
        .map((rowId) => s.page.items.find((item) => item.id === rowId))
        .filter((row): row is EmailSummary => !!row);
    const messagesOf = (rows: EmailSummary[]) => rows.flatMap((row) => (s.threads ? row.message_ids : [row.id]));
    const rows = listedRows();
    if (!rows.length) return;
    const locked = messagesOf(rows).some((messageId) => s.busyIds.has(messageId) || shownOnly(messageId));
    const choice = await showRowMenu(rowMenuEntries(rows, s.view, locked, tr), at);
    const rowButton = (rowId: number) =>
      document.querySelector<HTMLElement>(`.mail-row[data-row-id="${rowId}"] .mail-open`);
    rowButton(id)?.focus();
    if (!choice) return;
    if (choice === 'reply' || choice === 'reply-all' || choice === 'forward') {
      await compose(id, choice === 'forward', choice === 'reply-all');
      return;
    }
    const command = rowMenuAction(choice);
    // The list may have changed while the menu was open: a row that left is not acted on.
    const messages = messagesOf(listedRows());
    if (!command || !messages.length) return;
    // Rows sent away leave the focus on the next row, as triage does; `action` moves it itself when
    // the message on show is among them.
    if (leavesList.includes(command.action) && !(s.selected && messages.includes(s.selected.id))) {
      const listed = s.visibleOrder.filter((entry) => s.page.items.some((item) => item.id === entry));
      const from = listed.indexOf(id);
      const next = [...listed.slice(from + 1), ...listed.slice(0, Math.max(from, 0)).reverse()].find(
        (entry) => !rowIds.includes(entry),
      );
      (next === undefined ? document.querySelector<HTMLElement>('.mail-list h1') : rowButton(next))?.focus();
    }
    await action(id, command.action, command.value, messages);
  }
  async function restoreListFocus() {
    await tick();
    const active = document.activeElement;
    // A button of the reader belonged to the message that was sent away.
    if (
      active &&
      active !== document.body &&
      !(active as HTMLButtonElement).disabled &&
      !active.closest('.reader')
    )
      return;
    (selectedRowButton() || document.querySelector<HTMLElement>('.mail-list h1'))?.focus();
  }
  // The open button of the row on show in the list, a conversation's main row or one of its replies.
  function selectedRowButton() {
    return (
      document.querySelector<HTMLElement>('.mail-row.selected .mail-open') ||
      document.querySelector<HTMLElement>('.reply-row.selected .reply-open')
    );
  }
  function queued(pending: UndoAction) {
    ++v.workspaceRequest;
    const workspace = s.workspace;
    if (workspace)
      s.workspace = {
        ...workspace,
        undo_actions: [...workspace.undo_actions.filter((a) => a.id !== pending.id), pending],
      };
  }
  async function undoPending(pending: UndoAction) {
    try {
      await call('undo_action', { id: pending.id });
      await refresh();
      const op = pending.operation;
      if (op.kind === 'mail') {
        if (s.page.items.some((e) => e.message_ids.includes(op.email_id))) await open(op.email_id);
      } else {
        const draft = s.workspace?.drafts.find((d) => d.id === op.draft_id);
        if (draft) openComposer(draft);
      }
    } catch (error) {
      notify(errorText(error), true);
      await refresh();
    }
  }
  async function dismissPending(id: number) {
    try {
      await call('dismiss_action', { id });
      await loadWorkspace();
    } catch (error) {
      notify(errorText(error), true);
    }
  }
  async function compose(replyTo: number | null = null, forward = false, everyone = false) {
    // The control that asked for the composer, before the draft is made: that wait can move the focus.
    const opener = document.activeElement;
    try {
      // A row's menu answers everyone without opening the message: its recipients are read first.
      const message =
        everyone && replyTo !== null && s.selected?.id !== replyTo ? await call<Email>('read_mail', { id: replyTo }) : null;
      let draft = await call<Draft>('create_draft', { accountId: s.accountId, replyTo, forward });
      const selected = message ?? s.selected;
      if (everyone && selected && selected.id === replyTo) {
        const own = new Set((s.workspace?.accounts ?? []).map((account) => account.email.trim().toLowerCase()));
        draft = await call<Draft>('save_draft', {
          draft: { ...draft, ...replyAllRecipients(selected, draft.to, own) },
          importLeaseIds: [],
        });
      }
      openComposer(draft, opener);
      await loadWorkspace();
    } catch (e) {
      notify(errorText(e), true);
    }
  }
  // A mailto: link, of a message or of another program (lib/mailto.ts, rule 32): a new message from the
  // account in view, as «Nuovo messaggio» makes it, filled with the link's fields and opened. Never sent
  // from here: the person reads it and sends it.
  async function composeMailto(fields: MailtoFields) {
    const opener = document.activeElement;
    try {
      const created = await call<Draft>('create_draft', { accountId: s.accountId, replyTo: null, forward: false });
      const draft = await call<Draft>('save_draft', { draft: { ...created, ...mailtoDraft(fields) }, importLeaseIds: [] });
      openComposer(draft, opener);
      await loadWorkspace();
    } catch (e) {
      notify(errorText(e), true);
    }
  }
  // The reader's arrows: the row before or after the one on show, in the order the list shows them.
  function readerPosition() {
    const listed = selectedListId();
    return listed === null ? 0 : s.visibleOrder.indexOf(listed) + 1;
  }
  function stepReader(delta: number) {
    const position = readerPosition();
    const next = position - 1 + delta;
    if (position > 0 && next >= 0 && next < s.visibleOrder.length) void open(s.visibleOrder[next]);
  }
  function openComposer(draft: Draft, opener: Element | null = document.activeElement) {
    if (opener instanceof HTMLElement && opener !== document.body && !opener.closest('.composer-panel'))
      v.composerOpeners.set(draft.id, opener);
    if (!s.composers.some((item) => item.id === draft.id)) {
      s.composers = [...s.composers, structuredClone(draft)];
    }
    s.activeComposerId = draft.id;
  }
  // `returnFocus` is false when another composer takes the place of this one (a copy of the draft).
  function closeComposer(id: number, returnFocus = true) {
    s.composers = s.composers.filter((draft) => draft.id !== id);
    if (s.activeComposerId === id) s.activeComposerId = null;
    const opener = v.composerOpeners.get(id);
    v.composerOpeners.delete(id);
    if (returnFocus) void restoreComposerFocus(id, opener);
  }
  // A composer that closes (sent, saved, discarded) fades out with the focus still in it, or leaves it on
  // <body> when its tab was what had it. The focus goes to the row on show in the list, else to the control
  // that opened the composer, else to the heading of the list or of the Drafts page; a focus that is
  // already somewhere else stays.
  async function restoreComposerFocus(id: number, opener: HTMLElement | undefined) {
    await tick();
    const placed = placeFocusAfterComposer(id, opener);
    // The reload the same action started can take away what was just focused (a draft that was sent
    // leaves the Drafts list): once it has landed, a focus that fell to <body> goes to the next place.
    await v.workspaceSettling;
    await tick();
    if (placed && !placed.isConnected && document.activeElement === document.body) placeFocusAfterComposer(id, opener);
  }
  function placeFocusAfterComposer(id: number, opener: HTMLElement | undefined) {
    const active = document.activeElement;
    if (active && active !== document.body && !document.getElementById(`compose-panel-${id}`)?.contains(active)) return null;
    const target =
      selectedRowButton() ||
      (opener?.isConnected ? opener : null) ||
      document.querySelector<HTMLElement>('.mail-list h1, .drafts-page h1');
    target?.focus();
    return target;
  }
  async function sync() {
    if (syncing()) return;
    s.manualSyncError = '';
    s.manualSyncing = true;
    s.sectionLocks = {};
    try {
      const result = await call<{ added: number; errors: string[] }>('sync_mail');
      await refresh();
      void loadPeople();
      if (!desktop)
        notify(
          result.errors.length
            ? backendText(result.errors.join('\n'))
            : result.added
              ? result.added === 1
                ? tr('1 nuovo messaggio', '1 new message')
                : tr(`${result.added} nuovi messaggi`, `${result.added} new messages`)
              : tr('La posta in arrivo è aggiornata', 'Your inbox is up to date'),
          !!result.errors.length,
        );
    } catch (e) {
      s.manualSyncError = errorText(e);
    } finally {
      s.manualSyncing = false;
    }
  }
  function theme() {
    s.appearance = dark() === 'dark' ? 'light' : 'dark';
  }
  function keyboard(e: KeyboardEvent) {
    // The app's zoom replaces the browser's, in every field and dialog (docs/aspetto-nativo.md).
    const zoom = desktop ? zoomKey(e) : null;
    if (zoom) {
      e.preventDefault();
      void zoomWindow(zoom);
      return;
    }
    if (s.printPreview || shortcutCoveredByLayer(e.target)) return;
    const inComposer = e.target instanceof HTMLElement && e.target.closest('.composer-panel,.composer-dock');
    const editing = shortcutIgnored(e.target);
    // AltGr is Ctrl+Alt on Windows: a typed character, never a chord (Ctrl+AltGr+K, Ctrl+AltGr+Z).
    const chord = isAppChord(e);
    if (!inComposer && !editing && chord && !e.shiftKey && e.key.toLowerCase() === 'z') {
      const workspace = s.workspace;
      const latest =
        workspace &&
        [...workspace.undo_actions].reverse().find((a) => a.status === 'pending' && a.expires_at_ms > Date.now());
      if (latest) {
        e.preventDefault();
        void undoPending(latest);
      }
      return;
    }
    if (chord && e.key.toLowerCase() === 'k') {
      e.preventDefault();
      if (!isMailView()) navigate('Inbox');
      void tick().then(() => searchElement.current?.focus());
      return;
    }
    if (
      inComposer ||
      editing ||
      // An open attachment covers the message: shortcuts must not act on it.
      document.querySelector('.attachment-panel')
    )
      return;
    if (e.ctrlKey || e.metaKey || e.altKey) return;
    if (e.key.toLowerCase() === 'n') {
      e.preventDefault();
      void compose();
    }
    // While another message is being read the shortcuts wait for it, as they did when the reader
    // went blank: the target would be the one on screen, not the one just picked.
    const selected = s.selected;
    if (selected && !openingOther() && e.key.toLowerCase() === 'r') {
      e.preventDefault();
      void compose(selected.id);
    }
    if (
      selected &&
      !openingOther() &&
      e.key.toLowerCase() === 'e' &&
      !s.busyIds.has(selected.id) &&
      selectedInView()
    ) {
      e.preventDefault();
      void action(selected.id, 'archive');
    }
    // The arrows step through the messages only from the page, the list or the reader: in the
    // sidebar, on a top-bar button or in the activity panel they belong to that control.
    if (
      isMailView() &&
      ['ArrowUp', 'ArrowDown'].includes(e.key) &&
      arrowsMoveMail(e.target) &&
      s.visibleOrder.length
    ) {
      e.preventDefault();
      const listed = selectedListId();
      const at = listed === null ? -1 : s.visibleOrder.indexOf(listed);
      const next = Math.max(0, Math.min(s.visibleOrder.length - 1, at + (e.key === 'ArrowDown' ? 1 : -1)));
      const inList = !!(e.target instanceof HTMLElement && e.target.closest('.mail-list'));
      void open(s.visibleOrder[next]).then(async () => {
        await tick();
        if (inList)
          document
            .querySelector<HTMLElement>('.mail-row.selected .mail-open, .reply-row.selected .reply-open')
            ?.focus();
      });
    }
  }

  // The window listeners call the functions of the latest render (they read the same state).
  const latest = useRef({ keyboard, refreshRegional });
  latest.current = { keyboard, refreshRegional };

  const darkNow = dark();
  const italianNow = isItalian();
  useLayoutEffect(() => {
    document.documentElement.dataset.theme = darkNow;
    saveAppearance(s.appearance);
    void paintWindowBackground();
    void windowAppearance(s.appearance, italianNow);
  }, [darkNow, s.appearance, italianNow]);

  // Sorting/grouping in MailList supplies the actual visible row order.
  const mailView = isMailView();
  const orderKey = s.visibleOrder.join(',');
  useEffect(() => {
    prefetch();
    return () => clearTimeout(v.prefetchTimer);
  }, [orderKey, mailView]);

  // Errors stay separate alerts (ADR-0007 decision 7): one per distinct error text, again after a
  // sync without errors cleared it. Progress and completion get no toast; the indicator has them.
  const errors = syncErrorList();
  const syncErrorText = errors.join('\n');
  useEffect(() => {
    const text = syncErrorText;
    if (!text) {
      v.reportedSyncError = '';
      return;
    }
    if (text === v.reportedSyncError) return;
    v.reportedSyncError = text;
    notify(`${tr('Sincronizzazione non completata', 'Sync incomplete')}: ${backendText(text)}`, true);
  }, [syncErrorText]);

  // A message opened as shown-only stops being one when the view or the account changes.
  useEffect(() => {
    if (s.openedShownOnly.size) s.openedShownOnly = new Set();
  }, [s.view, s.accountId]);

  // The main view fades in when it changes (enter only: two views never share the screen).
  const firstView = useRef(true);
  useLayoutEffect(() => {
    if (firstView.current) {
      firstView.current = false;
      return;
    }
    if (reduceMotion || typeof content.current?.animate !== 'function') return;
    content.current.animate([{ opacity: 0 }, { opacity: 1 }], { duration: 120, easing: 'ease-out' });
  }, [s.view]);

  useEffect(() => {
    void init();
    void refreshActivity();
    const onKeyDown = (event: KeyboardEvent) => latest.current.keyboard(event);
    const onFocus = () => void latest.current.refreshRegional();
    window.addEventListener('keydown', onKeyDown);
    window.addEventListener('focus', onFocus);
    const colorScheme = window.matchMedia('(prefers-color-scheme: dark)');
    const systemThemeChanged = (event: MediaQueryListEvent) => {
      s.systemDark = event.matches;
    };
    s.systemDark = colorScheme.matches;
    colorScheme.addEventListener('change', systemThemeChanged);
    let disposed = false;
    const unsubscribers: (() => void)[] = [];
    // A file or a link dropped outside any drop target (native file drop is off: HTML5 drag needs
    // it on Windows) is stopped by installBrowserGuards (lib/browser-guards.ts, main.tsx).
    unsubscribers.push(
      registerCloseGuard(
        'meetings',
        async () => {
          const capture = await meetingCall<CaptureSnapshot>('meeting_capture_status');
          if (captureActive(capture) && !(await call<boolean>('meeting_confirm_close', { italian: isItalian() })))
            return false;
          if (!(await flushMeetingDrafts())) {
            notify(
              tr(
                'Gli appunti non sono ancora salvati. Apri Meeting per risolvere il conflitto.',
                'Your notes aren’t saved yet. Open Meetings to resolve the conflict.',
              ),
              true,
            );
            navigate('Meetings');
            return false;
          }
          if (captureActive(capture)) await meetingCall('meeting_stop');
          await call('meeting_cancel_all_jobs');
          return true;
        },
        {
          last: true,
          // Session end: the notes first; the recording only once the session really ends. Unsaved
          // notes answer false; a stop or cancel that fails throws, and endSession counts that too.
          sessionEnd: async (last) => {
            const saved = await flushMeetingDrafts();
            if (!last) return saved;
            if (captureActive(await meetingCall<CaptureSnapshot>('meeting_capture_status'))) await meetingCall('meeting_stop');
            await call('meeting_cancel_all_jobs');
            return saved;
          },
        },
      ),
    );
    unsubscribers.push(installWindowShell());
    void installWindowCloseHandler(notify)
      .then((unsubscribe) => (disposed ? unsubscribe() : unsubscribers.push(unsubscribe)))
      .catch((error) => notify(errorText(error), true));
    unsubscribers.push(
      registerCloseGuard(
        'settings',
        async () => {
          if (s.view !== 'Settings' || !s.settingsDirty) return true;
          s.leaveTarget = 'close';
          v.leaveAccount = null;
          return false;
        },
        // Session end: nobody can answer "Vuoi salvare?", so the changes are saved. Nothing to save
        // counts as saved; a save that fails answers false.
        {
          sessionEnd: async () =>
            s.view !== 'Settings' || !s.settingsDirty || (await settingsPanel.current?.save()) !== false,
        },
      ),
    );
    // Mail links to the composer: a message's, and in the desktop app those Windows hands to Qamille.
    unsubscribers.push(
      installMailtoRouting({
        compose: (fields) => void composeMailto(fields),
        refused: () => notify(tr('Questo link di posta non si può aprire.', 'This mail link cannot be opened.'), true),
        failed: (error) => notify(errorText(error), true),
        native: desktop ? { listen, call } : undefined,
      }),
    );
    // `workspace-changed` and its preview twin carry no content: until the backend stops sending
    // them (Task 2.2) they stay as a coalesced full refresh next to the typed `app-event`.
    const previewChanged = () => void refresh();
    const previewAppEvent = (event: Event) => onAppEvent((event as CustomEvent<unknown>).detail);
    if (!desktop) {
      window.addEventListener('preview-workspace-changed', previewChanged);
      window.addEventListener('app-event', previewAppEvent);
    }
    if (desktop) {
      void listen<unknown>('app-event', (event) => onAppEvent(event.payload))
        .then((unsubscribe) => {
          if (disposed) return unsubscribe();
          unsubscribers.push(unsubscribe);
          // Subscribe first so startup cannot miss an already running sync.
          void seedSyncActivities();
        })
        .catch((error) => notify(errorText(error), true));
      // Startup recovery runs off the main thread. Refresh once after the listener is attached so a
      // recovery that finished earlier is still visible; one finishing later emits the event.
      void listen('startup-recovery-done', () => {
        notify(tr('Recupero all’avvio completato.', 'Startup recovery completed.'));
        void refresh();
      })
        .then((unsubscribe) => (disposed ? unsubscribe() : unsubscribers.push(unsubscribe)))
        .catch((error) => notify(errorText(error), true));
      for (const [name, handler] of [
        ['workspace-changed', () => void refresh()],
        ['background-notice', (event: { payload: string }) => notify(backendText(event.payload), true)],
      ] as const) {
        void listen<string>(name, handler).then((unsubscribe) => {
          if (disposed) return unsubscribe();
          unsubscribers.push(unsubscribe);
          if (name === 'workspace-changed') void refresh();
        });
      }
    }
    // Google calendars are read into the local cache at startup and every 5 minutes, so the
    // calendar page already shows every appointment when it opens.
    let lastCalendarNotice = '';
    const syncCalendars = () =>
      void syncGoogleCalendars().then(({ errors }) => {
        const message = errors.length ? errorText(errors[0]) : '';
        if (message && message !== lastCalendarNotice) notify(message, true);
        lastCalendarNotice = message;
      });
    const calendarSync = desktop ? window.setInterval(syncCalendars, 5 * 60_000) : 0;
    const calendarFirstSync = desktop ? window.setTimeout(syncCalendars, 3000) : 0;
    return () => {
      disposed = true;
      window.removeEventListener('keydown', onKeyDown);
      window.removeEventListener('focus', onFocus);
      window.clearInterval(calendarSync);
      window.clearTimeout(calendarFirstSync);
      window.removeEventListener('preview-workspace-changed', previewChanged);
      window.removeEventListener('app-event', previewAppEvent);
      colorScheme.removeEventListener('change', systemThemeChanged);
      unsubscribers.forEach((fn) => fn());
      clearTimeout(v.rowsTimer);
      clearTimeout(v.searchTimer);
      clearTimeout(v.prefetchTimer);
    };
  }, []);

  const workspace = s.workspace;
  const view = s.view;
  const drag = desktop ? '' : undefined;
  const listedId = selectedListId();
  const syncingNow = syncing();

  return (
    <>
      {desktop && (s.fatal || !workspace) && (
        <header className="startup-window-bar" data-tauri-drag-region="">
          <span data-tauri-drag-region="">Qamille</span>
          <WindowControls onerror={(message) => notify(message, true)} />
        </header>
      )}
      {s.fatal ? (
        <main className="startup-state">
          <Icon name="info" size={32} />
          <h1>{tr('Qamille non è riuscito ad aprire il tuo spazio di lavoro', 'Qamille couldn’t open your workspace')}</h1>
          <p role="alert">{s.fatal}</p>
          <Button variant="primary" onClick={() => void init()}>
            {tr('Riprova', 'Try again')}
          </Button>
        </main>
      ) : workspace && s.onboarding ? (
        <Onboarding
          edition={s.onboarding}
          consented={workspace.settings.compute_consented_urls ?? []}
          onerror={(message) => notify(message, true)}
          onDone={() => {
            s.onboarding = null;
            void loadWorkspace();
          }}
        />
      ) : workspace ? (
        <div
          className={cn('app-shell', s.sidebarCollapsed && 'sidebar-collapsed', s.composers.length > 0 && 'has-composers')}
          inert={s.closingWindow}
        >
          <Sidebar
            workspace={workspace}
            view={view}
            accountId={s.accountId}
            navigate={navigate}
            selectAccount={selectAccount}
            compose={() => void compose()}
            collapsed={s.sidebarCollapsed}
            toggleCollapsed={toggleSidebar}
            activities={s.backendActivities}
            undoActions={workspace.undo_actions}
            openActivity={openActivity}
            showAllActivity={() => navigate('Activity')}
          />
          <main className="main-shell">
            <header className={cn('topbar', desktop && 'desktop')} data-tauri-drag-region={drag}>
              <div className={cn('workspace-name', mailView && 'path')} data-tauri-drag-region={drag}>
                {mailView ? (
                  <>
                    {tr('Posta', 'Mail')}
                    <span className="workspace-divider" data-tauri-drag-region={drag}>
                      /
                    </span>
                    <span data-tauri-drag-region={drag}>
                      {workspace.accounts.find((a) => a.id === s.accountId)?.name || tr('Tutti gli account', 'All accounts')}
                    </span>
                  </>
                ) : (
                  <span data-tauri-drag-region={drag}>{folderLabel(view)}</span>
                )}
              </div>
              {mailView && (
                <div className="global-search">
                  <Icon name="search" size={18} />
                  <input
                    ref={searchElement}
                    value={s.searchInput}
                    onChange={(event) => {
                      s.searchInput = event.target.value;
                      find();
                    }}
                    onKeyDown={(event) => {
                      // The Enter that picks an IME candidate is not a search, and the Escape that drops
                      // one is not a clear (aspetto-nativo.md, rule 24).
                      if (event.key === 'Enter' && !imeComposing(event.nativeEvent)) {
                        event.preventDefault();
                        find(true);
                      } else if (event.key === 'Escape' && !imeComposing(event.nativeEvent)) {
                        // Esc empties a field that has text, and the list goes back to the whole view;
                        // on an empty field it does what it did before: nothing.
                        if (s.searchInput) {
                          event.preventDefault();
                          applySearch('');
                        }
                      }
                    }}
                    aria-label={tr('Cerca email', 'Search mail')}
                    aria-keyshortcuts="Control+K"
                    placeholder={tr('Cerca una persona, una parola o una data', 'Search a person, a word or a date')}
                  />
                  <kbd>Ctrl K</kbd>
                </div>
              )}
              <div className="topbar-end">
                <div className="topbar-actions">
                  {!desktop && (
                    <span
                      className="preview-badge"
                      title={tr('Solo dati di esempio. L’invio è disattivato.', 'Sample data only. Sending is disabled.')}
                    >
                      {tr('Anteprima', 'Preview')}
                    </span>
                  )}
                  <Button
                    variant="icon"
                    className={cn(!syncingNow && errors.length > 0 && 'sync-failed')}
                    aria-label={tr('Sincronizza posta', 'Sync mail')}
                    title={
                      errors.length
                        ? `${tr('Premi per riprovare la sincronizzazione', 'Click to retry sync')}\n${backendText(syncErrorText)}`
                        : tr('Sincronizza posta', 'Sync mail')
                    }
                    onClick={() => void sync()}
                  >
                    <Icon name="refresh" size={18} />
                  </Button>
                  <Button
                    variant="icon"
                    aria-label={tr('Attiva o disattiva l’aspetto scuro', 'Toggle dark appearance')}
                    title={tr('Cambia aspetto', 'Toggle appearance')}
                    onClick={theme}
                  >
                    <Icon name={darkNow === 'dark' ? 'sun' : 'moon'} size={18} />
                  </Button>
                </div>
                {desktop && <WindowControls onerror={(message) => notify(message, true)} />}
              </div>
            </header>
            {/* Under the title bar: above it, it would push the window buttons down and take the drag strip. */}
            <LegacyLocalNotice onerror={(message) => notify(message, true)} />
            {mailView && (
              <SearchBar
                query={s.searchInput}
                workspace={workspace}
                accountId={s.accountId}
                page={s.page}
                loading={s.loading}
                error={s.searchError}
                displayedQuery={s.displayedQuery}
                sort={s.searchSort}
                remoteBusy={s.remoteBusy}
                remoteMore={!!s.remoteNext && s.remoteQuery === s.search && s.remoteAccount === s.accountId}
                canUndo={!!s.searchUndo}
                onquery={applySearch}
                onsort={(value) => {
                  s.searchSort = value;
                  void loadMail();
                }}
                onremote={() => void searchRemote()}
                onremotemore={() => void searchRemote(true)}
                oncancel={() => void cancelRemote()}
                onundo={undoSearch}
                onallaccounts={() => {
                  s.accountId = null;
                  void loadMail();
                }}
                focusSearch={() => searchElement.current?.focus()}
              />
            )}
            <div className="workspace-content" ref={content}>
              {mailView ? (
                <>
                  <MailList
                    selectionScope={`${view}:${s.accountId}:${s.search}:${s.threads}`}
                    threads={s.threads}
                    busyIds={s.busyIds}
                    outputBusy={s.outputBusy}
                    printEmails={(ids) => void printEmails(ids)}
                    exportEmails={(ids) => void exportEmails(ids, 'mbox')}
                    sectionLocks={s.sectionLocks}
                    visibleOrder={s.visibleOrder}
                    onVisibleOrderChange={(order) => {
                      // A new array with the same rows changes nothing.
                      if (order.join(',') !== s.visibleOrder.join(',')) s.visibleOrder = order;
                    }}
                    setThreads={setThreads}
                    page={s.page}
                    view={view}
                    search={s.search}
                    loading={s.loading}
                    searchError={s.searchError}
                    accounts={workspace.accounts}
                    people={s.people}
                    fetchingFolder={s.fetchingFolder.length > 0}
                    selected={listedId}
                    open={(id) => void open(id)}
                    prefetch={prefetch}
                    action={(id, a) =>
                      void action(
                        id,
                        a,
                        undefined,
                        s.threads ? s.page.items.find((item) => item.id === id)?.message_ids : undefined,
                      )
                    }
                    loadMore={() => void loadMail(true)}
                    rowMenu={(id, at, rows) => void openRowMenu(id, at, rows)}
                  />
                  <Reader
                    dark={darkNow === 'dark'}
                    opening={openingOther()}
                    outputBusy={s.outputBusy}
                    printEmail={(id, thread) => void printEmails([id], thread)}
                    exportEmail={(id) => void exportEmails([id], 'eml')}
                    email={s.selected}
                    conversation={s.threads ? s.threadEmails : []}
                    people={s.people}
                    inView={selectedInView()}
                    accounts={workspace.accounts}
                    busyIds={s.busyIds}
                    action={(id, a, value) => void action(id, a, value)}
                    reply={(id, forward) => void compose(id, forward)}
                    replyAll={(id) => void compose(id, false, true)}
                    position={readerPosition()}
                    total={s.visibleOrder.length}
                    step={stepReader}
                    notify={notify}
                    connectMailbox={{
                      connected: () => {
                        notify(tr('Account Google collegato', 'Google account connected'));
                        void refresh();
                      },
                      otherWays: () => {
                        navigate('Settings');
                        void tick().then(() => settingsPanel.current?.showSection('Accounts'));
                      },
                    }}
                  />
                </>
              ) : view === 'Settings' ? (
                <Settings
                  ref={settingsPanel}
                  dirty={s.settingsDirty}
                  onDirtyChange={(dirty) => {
                    s.settingsDirty = dirty;
                  }}
                  workspace={workspace}
                  changed={() => void refresh()}
                  appearance={s.appearance}
                  appearanceChanged={(value) => {
                    s.appearance = value;
                  }}
                  notify={notify}
                />
              ) : view === 'Calendar' ? (
                <Calendar
                  notify={notify}
                  openSettings={() => {
                    navigate('Settings');
                    void tick().then(() => settingsPanel.current?.showSection('Accounts'));
                  }}
                />
              ) : view === 'Contacts' ? (
                <Contacts notify={notify} />
              ) : view === 'Meetings' ? (
                <Meetings
                  notify={notify}
                  settings={workspace.settings}
                  openSettings={(section) => {
                    navigate('Settings');
                    void tick().then(() => settingsPanel.current?.showSection(section));
                  }}
                />
              ) : view === 'Assistant' ? (
                <Assistant
                  openMeeting={(source) => {
                    requestMeetingOpen(source);
                    navigate('Meetings');
                  }}
                  openMail={(id) => {
                    navigate('Inbox');
                    void open(id);
                  }}
                />
              ) : view === 'Activity' ? (
                <ActivityPage activities={s.backendActivities} accounts={workspace.accounts} />
              ) : view === 'Drafts' ? (
                <section className="drafts-page">
                  <div className="page-heading">
                    <div>
                      <span className="page-kicker">{tr('Riprendi da dove avevi lasciato', 'Pick up where you left off')}</span>
                      <h1 tabIndex={-1}>{folderLabel('Drafts')}</h1>
                      <p>
                        {workspace.drafts.length}{' '}
                        {workspace.drafts.length === 1
                          ? tr('messaggio salvato', 'saved message')
                          : tr('messaggi salvati', 'saved messages')}
                      </p>
                    </div>
                    {/* The sidebar's New message is the window's one primary action. */}
                    <Button variant="secondary" onClick={() => void compose()}>
                      <Icon name="plus" size={17} />
                      {tr('Nuovo messaggio', 'New message')}
                    </Button>
                  </div>
                  <div className="drafts-list">
                    {workspace.drafts.map((draft) => (
                      <button key={draft.id} onClick={() => openComposer(draft)}>
                        <span className="draft-icon">
                          <Icon name="edit" />
                        </span>
                        <span>
                          <strong>{draft.subject || tr('Messaggio senza oggetto', 'Untitled message')}</strong>
                          <small>
                            {draft.to || tr('Ancora nessun destinatario', 'No recipient yet')}
                            {draft.send_at ? tr(' · Programmata', ' · Scheduled') : ''}
                          </small>
                        </span>
                        <time>{shortDate(draft.updated)}</time>
                        <Icon name="chevron" size={18} />
                      </button>
                    ))}
                    {!workspace.drafts.length && (
                      <div className="empty-list">
                        <Icon name="edit" size={32} />
                        <h3>{tr('Ancora nessuna bozza', 'No drafts yet')}</h3>
                        <p>
                          {tr(
                            'Inizia un messaggio. Le modifiche vengono salvate automaticamente.',
                            'Start a message. Your changes are saved automatically.',
                          )}
                        </p>
                      </div>
                    )}
                  </div>
                </section>
              ) : null}
            </div>
            {s.composers.length > 0 && (
              <Suspense fallback={null}>
                <ComposerDock
                  drafts={s.composers}
                  activeId={s.activeComposerId}
                  onActiveIdChange={(id) => {
                    s.activeComposerId = id;
                  }}
                  windowClosing={s.closingWindow}
                  onWindowClosingChange={(closing) => {
                    s.closingWindow = closing;
                  }}
                  workspace={workspace}
                  close={closeComposer}
                  compose={() => void compose()}
                  changed={() => {
                    v.workspaceSettling = loadWorkspace().catch((e) => notify(errorText(e), true));
                  }}
                  notify={notify}
                  queued={queued}
                  copied={(id, saved) => {
                    closeComposer(id, false);
                    openComposer(saved);
                  }}
                />
              </Suspense>
            )}
          </main>
        </div>
      ) : (
        <main className="startup-state" role="status">
          <span className="brand-mark">
            <svg width="13" height="16" viewBox="0 0 21 25" fill="none" aria-hidden="true">
              <path d="M12 1 2 14h8l-1 10 10-14h-8l1-9Z" fill="currentColor" />
            </svg>
          </span>
          <h1>{tr('Apertura del tuo spazio di lavoro', 'Opening your workspace')}</h1>
          <p>{tr('I tuoi messaggi saranno qui tra un attimo.', 'Your messages will be here in a moment.')}</p>
        </main>
      )}
      {!!workspace?.undo_actions.length && (
        <UndoStack actions={workspace.undo_actions} undo={undoPending} dismiss={dismissPending} />
      )}
      {workspace && <MeetingRecordingIndicator onopen={() => navigate('Meetings')} notify={notify} />}
      {s.leaveTarget && (
        <UnsavedDialog
          save={() => void finishLeaving(true)}
          discard={() => void finishLeaving(false)}
          stay={() => {
            s.leaveTarget = null;
          }}
        />
      )}
      {s.historyUnread && (
        <HistoryUnreadDialog
          accountId={s.historyUnread.accountId}
          account={workspace?.accounts.find((a) => a.id === s.historyUnread?.accountId)?.email ?? ''}
          count={s.historyUnread.count}
          onClose={(message, error) => {
            s.historyUnread = null;
            if (message) notify(message, error);
          }}
        />
      )}
      {s.printPreview && (
        <PrintPreview emails={s.printPreview.emails} title={s.printPreview.title} close={() => (s.printPreview = null)} />
      )}
      <Toaster label={tr('Notifiche', 'Notifications')} closeLabel={tr('Chiudi la notifica', 'Dismiss notification')} />
    </>
  );
}
