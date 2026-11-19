// The Meetings page, a thin shell: it reads the meetings and the calendar once when it opens,
// keeps what is selected and what is searched, and puts the pieces together. The list is
// `meetings/MeetingList`, a row is `MeetingRowItem`, "Registra ora" is `RecordNow`; what the
// rows and the groups are is decided in `lib/meetings-view` (pure, no screen). The right-hand side
// is the detail of the selected meeting (`meetings/MeetingPane` reads it and shows it): the
// shell tells it which meeting, and takes back every change so the open meeting and its row in
// the list stay the same thing.
//
// A page at rest runs nothing (INV-005 of ADR-0007). It is read again only when something happens:
//  - the recording starts or stops, or is another one (the recording indicator is the only poller
//    of the capture and publishes every snapshot on the capture bus): the list and the open
//    meeting are read once, whoever started or stopped it;
//  - a transcription or a summary is waiting or running: one timer of a second (`MeetingFollower`,
//    lib/meetings-follow) reads those meetings (usually one), moves their rows and the open detail
//    along, and when one has no job left it reads the list once. It stops by itself when nothing
//    is in flight, skips the seconds the window is hidden, drops an answer a newer read has
//    already replaced, and waits a few ticks for the job that follows a stop or a transcript;
//  - what the engine can do (`meeting_readiness`) is read when the page opens and again when the
//    settings it was given change; a recording waiting for its transcript says why it cannot
//    start (the rows read it, nothing polls it);
//  - a list read asks for as many meetings as are on screen (the pages "Mostra altre riunioni" added
//    stay), and a row that was refreshed after the read began is not overwritten by it;
//  - the clock of the list: one timeout, armed for the next moment an event of Prossime starts,
//    changes its minute or ends (a computed deadline, not polling), moves `now` and nothing else;
//  - the Assistant or the indicator ask for a meeting (`requestMeetingOpen`): it is opened, on the
//    transcript at that line or on the summary.
//
// The page state is a deep-reactive store (`useReactive`): the reads below continue after awaits,
// in timers and in listeners, and must see the state as it is then, as the Svelte page did. The
// bookkeeping that is never drawn (tickets, stamps, timers) lives in a ref.
import { useEffect, useMemo, useRef, useState } from 'react';
import { call, errorText } from '../lib/api';
import { tr } from '../lib/i18n';
import {
  lastCapture,
  meetingCall,
  meetingMetadata,
  publishCapture,
  subscribeCapture,
  subscribeMeetingOpen,
  takeMeetingOpen,
  type CaptureSnapshot,
  type MeetingDetail,
  type MeetingHit,
  type MeetingPage,
  type MeetingReadiness,
} from '../lib/meetings';
import { keepFocusInDetail } from '../lib/meetings-focus';
import { MeetingFollower } from '../lib/meetings-follow';
import {
  captureMark,
  groupRows,
  hitTarget,
  mergeMeetingRows,
  nextClockBoundary,
  processingIds,
  stoppedMeeting,
  usableHits,
  type MeetingRow,
} from '../lib/meetings-view';
import { useReactive } from '../lib/store';
import { tick } from '../lib/tick';
import type { CalendarEvent, Settings } from '../lib/types';
import MeetingList from './meetings/MeetingList';
import MeetingPane, { type MeetingPaneHandle } from './meetings/MeetingPane';
import RecordNow from './meetings/RecordNow';
import './Meetings.css';

export type MeetingsProps = {
  notify: (message: string, error?: boolean) => void;
  settings: Settings;
  /** Opens Settings on a section: the links of the detail ("Impostazioni › Riunioni"). */
  openSettings?: (section: 'Meetings' | 'Assistant') => void;
};

type Tab = 'summary' | 'transcript';

/** Meetings per request; "Mostra altre riunioni" asks for the next ones. */
const PAGE = 50;
/** Wait for the user to stop typing before the search goes to the backend. */
const SEARCH_WAIT_MS = 250;
/** The most meetings the backend gives in one request. */
const MAX_PAGE = 100;
/** The longest a timeout can wait (a signed 32-bit number of milliseconds). */
const MAX_TIMEOUT_MS = 2 ** 31 - 1;
/** The clock timer never wakes sooner than this, whatever the arithmetic says. */
const CLOCK_MIN_MS = 50;
const unixNow = () => Math.floor(Date.now() / 1000);

export default function Meetings(props: MeetingsProps) {
  const { notify, settings } = props;
  // The callbacks below outlive the render that made them (timers, listeners, the follower): they
  // read the props through this ref.
  const latest = useRef(props);
  latest.current = props;

  const s = useReactive(() => ({
    details: [] as MeetingDetail[],
    events: [] as CalendarEvent[],
    nextOffset: null as number | null,
    now: unixNow(),
    status: 'loading' as 'loading' | 'ready' | 'failed',
    live: lastCapture(),
    /** What is open on the right: the row, and (from a search result or a link) where in the meeting. */
    selectedKey: null as string | null,
    target: null as { key: string; tab: Tab; segmentId: string | null } | null,
    openedHit: null as MeetingHit | null,
    /** The tab of the open meeting, and a counter that makes the pane read it again on every pick. */
    tab: 'summary' as Tab,
    reload: 0,
    /** The open meeting as the pane last read it: it keeps its row even when the list does not reach it. */
    opened: null as MeetingDetail | null,
    query: '',
    /** `null`: no results to show (nothing searched yet, or the last search failed). */
    hits: null as MeetingHit[] | null,
    /** The words `hits` were found for: while the user types on, the results stay and keep these. */
    hitsQuery: '',
    searchFailed: false,
    moreBusy: false,
    joining: false,
    /** The meeting whose "Riprova" is on its way (one at a time). */
    retrying: null as number | null,
    /** Why the engine cannot transcribe now, from Settings; `null` when it can (or is not known yet). */
    blocker: null as string | null,
    clockBeat: 0,
  }));
  const pane = useRef<MeetingPaneHandle>(null);
  const m = useRef({
    /**
     * Counts every time the page takes meetings from somewhere else (a list read, the pane's read, a
     * delete). A read of the meetings in flight that started before it is older than what is on
     * screen, and is dropped.
     */
    dataStamp: 0,
    /** `dataStamp` when each meeting's row was last refreshed by a read of that meeting. */
    rowStamp: new Map<number, number>(),
    /** How many meetings the pages read so far hold: a list read asks for as many. */
    depth: 0,
    loadTicket: 0,
    /** A list read is on its way (the newest one). */
    loadRunning: false,
    searchTimer: undefined as ReturnType<typeof setTimeout> | undefined,
    searchTicket: 0,
    readinessTicket: 0,
    /** What the page last knew of the recording: whether one is going on and which meeting it is. */
    seenCapture: lastCapture(),
  }).current;

  const selectedIdOf = () => (s.selectedKey?.startsWith('m:') ? Number(s.selectedKey.slice(2)) : null);
  const selectedId = selectedIdOf();
  const rows = useMemo(
    () => mergeMeetingRows({ meetings: s.details, events: s.events, now: s.now }),
    [s.details, s.events, s.now],
  );
  const groups = useMemo(() => groupRows(rows), [rows]);
  /** The line a search result pointed at, while its transcript is the one being read. */
  const highlight =
    s.target && s.target.key === s.selectedKey && s.tab === 'transcript' ? s.target.segmentId : null;

  const touch = (id: number) => m.rowStamp.set(id, ++m.dataStamp);
  /** The open meeting keeps its row: it may be further back than the pages that were read. */
  function withOpen(items: MeetingDetail[]): MeetingDetail[] {
    const open = s.opened;
    if (!open || open.deleted || open.meeting.id !== selectedIdOf()) return items;
    if (items.some((item) => item.meeting.id === open.meeting.id)) return items;
    return [...items, meetingMetadata(open)];
  }
  /**
   * The meetings and the calendar, read again from the start but down to the depth already on
   * screen: pages added with "Mostra altre riunioni" stay, and so does the scroll. A meeting whose
   * row was refreshed after the read began keeps that row: the read may have been answered before
   * the job it shows.
   */
  async function load() {
    const ticket = ++m.loadTicket;
    m.loadRunning = true;
    const began = m.dataStamp;
    const want = Math.max(PAGE, m.depth);
    try {
      // The calendar is a bonus: without it the page still lists the recordings.
      const calendarRead = call<CalendarEvent[]>('list_events').catch(() => [] as CalendarEvent[]);
      const items: MeetingDetail[] = [];
      // A meeting added between two requests moves the others down a place: one can come twice.
      const seen = new Set<number>();
      let next: number | null = 0;
      while (next !== null && items.length < want) {
        const page: MeetingPage = await meetingCall<MeetingPage>('meeting_list_page', {
          offset: next,
          limit: Math.min(MAX_PAGE, want - items.length),
          fromDate: null,
          toDate: null,
          state: null,
        });
        if (ticket !== m.loadTicket) return;
        for (const item of page.items)
          if (!seen.has(item.meeting.id)) {
            seen.add(item.meeting.id);
            items.push(item);
          }
        next = page.items.length ? page.next_offset : null;
      }
      const calendar = await calendarRead;
      if (ticket !== m.loadTicket) return;
      const shown = new Map(s.details.map((detail) => [detail.meeting.id, detail]));
      const fresher = (item: MeetingDetail) =>
        (m.rowStamp.get(item.meeting.id) ?? 0) > began ? (shown.get(item.meeting.id) ?? item) : item;
      m.dataStamp++;
      s.details = withOpen(items.map(fresher));
      m.depth = items.length;
      s.events = calendar;
      s.nextOffset = next;
      s.now = unixNow();
      s.status = 'ready';
    } catch (error) {
      if (ticket !== m.loadTicket) return;
      // A list already on screen stays (a reload that fails says so and changes nothing).
      if (s.status === 'loading') s.status = 'failed';
      latest.current.notify(errorText(error), true);
    } finally {
      if (ticket === m.loadTicket) m.loadRunning = false;
    }
  }

  async function showMore() {
    if (s.nextOffset === null || s.moreBusy) return;
    // A read of the list that began earlier and ends later must not erase the page added here: it
    // is dropped (the ticket), and read again once this page is in, down to the depth on screen.
    const interrupted = m.loadRunning;
    m.loadRunning = false;
    const ticket = ++m.loadTicket;
    s.moreBusy = true;
    try {
      const page = await meetingCall<MeetingPage>('meeting_list_page', {
        offset: s.nextOffset,
        limit: PAGE,
        fromDate: null,
        toDate: null,
        state: null,
      });
      if (ticket !== m.loadTicket) return;
      const known = new Set(s.details.map((detail) => detail.meeting.id));
      s.details = [...s.details, ...page.items.filter((detail) => !known.has(detail.meeting.id))];
      m.depth += page.items.length;
      s.nextOffset = page.next_offset;
    } catch (error) {
      latest.current.notify(errorText(error), true);
    } finally {
      s.moreBusy = false;
      if (interrupted && ticket === m.loadTicket) void load();
    }
  }

  function search(value: string) {
    s.query = value;
    clearTimeout(m.searchTimer);
    const ticket = ++m.searchTicket;
    s.searchFailed = false;
    const text = value.trim();
    if (!text) {
      s.hits = null;
      s.hitsQuery = '';
      return;
    }
    // The results of the words typed before stay on screen until the new ones are back: the list
    // does not flash to "Cerco…" at every key.
    m.searchTimer = setTimeout(async () => {
      try {
        const found = await meetingCall<MeetingHit[]>('meeting_search', { query: text });
        if (ticket !== m.searchTicket) return;
        s.hits = usableHits(found);
        s.hitsQuery = text;
      } catch (error) {
        if (ticket !== m.searchTicket) return;
        s.hits = null;
        s.hitsQuery = '';
        s.searchFailed = true;
        latest.current.notify(errorText(error), true);
      }
    }, SEARCH_WAIT_MS);
  }

  /** A meeting read in full: its row follows, and the open one is kept as it was read. */
  function applyRow(next: MeetingDetail) {
    const open = selectedIdOf();
    if (next.meeting.id === open) s.opened = next;
    const row = meetingMetadata(next);
    if (s.details.some((item) => item.meeting.id === next.meeting.id))
      s.details = s.details.map((item) => (item.meeting.id === next.meeting.id ? row : item));
    // The open meeting is not in the pages read (a link from outside, an old one): it gets its row.
    else if (next.meeting.id === open && !next.deleted) s.details = [...s.details, row];
  }
  /** A meeting changed (a command, a rename, a job, a new read): its row follows. */
  function refreshRow(next: MeetingDetail) {
    touch(next.meeting.id);
    applyRow(next);
  }
  /** Deleted from the detail: nothing is selected, the list is read again, a stale result goes. */
  async function deleted() {
    m.dataStamp++;
    s.selectedKey = null;
    s.target = null;
    s.openedHit = null;
    s.opened = null;
    s.hits = null;
    s.hitsQuery = '';
    if (s.query) search(s.query);
    void keepFocusInDetail();
    await load();
  }

  /** Shows a meeting on the right: on the summary, or where a result or a link pointed at. */
  function show(meetingId: number, hit: MeetingHit | null = null) {
    const key = `m:${meetingId}`;
    if (s.selectedKey !== key) s.opened = null;
    s.selectedKey = key;
    s.target = hit ? { key, ...hitTarget(hit) } : null;
    s.tab = s.target?.tab ?? 'summary';
    s.reload++;
  }
  /** An event has nothing to open: only a recording does. */
  function select(row: MeetingRow) {
    if (!row.openable || row.meetingId === null) return;
    s.openedHit = null;
    show(row.meetingId);
  }
  /** A result of the search: the meeting, on the transcript at that line or on the summary. */
  function openHit(hit: MeetingHit) {
    s.openedHit = hit;
    show(hit.meeting_id, hit);
  }
  /**
   * The Assistant or the recording indicator asked for a meeting. The search gives way to the list
   * so the row can be seen. The pane reads the meeting by itself; if the pages read did not reach
   * it, its row appears with that read.
   */
  function openFromOutside(hit: MeetingHit) {
    s.openedHit = null;
    if (s.query) search('');
    show(hit.meeting_id, hit);
  }
  function openRequested() {
    const hit = takeMeetingOpen();
    if (hit) openFromOutside(hit);
  }

  /** The same call as the Calendar's "Join and record": the desktop opens the link and records. */
  async function join(row: MeetingRow) {
    if (!row.meetingLink || s.joining) return;
    s.joining = true;
    try {
      await call('event_join_record', {
        url: row.meetingLink,
        key: `event::${row.eventId ?? ''}:${row.start}:${crypto.randomUUID()}`,
        title: row.title,
        eventId: row.eventId,
      });
      latest.current.notify(tr('Registrazione avviata', 'Recording started'));
      await load();
    } catch (error) {
      latest.current.notify(errorText(error), true);
    } finally {
      s.joining = false;
    }
  }

  /**
   * A snapshot of the recording. When it is another recording, the list and the open meeting are
   * read, once; and a meeting whose recording ended is followed for a moment, because its
   * transcription is queued just after the stop (the snapshot can come before the job exists).
   */
  function heardCapture(snapshot: CaptureSnapshot) {
    s.live = snapshot;
    if (captureMark(snapshot) === captureMark(m.seenCapture)) return;
    const stopped = stoppedMeeting(m.seenCapture, snapshot);
    m.seenCapture = snapshot;
    void load();
    if (selectedIdOf() !== null) s.reload++;
    if (stopped !== null) follower.settle(stopped);
  }

  /** "Registra ora" started a recording: show it, open it, and leave the search so it can be seen. */
  async function started(detail: MeetingDetail) {
    // The page started it itself: the list is read below, not once more for the snapshot.
    m.seenCapture = detail.capture;
    publishCapture(detail.capture);
    show(detail.meeting.id);
    if (s.query) search('');
    await load();
  }

  // ─── What the engine can do ─────────────────────────────────────────────────────────────────
  // Read when the page opens and again when the settings it was given change (Settings is another
  // page: coming back opens this one again). A failed read leaves what the page knew.
  async function readReadiness() {
    const ticket = ++m.readinessTicket;
    try {
      const ready = await meetingCall<MeetingReadiness>('meeting_readiness');
      if (ticket === m.readinessTicket) s.blocker = ready.transcription_blocker;
    } catch {
      /* The rows say nothing about the engine until it is known. */
    }
  }
  useEffect(() => {
    void readReadiness();
    return () => {
      m.readinessTicket++;
    };
  }, [settings]);

  /** A meeting read in full by someone else: the open one goes through the pane, any other is a row. */
  function applyFull(full: MeetingDetail) {
    if (full.meeting.id === selectedIdOf() && pane.current) pane.current.accept(full);
    else refreshRow(full);
  }
  /**
   * "Riprova" on a row: a new transcription with the engine of now (not the failed job run again
   * with the options it failed with). One at a time; the button stays enabled while it waits so it
   * keeps the focus, and when the row moves to In elaborazione the focus follows it.
   */
  async function transcribeRow(row: MeetingRow) {
    const id = row.meetingId;
    if (id === null || s.retrying !== null) return;
    s.retrying = id;
    try {
      applyFull(await meetingCall<MeetingDetail>('meeting_transcribe_default', { id }));
      await tick();
      document.querySelector<HTMLElement>(`.meetings-page [data-row-key="m:${id}"] .item-main`)?.focus();
    } catch (error) {
      latest.current.notify(errorText(error), true);
      // The row may be out of date (the audio was deleted meanwhile): read the list once.
      void load();
    } finally {
      s.retrying = null;
    }
  }

  // ─── The meetings in flight ─────────────────────────────────────────────────────────────────
  // Made once; its callbacks reach the page through `actions`, which every render refreshes.
  const actions = useRef({ applyFull, load });
  actions.current = { applyFull, load };
  const [follower] = useState(
    () =>
      new MeetingFollower({
        read: (id) => meetingCall<MeetingDetail>('meeting_read', { id }),
        // The open meeting goes through the pane, which shows it and hands it back for its row.
        apply: (detail) => actions.current.applyFull(detail),
        reload: () => actions.current.load(),
        stamp: () => m.dataStamp,
        giveUp: (error) => latest.current.notify(errorText(error), true),
        hidden: () => document.hidden,
      }),
  );
  useEffect(() => {
    follower.sync(processingIds(s.opened ? [...s.details, s.opened] : s.details));
  }, [follower, s.details, s.opened]);

  // ─── The clock of the list ──────────────────────────────────────────────────────────────────
  // "Iniziata da N min" and the end of an event move with the time of day, which only `load()`
  // sets. One timeout waits for the next moment a row changes by itself (`nextClockBoundary`), only
  // updates `now`, and is armed again by the rows it changed; with nothing to wait for there is no
  // timer. It asks the backend for nothing.
  useEffect(() => {
    const boundary = nextClockBoundary(rows, s.now);
    if (boundary === null) return;
    const wait = Math.min(MAX_TIMEOUT_MS, Math.max(CLOCK_MIN_MS, boundary * 1000 - Date.now()));
    const timer = setTimeout(() => {
      s.now = unixNow();
      s.clockBeat++;
    }, wait);
    return () => clearTimeout(timer);
  }, [rows, s, s.now, s.clockBeat]);

  // The bus listeners are added once and call the newest handlers.
  const heardCaptureRef = useRef(heardCapture);
  heardCaptureRef.current = heardCapture;
  const openRequestedRef = useRef(openRequested);
  openRequestedRef.current = openRequested;
  useEffect(() => {
    void actions.current.load();
    s.live = lastCapture();
    m.seenCapture = s.live;
    const stop = subscribeCapture((snapshot) => heardCaptureRef.current(snapshot));
    const stopOpen = subscribeMeetingOpen(() => openRequestedRef.current());
    // A meeting asked for before the page was there (the Assistant requests it, then navigates).
    openRequestedRef.current();
    return () => {
      stop();
      stopOpen();
      follower.dispose();
      clearTimeout(m.searchTimer);
      m.searchTicket++;
      m.loadTicket++;
      m.dataStamp++;
    };
  }, [follower, m, s]);

  return (
    <section className="meetings-page">
      <MeetingList
        groups={groups}
        selectedKey={s.selectedKey}
        query={s.query}
        hits={s.hits}
        hitsQuery={s.hitsQuery}
        live={s.live}
        now={s.now}
        status={s.status}
        searchFailed={s.searchFailed}
        openedHit={s.openedHit}
        moreBusy={s.moreBusy}
        joinBusy={s.joining}
        blocker={s.blocker}
        transcribeBusy={s.retrying !== null}
        onsettings={() => latest.current.openSettings?.('Meetings')}
        ontranscribe={(row) => void transcribeRow(row)}
        onselect={select}
        onjoin={(row) => void join(row)}
        onquery={search}
        onopenhit={openHit}
        onmore={s.nextOffset === null ? undefined : () => void showMore()}
        onretry={() => {
          s.status = 'loading';
          void load();
        }}
        actions={<RecordNow settings={settings} live={s.live} notify={notify} onstarted={(detail) => void started(detail)} />}
      />
      {/* The data attributes say what the page holds for the detail (and what a search result asked of it). */}
      <section
        className="detail-pane"
        tabIndex={-1}
        aria-label={tr('Dettaglio riunione', 'Meeting detail')}
        data-key={s.selectedKey ?? ''}
        data-tab={s.target?.tab ?? ''}
        data-segment={s.target?.segmentId ?? ''}
      >
        <MeetingPane
          ref={pane}
          meetingId={selectedId}
          reload={s.reload}
          tab={s.tab}
          highlight={highlight}
          events={s.events}
          live={s.live}
          notify={notify}
          ontab={(next) => {
            s.tab = next;
          }}
          onchange={refreshRow}
          ondeleted={() => void deleted()}
          onsettings={(section) => latest.current.openSettings?.(section)}
        />
      </section>
    </section>
  );
}
