import { useCallback, useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react';
import { calendarWords as dateWords } from '../lib/calendar-words';
import SidePanel from '../ds/primitives/SidePanel';
import Button from '../ds/primitives/Button';
import Select from '../ds/primitives/Select';
import Checkbox from '../ds/primitives/Checkbox';
import DateField from '../ds/primitives/DateField';
import Icon from '../ds/primitives/Icon';
import { call, desktop, errorText } from '../lib/api';
import {
  calendarArgs,
  calendarCall,
  calendarEvents,
  calendarKey,
  eventDateTime,
  eventKey,
  loadCalendarChoices,
  localCalendar,
  syncGoogleCalendars,
  type CalendarChoice,
  type CalendarListError,
  type ProviderEvent,
  type VisibleCalendarEvent,
} from '../lib/calendar';
import { LoadGate } from '../lib/load-gate';
import { calendarFormChanges, calendarTimeChanges, type CalendarForm } from '../lib/calendar-edit';
import { folderLabel, tr } from '../lib/i18n';
import { readPreference, savePreference } from '../lib/preferences';
import {
  regionalLocale,
  calendarDayLabel,
  dateInput,
  dateKey,
  firstWeekday,
  formatDateTime,
  parseLocalDateTime,
  regionalTimeZone,
} from '../lib/regional';
import { useReactive } from '../lib/store';
import { tick } from '../lib/tick';
import { useDirtyCloseGuard } from '../lib/use-dirty-close-guard';
import { cn } from '../lib/utils';
import './Calendar.css';
import { Input, Textarea } from '../ui/Input';

export type CalendarProps = {
  notify: (message: string, error?: boolean) => void;
  openSettings?: () => void;
};

type Account = { id: number; email: string; kind: string; provider_id?: string };
type AccountState = 'loading' | 'ready' | 'connecting' | 'consent' | 'signin' | 'error';
type View = 'day' | 'week' | 'month';
type Drag = { day: string; anchor: number; from: number; until: number; moved: boolean };

const PALETTE = ['#416adb', '#d9534f', '#2f9e6e', '#b7791f', '#8e5bd1', '#0e8fa8', '#d6568f', '#6b7280'];
const HOUR_HEIGHT = 44;
const DAY_MINUTES = 24 * 60;
const DAY_MS = 86_400_000;
const SNAP = 15;
const hours = Array.from({ length: 24 }, (_, hour) => hour);
const VIEWS: [View, string, string][] = [
  ['day', 'Giorno', 'Day'],
  ['week', 'Settimana', 'Week'],
  ['month', 'Mese', 'Month'],
];
const RECURRENCE: [string, string, string][] = [
  ['', 'Non si ripete', 'Does not repeat'],
  ['RRULE:FREQ=DAILY', 'Ogni giorno', 'Every day'],
  ['RRULE:FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR', 'Ogni giorno feriale (lun–ven)', 'Every weekday (Mon–Fri)'],
  ['RRULE:FREQ=WEEKLY', 'Ogni settimana', 'Every week'],
  ['RRULE:FREQ=MONTHLY', 'Ogni mese', 'Every month'],
  ['RRULE:FREQ=YEARLY', 'Ogni anno', 'Every year'],
];
// A Select cannot choose the empty value, so "does not repeat" travels as this word and is '' in the form.
const NO_REPEAT = 'none';

// Each calendar keeps the colour it was given the first time it appeared: the next colour map, or null.
function withColors(current: Record<string, number>, items: CalendarChoice[]): Record<string, number> | null {
  const next = { ...current };
  let changed = false;
  for (const item of items) {
    const key = calendarKey(item);
    if (typeof next[key] === 'number') continue;
    const used = PALETTE.map((_, i) => Object.values(next).filter((v) => v === i).length);
    next[key] = used.indexOf(Math.min(...used));
    changed = true;
  }
  return changed ? next : null;
}

const addDays = (day: string, count: number) => {
  const d = new Date(`${day}T12:00:00Z`);
  d.setUTCDate(d.getUTCDate() + count);
  return d.toISOString().slice(0, 10);
};
const weekStart = (day: string) => addDays(day, -((new Date(`${day}T12:00:00Z`).getUTCDay() - firstWeekday() + 7) % 7));

// Wall-clock arithmetic on the editor's "YYYY-MM-DD[THH:MM]" strings.
const wall = (value: string) => Date.parse(`${value.length === 10 ? `${value}T00:00` : value}:00Z`);
const fromWall = (ms: number, dateOnly: boolean) => new Date(ms).toISOString().slice(0, dateOnly ? 10 : 16);
const pad = (n: number) => String(n).padStart(2, '0');
const minutesOf = (value: string) => Number(value.slice(11, 13)) * 60 + Number(value.slice(14, 16));
function slotValue(day: string, minute: number) {
  if (minute >= DAY_MINUTES) return `${fromWall(wall(day) + DAY_MS, true)}T00:00`;
  return `${day}T${pad(Math.floor(minute / 60))}:${pad(minute % 60)}`;
}
function clockLabel(minute: number) {
  return new Intl.DateTimeFormat(regionalLocale(), { hour: 'numeric', minute: '2-digit', timeZone: 'UTC' }).format(
    Date.UTC(2026, 0, 1, 0, minute),
  );
}
function hourLabel(hour: number) {
  return new Intl.DateTimeFormat(regionalLocale(), { hour: 'numeric', timeZone: 'UTC' }).format(Date.UTC(2026, 0, 1, hour));
}
function minuteOfDay(timestamp: number) {
  const match = formatDateTime(timestamp, { hour: '2-digit', minute: '2-digit', hourCycle: 'h23' }).match(/(\d+)\D+(\d+)/);
  return match ? Number(match[1]) * 60 + Number(match[2]) : 0;
}

const isAllDay = (event: VisibleCalendarEvent) => Boolean(event.raw?.start.date);
function eventStyle(item: { from: number; until: number; column: number; columns: number }) {
  const top = (item.from / 60) * HOUR_HEIGHT;
  const height = Math.max(((item.until - item.from) / 60) * HOUR_HEIGHT - 2, 22);
  const width = 100 / item.columns;
  // The title takes the lines the block has room for (16px each), above the 14px time line and the padding.
  const titleLines = Math.max(1, Math.floor((height - 20) / 16));
  return {
    top: `${top}px`,
    height: `${height}px`,
    left: `calc(${item.column * width}% + 2px)`,
    width: `calc(${width}% - 4px)`,
    '--title-lines': titleLines,
  } as React.CSSProperties;
}
const blockStyle = (from: number, until: number) => ({
  top: `${(from / 60) * HOUR_HEIGHT}px`,
  height: `${Math.max(((until - from) / 60) * HOUR_HEIGHT - 2, 22)}px`,
});
const calColor = (color: string) => ({ '--cal-color': color }) as React.CSSProperties;

export default function Calendar({ notify, openSettings }: CalendarProps) {
  // Timers and async continuations read the newest callback, as Svelte's live props did.
  const notifyRef = useRef(notify);
  notifyRef.current = notify;
  const say = (message: string, error?: boolean) => notifyRef.current(message, error);

  const s = useReactive(() => {
    // Calendars are shown together; the person hides the ones they don't want to see.
    const saved = (() => {
      try {
        return JSON.parse(readPreference('calendar-view') || '{}') ?? {};
      } catch {
        return {};
      }
    })();
    const colors: Record<string, number> = saved.colors && typeof saved.colors === 'object' ? { ...saved.colors } : {};
    return {
      // Calendar sources
      events: [] as VisibleCalendarEvent[],
      calendars: [localCalendar] as CalendarChoice[],
      calendarErrors: [] as CalendarListError[],
      calendarsLoaded: !desktop,
      hidden: (Array.isArray(saved.hidden) ? saved.hidden.filter((k: unknown) => typeof k === 'string') : []) as string[],
      colorIndex: withColors(colors, [localCalendar]) ?? colors,
      accounts: [] as Account[],
      connectingId: null as number | null,
      syncing: false,
      syncedAt: null as number | null,
      // Event editor
      form: false,
      busy: false,
      selected: null as VisibleCalendarEvent | null,
      title: '',
      start: '',
      end: '',
      allDay: false,
      location: '',
      attendees: '',
      link: '',
      description: '',
      recurrence: '',
      scope: 'instance',
      online: false,
      sendUpdates: 'all',
      targetKey: calendarKey(localCalendar),
      confirmingDelete: false,
      sheetError: '',
      initialForm: null as CalendarForm | null,
      // Week grid
      offset: 0,
      regionalRevision: 0,
      view: (saved.view === 'day' || saved.view === 'month' ? saved.view : 'week') as View,
      now: Date.now(),
      monthHeight: 0,
      // Click or drag on an empty slot to create an event there.
      drag: null as Drag | null,
      hover: null as { day: string; minute: number } | null,
    };
  });
  const [loads] = useState(() => new LoadGate());
  const creationKey = useRef('');
  const editGeneration = useRef(0);
  const titleInput = useRef<HTMLInputElement>(null);
  const newEventButton = useRef<HTMLButtonElement>(null);

  const formValues = (): CalendarForm => ({
    title: s.title,
    start: s.start,
    end: s.end,
    allDay: s.allDay,
    location: s.location,
    description: s.description,
    attendees: s.attendees,
    recurrence: s.recurrence,
    link: s.link,
  });
  const colorOf = (calendar?: CalendarChoice) =>
    PALETTE[(calendar ? (s.colorIndex[calendarKey(calendar)] ?? 0) : 0) % PALETTE.length];
  function assignColors(items: CalendarChoice[]) {
    const next = withColors(s.colorIndex, items);
    if (next) s.colorIndex = next;
  }

  // Everything derived from the state, computed afresh wherever it is needed (render, timers,
  // continuations after an await), so nothing reads a stale snapshot.
  function derive() {
    const shown = s.calendars.filter((c) => !s.hidden.includes(calendarKey(c)));
    const allShown = shown.length === s.calendars.length;
    const hasGoogle = shown.some((c) => c.provider_id !== 'local');
    const defaultTarget =
      shown.length === 1 && shown[0].writable
        ? shown[0]
        : shown.find((c) => c.writable && c.provider_id === 'local') || shown.find((c) => c.writable) || localCalendar;
    const todayKey = dateKey(s.now / 1000);
    const nowMinute = minuteOfDay(s.now / 1000);
    // First day of the month on screen (month view) or the day the offset points to.
    const t = new Date(`${todayKey}T12:00:00Z`);
    const monthStart = new Date(Date.UTC(t.getUTCFullYear(), t.getUTCMonth() + s.offset, 1, 12)).toISOString().slice(0, 10);
    // Columns of the time grid: one day, or the seven days of a week.
    const days =
      s.view === 'day'
        ? [addDays(todayKey, s.offset)]
        : (() => {
            const first = weekStart(addDays(todayKey, s.offset * 7));
            return Array.from({ length: 7 }, (_, i) => addDays(first, i));
          })();
    // Cells of the month grid: six full weeks, so the grid never changes height.
    const monthFirst = weekStart(monthStart);
    const monthDays = Array.from({ length: 42 }, (_, i) => addDays(monthFirst, i));
    const rangeDays = s.view === 'month' ? monthDays : days;
    const focusDay = s.view === 'month' ? monthStart : days[0];

    const writableCalendars = s.calendars.filter((c) => c.writable);
    // An existing event stays in its calendar; a new one goes where the person picks.
    const selectedCalendar = s.selected?.calendar ?? defaultTarget;
    const formCalendar = s.selected
      ? selectedCalendar
      : writableCalendars.find((c) => calendarKey(c) === s.targetKey) || localCalendar;
    const formLocal = formCalendar.id === 'local';
    const readOnly = Boolean(s.selected) && !selectedCalendar.writable;
    const isInstance = Boolean(s.selected?.raw?.recurringEventId);
    const isRecurring = isInstance || Boolean(s.selected?.raw?.recurrence?.length);
    const dirty = s.form && s.initialForm !== null && JSON.stringify(formValues()) !== JSON.stringify(s.initialForm);
    const timeError = s.form && s.start !== '' && s.end !== '' && !(wall(s.end) > wall(s.start));
    return {
      shown,
      allShown,
      hasGoogle,
      defaultTarget,
      todayKey,
      nowMinute,
      monthStart,
      days,
      monthDays,
      rangeDays,
      focusDay,
      writableCalendars,
      selectedCalendar,
      formCalendar,
      formLocal,
      readOnly,
      isInstance,
      isRecurring,
      dirty,
      timeError,
    };
  }
  const D = derive();
  const {
    shown,
    allShown,
    hasGoogle,
    todayKey,
    nowMinute,
    monthStart,
    days,
    monthDays,
    rangeDays,
    writableCalendars,
    formCalendar,
    formLocal,
    readOnly,
    isInstance,
    isRecurring,
    timeError,
  } = D;
  const weekdayHeads = monthDays.slice(0, 7);
  const weekTitle =
    s.view === 'month'
      ? calendarDayLabel(monthStart, { month: 'long', year: 'numeric' })
      : s.view === 'day'
        ? calendarDayLabel(days[0], { weekday: 'long', day: 'numeric', month: 'long', year: 'numeric' })
        : days[0].slice(0, 7) === days[6].slice(0, 7)
          ? calendarDayLabel(days[0], { month: 'long', year: 'numeric' })
          : `${calendarDayLabel(days[0], { month: 'short' })} – ${calendarDayLabel(days[6], { month: 'short', year: 'numeric' })}`;
  const stepLabel = (next: boolean) =>
    s.view === 'day'
      ? next
        ? tr('Giorno successivo', 'Next day')
        : tr('Giorno precedente', 'Previous day')
      : s.view === 'month'
        ? next
          ? tr('Mese successivo', 'Next month')
          : tr('Mese precedente', 'Previous month')
        : next
          ? tr('Settimana successiva', 'Next week')
          : tr('Settimana precedente', 'Previous week');

  function setView(next: View) {
    if (next === s.view) return;
    const { rangeDays, todayKey, focusDay } = derive();
    // Keep the day the person is looking at: today if it is on screen, otherwise the first visible day.
    const anchor = rangeDays.includes(todayKey) ? todayKey : focusDay;
    s.drag = null;
    s.hover = null;
    s.view = next;
    const diff = Math.round((Date.parse(`${anchor}T12:00:00Z`) - Date.parse(`${todayKey}T12:00:00Z`)) / DAY_MS);
    if (next === 'day') s.offset = diff;
    else if (next === 'week')
      s.offset = Math.floor((diff + ((new Date(`${todayKey}T12:00:00Z`).getUTCDay() - firstWeekday() + 7) % 7)) / 7);
    else {
      const a = new Date(`${anchor}T12:00:00Z`);
      const t = new Date(`${todayKey}T12:00:00Z`);
      s.offset = (a.getUTCFullYear() - t.getUTCFullYear()) * 12 + a.getUTCMonth() - t.getUTCMonth();
    }
  }
  function openDay(day: string) {
    const { todayKey } = derive();
    const diff = Math.round((Date.parse(`${day}T12:00:00Z`) - Date.parse(`${todayKey}T12:00:00Z`)) / DAY_MS);
    s.view = 'day';
    s.offset = diff;
  }

  // Derived editor state
  const durationLabel = (() => {
    const minutes = Math.round((wall(s.end) - wall(s.start)) / 60000);
    if (!(minutes > 0)) return '';
    if (s.allDay) {
      const count = Math.round(minutes / DAY_MINUTES);
      return count === 1 ? tr('1 giorno', '1 day') : tr(`${count} giorni`, `${count} days`);
    }
    return [Math.floor(minutes / 60) ? `${Math.floor(minutes / 60)} h` : '', minutes % 60 ? `${minutes % 60} min` : '']
      .filter(Boolean)
      .join(' ');
  })();
  const guestCount = s.attendees
    .split(',')
    .map((part) => part.trim())
    .filter(Boolean).length;
  const recurrenceItems = [
    ...RECURRENCE.map(([value, it, en]) => ({ value: value || NO_REPEAT, label: tr(it, en) })),
    ...(s.recurrence && !RECURRENCE.some(([value]) => value === s.recurrence)
      ? [{ value: s.recurrence, label: tr('Personalizzata', 'Custom') }]
      : []),
  ];
  const updateItems = [
    { value: 'all', label: tr('Tutti gli invitati', 'All guests') },
    { value: 'externalOnly', label: tr('Solo invitati esterni', 'External guests only') },
    { value: 'none', label: tr('Nessuno', 'Nobody') },
  ];
  const scopeItems = [
    { value: 'instance', label: tr('Solo questo evento', 'This event only') },
    { value: 'following', label: tr('Questo e i successivi', 'This and following') },
    { value: 'series', label: tr('Tutta la serie', 'All events in the series') },
  ];
  // The date fields write dates the way the regional format does; the calendar's own words follow the interface language.
  const dateLocale = regionalLocale();
  const calendarWords = dateWords();

  // Calendar list, grouped the way people think about it: this computer, then each Google account.
  const localCalendars = (() => {
    const own = s.calendars.filter((c) => c.provider_id === 'local');
    return own.length ? own : [localCalendar];
  })();
  const accountGroups = (() => {
    const ids = new Set<number>(s.accounts.map((a) => a.id));
    for (const c of s.calendars) if (c.account_id !== undefined) ids.add(c.account_id);
    for (const e of s.calendarErrors) if (e.account_id !== undefined) ids.add(e.account_id);
    return [...ids].map((id) => {
      const email = s.accounts.find((a) => a.id === id)?.email || `Google · ${id}`;
      const items = s.calendars.filter((c) => c.account_id === id);
      const error = s.calendarErrors.find((e) => e.account_id === id);
      const state: AccountState =
        s.connectingId === id
          ? 'connecting'
          : items.length
            ? 'ready'
            : error?.code === 'sign_in_required'
              ? 'signin'
              : error && error.code !== 'calendar_consent_required'
                ? 'error'
                : s.calendarsLoaded
                  ? 'consent'
                  : 'loading';
      return { id, email, items, error, state };
    });
  })();
  const looseErrors = s.calendarErrors.filter((e) => e.account_id === undefined);
  const accountEmail = (calendar: CalendarChoice) => s.accounts.find((a) => a.id === calendar.account_id)?.email;
  const calendarItems = writableCalendars.map((choice) => ({
    value: calendarKey(choice),
    label: `${choice.name}${
      choice.provider_id === 'local'
        ? ` · ${tr('questo computer', 'this computer')}`
        : accountEmail(choice) && accountEmail(choice) !== choice.name
          ? ` · ${accountEmail(choice)}`
          : ''
    }`,
  }));
  // Google names the primary calendar after the account, which the group heading already shows.
  const rowName = (calendar: CalendarChoice) =>
    calendar.account_id !== undefined && calendar.name.toLowerCase() === accountEmail(calendar)?.toLowerCase()
      ? tr('Calendario principale', 'Primary calendar')
      : calendar.name;

  // Grid layout
  function allDayEvents(day: string) {
    return s.events.filter((e) => isAllDay(e) && e.raw!.start.date! <= day && e.raw!.end.date! > day);
  }
  const hasAllDay = days.some((day) => allDayEvents(day).length);
  // How many event lines fit in a month cell: day number (24px) plus 21px per line.
  const monthChips = Math.max(1, Math.floor((s.monthHeight / 6 - 33) / 21));
  // Month cells list all-day events first, then the timed ones by start.
  function monthEvents(day: string) {
    return s.events
      .filter((e) =>
        isAllDay(e)
          ? e.raw!.start.date! <= day && e.raw!.end.date! > day
          : dateKey(e.start) <= day && dateKey(Math.max(e.start, e.end - 1)) >= day,
      )
      .sort((a, b) => Number(isAllDay(b)) - Number(isAllDay(a)) || a.start - b.start);
  }
  // Events that overlap share the day's width; the others use all of it.
  function placedEvents(day: string) {
    const items = s.events
      .filter((event) => !isAllDay(event))
      .filter((event) => dateKey(event.start) <= day && dateKey(Math.max(event.start, event.end - 1)) >= day)
      .map((event) => {
        const from = dateKey(event.start) === day ? minuteOfDay(event.start) : 0;
        const until = dateKey(event.end) === day ? minuteOfDay(event.end) : DAY_MINUTES;
        return { event, from, until: Math.max(until, from + 30), column: 0, columns: 1 };
      })
      .sort((a, b) => a.from - b.from);
    let cluster: typeof items = [];
    let clusterEnd = -1;
    const close = () => {
      const columns = Math.max(1, ...cluster.map((item) => item.column + 1));
      for (const item of cluster) item.columns = columns;
      cluster = [];
    };
    for (const item of items) {
      if (item.from >= clusterEnd) {
        close();
        clusterEnd = -1;
      }
      const taken = new Set(cluster.filter((other) => other.until > item.from).map((other) => other.column));
      while (taken.has(item.column)) item.column++;
      cluster.push(item);
      clusterEnd = Math.max(clusterEnd, item.until);
    }
    close();
    return items;
  }

  const draft = (() => {
    if (s.drag) return { day: s.drag.day, from: s.drag.from, until: s.drag.until };
    if (!s.form || s.selected || s.allDay || timeError || !s.start) return null;
    const day = s.start.slice(0, 10);
    return { day, from: minutesOf(s.start), until: s.end.slice(0, 10) === day ? minutesOf(s.end) : DAY_MINUTES };
  })();
  function pointerMinute(e: ReactPointerEvent<HTMLElement>) {
    const rect = e.currentTarget.getBoundingClientRect();
    const minute = ((e.clientY - rect.top) / HOUR_HEIGHT) * 60;
    return Math.max(0, Math.min(DAY_MINUTES - SNAP, Math.floor(minute / SNAP) * SNAP));
  }
  function slotDown(day: string, e: ReactPointerEvent<HTMLElement>) {
    if (e.button !== 0) return;
    e.currentTarget.setPointerCapture(e.pointerId);
    const minute = pointerMinute(e);
    const from = Math.floor(minute / 30) * 30;
    s.hover = null;
    s.drag = { day, anchor: minute, from, until: Math.min(from + 60, DAY_MINUTES), moved: false };
  }
  function slotMove(day: string, e: ReactPointerEvent<HTMLElement>) {
    const minute = pointerMinute(e);
    const drag = s.drag;
    if (!drag) {
      const half = Math.floor(minute / 30) * 30;
      if (s.hover?.day !== day || s.hover.minute !== half) s.hover = { day, minute: half };
      return;
    }
    if (!drag.moved && Math.abs(minute - drag.anchor) < SNAP) return;
    drag.moved = true;
    drag.from = Math.min(drag.anchor, minute);
    drag.until = Math.max(drag.anchor, minute) + SNAP;
  }
  function slotUp() {
    if (!s.drag) return;
    const { day, from, until } = s.drag;
    s.drag = null;
    requestSlot(day, from, until);
  }

  function defaultSlot(): [string, number] {
    const { rangeDays, focusDay } = derive();
    const today = dateKey(Date.now() / 1000);
    if (!rangeDays.includes(today)) return [focusDay, 9 * 60];
    const next = Math.ceil((minuteOfDay(Date.now() / 1000) + 1) / 30) * 30;
    return [today, Math.min(next, DAY_MINUTES - 60)];
  }
  const busyEditing = () =>
    say(tr('Salva o annulla prima le modifiche all’evento aperto.', 'Save or discard the open event first.'), true);
  function requestSlot(day: string, from: number, until: number) {
    if (s.form && !s.selected) {
      // A draft is open: move it instead of starting over, so typed details stay.
      s.allDay = false;
      s.start = slotValue(day, from);
      s.end = slotValue(day, until);
      return;
    }
    if (s.form && derive().dirty) return busyEditing();
    openEditor(null, [day, from, until]);
  }
  function requestEvent(event: VisibleCalendarEvent) {
    if (s.form && derive().dirty && (!s.selected || eventKey(s.selected) !== eventKey(event))) return busyEditing();
    openEditor(event);
  }
  function newEvent() {
    if (s.form && !s.selected) return titleInput.current?.focus();
    if (s.form && derive().dirty) return busyEditing();
    openEditor(null);
  }
  function toggleCalendar(choice: CalendarChoice) {
    const key = calendarKey(choice);
    if (s.hidden.includes(key)) s.hidden = s.hidden.filter((k) => k !== key);
    else {
      const { selectedCalendar, dirty } = derive();
      if (s.form && s.selected && calendarKey(selectedCalendar) === key) {
        if (dirty) return busyEditing();
        closeEditor();
      }
      s.hidden = [...s.hidden, key];
    }
  }
  function toggleAllCalendars() {
    const { allShown, dirty } = derive();
    if (allShown) {
      if (s.form && s.selected) {
        if (dirty) return busyEditing();
        closeEditor();
      }
      s.hidden = s.calendars.map(calendarKey);
    } else s.hidden = [];
  }

  function openEditor(event: VisibleCalendarEvent | null, slot?: [string, number, number]) {
    editGeneration.current++;
    s.selected = event;
    s.confirmingDelete = false;
    s.sheetError = '';
    s.allDay = Boolean(event?.raw?.start.date);
    s.title = event?.title || '';
    if (event) {
      s.start = s.allDay ? event.raw!.start.date! : dateInput(event.start);
      s.end = s.allDay ? event.raw!.end.date! : dateInput(event.end);
    } else {
      const [day, from] = slot ?? defaultSlot();
      s.start = slotValue(day, from);
      s.end = slotValue(day, slot ? slot[2] : from + 60);
    }
    s.location = event?.location || '';
    s.attendees = event?.attendees.join(', ') || '';
    s.link = event?.meeting_link || '';
    s.description = event?.notes || '';
    s.recurrence = event?.raw?.recurrence?.[0] || '';
    s.scope = 'instance';
    s.online = false;
    s.sendUpdates = 'all';
    creationKey.current = crypto.randomUUID();
    s.targetKey = calendarKey(derive().defaultTarget);
    s.initialForm = formValues();
    s.form = true;
  }
  function closeEditor() {
    s.form = false;
    s.selected = null;
    s.confirmingDelete = false;
    s.initialForm = null;
  }
  // A click on the calendar behind the panel closes it, unless there is unsaved work.
  function scrimClick() {
    if (derive().dirty) return busyEditing();
    closeEditor();
  }

  // Time fields: changing the start keeps the duration; all-day ends are exclusive in
  // storage but shown inclusive, the way people say "from Monday to Wednesday".
  function setStart(next: string) {
    if (!next || Number.isNaN(wall(next))) return;
    const duration = wall(s.end) - wall(s.start);
    s.start = next;
    if (duration > 0) s.end = fromWall(wall(next) + duration, s.allDay);
  }
  const startDate = () => s.start.slice(0, 10);
  const startTime = () => s.start.slice(11, 16);
  const endDate = () => (s.allDay ? fromWall(wall(s.end) - DAY_MS, true) : s.end.slice(0, 10));
  const endTime = () => s.end.slice(11, 16);
  const setStartDate = (value: string) => value && setStart(s.allDay ? value : `${value}T${startTime()}`);
  const setStartTime = (value: string) => value && setStart(`${startDate()}T${value}`);
  function setEndDate(value: string) {
    if (value) s.end = s.allDay ? fromWall(wall(value) + DAY_MS, true) : `${value}T${endTime()}`;
  }
  function setEndTime(value: string) {
    if (value) s.end = `${s.end.slice(0, 10)}T${value}`;
  }
  function setAllDay(on: boolean) {
    if (on === s.allDay) return;
    if (on) {
      const first = s.start.slice(0, 10);
      // A timed event ending after midnight still covers its last day.
      const lastExclusive = wall(s.end.slice(0, 10)) + (s.end.slice(11, 16) === '00:00' ? 0 : DAY_MS);
      s.start = first;
      s.end = fromWall(Math.max(lastExclusive, wall(first) + DAY_MS), true);
    } else {
      const last = fromWall(wall(s.end) - DAY_MS, true);
      s.start = `${s.start}T09:00`;
      s.end = `${last}T10:00`;
    }
    s.allDay = on;
  }

  async function load(cached = false) {
    const ticket = loads.begin(cached);
    const { shown: sources, rangeDays } = derive();
    const from = rangeDays[0];
    try {
      const to = addDays(rangeDays[rangeDays.length - 1], 1);
      // Google is read live (the cache holds a recurring series once, not its occurrences); the
      // 2-second refresh re-reads only local calendars and keeps the last Google read.
      const previous = s.events;
      const parts = await Promise.allSettled(
        sources.map(async (source) =>
          cached && source.provider_id !== 'local'
            ? previous.filter((event) => event.calendar && calendarKey(event.calendar) === calendarKey(source))
            : (await calendarEvents(source, from, to, false)).map((event) => ({ ...event, calendar: source })),
        ),
      );
      const result = parts.flatMap((part) => (part.status === 'fulfilled' ? part.value : []));
      const failed = parts.find((part) => part.status === 'rejected');
      if (!cached && failed && failed.status === 'rejected') throw failed.reason;
      if (!loads.accept(ticket)) return;
      s.events = result;
      if (!cached && sources.length === 1 && sources[0].provider_id !== 'local') s.syncedAt = Date.now() / 1000;
    } catch (e) {
      if (!cached) say(errorText(e), true);
    }
  }
  async function choices() {
    try {
      const result = await loadCalendarChoices();
      assignColors(result.items);
      s.calendars = result.items;
      s.calendarErrors = result.errors;
    } catch (e) {
      say(errorText(e), true);
    } finally {
      s.calendarsLoaded = true;
    }
  }
  async function connect(id: number) {
    if (s.connectingId !== null) return;
    s.connectingId = id;
    try {
      await calendarCall('authorize_calendar', { account_id: String(id) });
      await choices();
      const own = s.calendars.filter((c) => c.account_id === id);
      void syncGoogleCalendars(own).then(() => load(true));
      say(tr('Google Calendar collegato', 'Google Calendar connected'));
    } catch (e) {
      say(errorText(e), true);
    } finally {
      s.connectingId = null;
    }
  }
  async function sync() {
    const { hasGoogle, shown } = derive();
    if (!hasGoogle || s.syncing) return;
    s.syncing = true;
    try {
      const result = await syncGoogleCalendars(shown);
      if (result.synced) s.syncedAt = Date.now() / 1000;
      if (result.errors.length) say(errorText(result.errors[0]), true);
      await load();
    } catch (e) {
      say(errorText(e), true);
    } finally {
      s.syncing = false;
    }
  }

  // True when the event was saved; false when nothing was sent (invalid times, read-only, a save
  // already running) or the save failed.
  async function save(): Promise<boolean> {
    const { timeError, readOnly, formCalendar } = derive();
    if (s.busy || timeError || readOnly) return false;
    const generation = editGeneration.current;
    const event = s.selected;
    const edited = formValues();
    const calendar = { ...formCalendar };
    s.busy = true;
    s.sheetError = '';
    try {
      if (desktop) {
        const baseline = { ...s.initialForm! };
        const displayZone = regionalTimeZone();
        const editScope = s.scope;
        const local = calendar.id === 'local';
        const args: Record<string, unknown> = {
          ...calendarArgs(calendar),
          send_updates: s.sendUpdates,
          ...(event
            ? { id: event.id, expected_revision: event.raw?.revision, scope: editScope }
            : { idempotency_key: creationKey.current }),
        };
        if (s.online) args.online = true;
        if (event?.raw) {
          let master: ProviderEvent | undefined;
          if (event.raw.recurringEventId && editScope === 'series') {
            master = await calendarCall<ProviderEvent>('read_event', { ...calendarArgs(calendar), id: event.raw.recurringEventId });
            args.id = master.id;
            args.expected_revision = master.revision;
          }
          Object.assign(
            args,
            calendarFormChanges(baseline, edited, local),
            calendarTimeChanges(baseline, edited, event.raw, master, calendar.time_zone || displayZone, displayZone),
          );
        } else {
          Object.assign(args, {
            title: edited.title,
            start: eventDateTime(edited.start, edited.allDay),
            end: eventDateTime(edited.end, edited.allDay),
            all_day: edited.allDay,
            time_zone: regionalTimeZone(),
            location: edited.location,
            description: edited.description,
            attendees: edited.attendees
              .split(',')
              .map((part) => part.trim())
              .filter(Boolean),
            ...(local ? { meeting_link: edited.link || null } : edited.recurrence ? { recurrence: [edited.recurrence] } : {}),
          });
        }
        await calendarCall(event ? 'update_event' : 'create_event', args);
      } else
        await call('save_event', {
          event: {
            id: Number(event?.id) || 0,
            title: edited.title,
            start: parseLocalDateTime(edited.start),
            end: parseLocalDateTime(edited.end),
            location: edited.location,
            attendees: edited.attendees
              .split(',')
              .map((part) => part.trim())
              .filter(Boolean),
            meeting_link: edited.link || null,
            conference: '',
            rsvp: event?.rsvp || 'None',
            notes: event?.notes || '',
            color: [65, 106, 219],
          },
        });
      if (generation === editGeneration.current) closeEditor();
      // Show the calendar the new event went to, so it never seems to vanish.
      if (!event && s.hidden.includes(calendarKey(calendar))) s.hidden = s.hidden.filter((k) => k !== calendarKey(calendar));
      else await load();
      say(tr('Evento salvato', 'Event saved'));
      return true;
    } catch (e) {
      s.sheetError = errorText(e);
      return false;
    } finally {
      s.busy = false;
    }
  }
  // Closing the window over typed event details asks first (focus returns to the sheet). Unlike the
  // contact form, the sheet never saves at session end or on a reload: that would create an event
  // (or send invitations to its guests) nobody confirmed. It answers `false` without any call, so
  // "Ricarica la finestra" asks and a sign-out neither creates nor sends anything.
  useDirtyCloseGuard(
    'calendar-event',
    Boolean(D.dirty),
    () => {
      busyEditing();
      titleInput.current?.focus();
    },
    async () => false,
  );

  async function remove() {
    const event = s.selected;
    if (!event || s.busy) return;
    const calendar = { ...derive().selectedCalendar };
    const deleteScope = s.scope;
    const updates = s.sendUpdates;
    s.busy = true;
    try {
      if (desktop) {
        let id = event.id;
        let revision = event.raw?.revision;
        if (event.raw?.recurringEventId && deleteScope === 'series') {
          const master = await calendarCall<{ id: string; revision: string }>('read_event', {
            ...calendarArgs(calendar),
            id: event.raw.recurringEventId,
          });
          id = master.id;
          revision = master.revision;
        }
        await calendarCall('delete_event', {
          ...calendarArgs(calendar),
          id,
          expected_revision: revision,
          scope: deleteScope,
          send_updates: updates,
        });
      } else await call('delete_event', { id: Number(event.id) });
      closeEditor();
      await load();
      // The sheet gave the focus back to the event it came from, which is gone now: take it to the
      // calendar's own action instead of <body>.
      await tick();
      if (document.activeElement === document.body) newEventButton.current?.focus();
      say(tr('Evento eliminato', 'Event deleted'));
    } catch (e) {
      s.sheetError = errorText(e);
    } finally {
      s.busy = false;
      s.confirmingDelete = false;
    }
  }

  // Remember the view, the hidden calendars and the colours.
  const savedView = JSON.stringify({ view: s.view, hidden: s.hidden, colors: s.colorIndex });
  useEffect(() => {
    savePreference('calendar-view', savedView);
  }, [savedView]);

  // Read the events again when the calendars, the hidden ones or the range on screen change.
  // `s.hidden` and `s.calendars` change identity only when they are replaced, as in the Svelte effect.
  useEffect(() => {
    void load();
  }, [s.hidden, s.calendars, s.view, s.offset, s.regionalRevision, todayKey]);

  // Open the day grid near the current hour (or the working morning on other days), when the grid appears.
  const scroller = useCallback((element: HTMLDivElement | null) => {
    if (!element) return;
    const { rangeDays, todayKey, nowMinute } = derive();
    const minute = rangeDays.includes(todayKey) ? nowMinute : 8 * 60;
    // Start on an hour line with room above it: its label is centred on the line and would be cut in half.
    const hour = Math.max(0, Math.floor(minute / 60) - 3);
    element.scrollTop = Math.max(0, hour * HOUR_HEIGHT - 10);
  }, []);
  // The month cells' height decides how many event lines fit (Svelte's bind:clientHeight).
  const monthCells = useCallback((element: HTMLDivElement | null) => {
    if (!element) return;
    const measure = () => {
      s.monthHeight = element.clientHeight;
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    if (desktop) {
      void choices();
      void call<{ accounts: Account[] }>('workspace_snapshot')
        .then((snapshot) => {
          s.accounts = snapshot.accounts.filter((a) => a.kind === 'Gmail' && (!a.provider_id || a.provider_id === 'gmail'));
        })
        .catch((e) => say(errorText(e), true));
    }
    const poll = window.setInterval(() => {
      if (!s.form && !s.drag && !s.syncing && s.connectingId === null && document.visibilityState === 'visible') void load(true);
    }, 2000);
    const clock = window.setInterval(() => (s.now = Date.now()), 30_000);
    const changed = () => s.regionalRevision++;
    window.addEventListener('regionalchange', changed);
    return () => {
      window.clearInterval(poll);
      window.clearInterval(clock);
      window.removeEventListener('regionalchange', changed);
    };
  }, []);

  const editing = (event: VisibleCalendarEvent) => s.form && s.selected !== null && eventKey(s.selected) === eventKey(event);
  const newEventLabel = (day: string) =>
    tr(
      `Nuovo evento ${calendarDayLabel(day, { weekday: 'long', day: 'numeric', month: 'long' })}`,
      `New event on ${calendarDayLabel(day, { weekday: 'long', day: 'numeric', month: 'long' })}`,
    );

  const calendarRow = (choice: CalendarChoice) => {
    const active = !s.hidden.includes(calendarKey(choice));
    return (
      <button
        key={calendarKey(choice)}
        className={cn('cal-row', active && 'active')}
        role="checkbox"
        aria-checked={active}
        style={calColor(colorOf(choice))}
        onClick={() => toggleCalendar(choice)}
      >
        <span className="cal-dot" aria-hidden="true"></span>
        <span className="cal-name" title={choice.name}>
          {rowName(choice)}
        </span>
        {!choice.writable && <span className="cal-tag">{tr('Sola lettura', 'Read-only')}</span>}
      </button>
    );
  };

  const selected = s.selected;

  return (
    <section className="calendar-page" aria-labelledby="calendar-page-title">
      <h1 id="calendar-page-title" className="sr-only">
        {folderLabel('Calendar')}
      </h1>
      <div className="cal-layout">
        <aside className="cal-rail" aria-labelledby="cal-rail-title">
          <h2 id="cal-rail-title" className="rail-title">
            {tr('Calendari', 'Calendars')}
          </h2>
          {desktop && (
            <div className="rail-group">
              <button
                className={cn('cal-row', allShown && 'active')}
                role="checkbox"
                aria-checked={allShown ? 'true' : shown.length ? 'mixed' : 'false'}
                onClick={toggleAllCalendars}
              >
                <span className="cal-dot" aria-hidden="true"></span>
                <span className="cal-name">{tr('Tutti i calendari', 'All calendars')}</span>
              </button>
            </div>
          )}
          <div className="rail-group">
            <h3>{tr('Su questo computer', 'On this computer')}</h3>
            {localCalendars.map(calendarRow)}
          </div>
          {accountGroups.map((group) => (
            <div className="rail-group" key={group.id}>
              <h3 title={group.email}>{group.email}</h3>
              {group.state === 'ready' ? (
                group.items.map(calendarRow)
              ) : group.state === 'loading' ? (
                <p className="rail-note">{tr('Caricamento…', 'Loading…')}</p>
              ) : group.state === 'connecting' ? (
                <p className="rail-note" role="status">
                  <span className="spinner" aria-hidden="true"></span>
                  {tr('Concedi l’accesso nella finestra del browser…', 'Allow access in the browser window…')}
                </p>
              ) : group.state === 'consent' ? (
                <div className="rail-callout">
                  <p>
                    {tr(
                      'Google Calendar non è ancora collegato a questo account.',
                      'Google Calendar isn’t connected to this account yet.',
                    )}
                  </p>
                  <Button
                    variant="secondary"
                    className="rail-action"
                    disabled={s.connectingId !== null}
                    onClick={() => void connect(group.id)}
                  >
                    <Icon name="link" size={14} />
                    {tr('Collega Google Calendar', 'Connect Google Calendar')}
                  </Button>
                </div>
              ) : group.state === 'signin' ? (
                <div className="rail-callout">
                  <p>
                    {tr(
                      'Per il calendario serve l’accesso con Google su questo account.',
                      'This account needs Google sign-in for its calendar.',
                    )}
                  </p>
                  {openSettings && (
                    <Button variant="secondary" className="rail-action" onClick={openSettings}>
                      <Icon name="settings" size={14} />
                      {tr('Apri impostazioni account', 'Open account settings')}
                    </Button>
                  )}
                </div>
              ) : (
                <div className="rail-callout">
                  <p>{tr('Impossibile caricare i calendari.', 'Couldn’t load calendars.')}</p>
                  {group.error && <small>{errorText(group.error.message)}</small>}
                  <Button variant="secondary" className="rail-action" onClick={() => void choices()}>
                    <Icon name="refresh" size={14} />
                    {tr('Riprova', 'Try again')}
                  </Button>
                </div>
              )}
            </div>
          ))}
          {desktop && s.calendarsLoaded && !accountGroups.length && (
            <div className="rail-group">
              <h3>Google Calendar</h3>
              <div className="rail-callout">
                <p>
                  {tr(
                    'Aggiungi un account Google per vedere qui i suoi calendari.',
                    'Add a Google account to see its calendars here.',
                  )}
                </p>
                {openSettings && (
                  <Button variant="secondary" className="rail-action" onClick={openSettings}>
                    <Icon name="plus" size={14} />
                    {tr('Aggiungi account', 'Add account')}
                  </Button>
                )}
              </div>
            </div>
          )}
          {looseErrors.map((error, i) => (
            <p key={i} className="rail-note rail-error">
              {errorText(error.message)}
            </p>
          ))}
        </aside>

        <div className="cal-main">
          <header className="calendar-toolbar">
            <div className="toolbar-nav">
              <Button variant="secondary" onClick={() => (s.offset = 0)}>
                {tr('Oggi', 'Today')}
              </Button>
              <Button variant="icon" aria-label={stepLabel(false)} tip={stepLabel(false)} onClick={() => s.offset--}>
                <span className="rotate">
                  <Icon name="chevron" size={18} />
                </span>
              </Button>
              <Button variant="icon" aria-label={stepLabel(true)} tip={stepLabel(true)} onClick={() => s.offset++}>
                <Icon name="chevron" size={18} />
              </Button>
              <h2 className="week-title" aria-live="polite">
                {weekTitle}
              </h2>
            </div>
            <div className="toolbar-actions">
              <div className="view-switch" role="group" aria-label={tr('Vista del calendario', 'Calendar view')}>
                {VIEWS.map(([id, it, en]) => (
                  <button
                    key={id}
                    className={s.view === id ? 'active' : undefined}
                    aria-pressed={s.view === id}
                    onClick={() => setView(id)}
                  >
                    {tr(it, en)}
                  </button>
                ))}
              </div>
              <span className="sync-note">
                {regionalTimeZone()}
                {hasGoogle && s.syncedAt
                  ? ` · ${tr('aggiornato alle', 'updated at')} ${formatDateTime(s.syncedAt, { hour: '2-digit', minute: '2-digit' })}`
                  : null}
              </span>
              {hasGoogle && (
                <Button
                  variant="icon"
                  className={s.syncing ? 'spinning' : undefined}
                  disabled={s.syncing}
                  aria-label={tr('Sincronizza i calendari visibili', 'Sync visible calendars')}
                  title={tr('Sincronizza con Google', 'Sync with Google')}
                  onClick={() => void sync()}
                >
                  <Icon name="refresh" size={17} />
                </Button>
              )}
              <Button variant="primary" ref={newEventButton} onClick={newEvent}>
                <Icon name="plus" size={17} />
                {tr('Nuovo evento', 'New event')}
              </Button>
            </div>
          </header>

          {s.view === 'month' ? (
            <div className="week-grid month-grid">
              <div className="month-head">
                {weekdayHeads.map((day) => (
                  <span key={day}>{calendarDayLabel(day, { weekday: 'short' })}</span>
                ))}
              </div>
              <div className="month-cells" ref={monthCells}>
                {monthDays.map((day) => {
                  const list = monthEvents(day);
                  const lines = list.length <= monthChips ? list : list.slice(0, monthChips - 1);
                  return (
                    <div
                      key={day}
                      className={cn(
                        'month-cell',
                        day.slice(0, 7) !== monthStart.slice(0, 7) && 'outside',
                        day === todayKey && 'today',
                      )}
                    >
                      <button
                        className="month-slot"
                        tabIndex={-1}
                        aria-label={newEventLabel(day)}
                        onClick={() => requestSlot(day, 9 * 60, 10 * 60)}
                      ></button>
                      <button
                        className="month-daynum"
                        aria-label={calendarDayLabel(day, { weekday: 'long', day: 'numeric', month: 'long' })}
                        onClick={() => openDay(day)}
                      >
                        {Number(day.slice(-2))}
                      </button>
                      {lines.map((event) => (
                        <button
                          key={eventKey(event)}
                          className={cn('month-chip', editing(event) && 'editing')}
                          style={calColor(colorOf(event.calendar))}
                          title={event.title}
                          onClick={() => requestEvent(event)}
                        >
                          {!isAllDay(event) && <em>{formatDateTime(event.start, { hour: 'numeric', minute: '2-digit' })}</em>}
                          {event.title || tr('(Senza titolo)', '(No title)')}
                        </button>
                      ))}
                      {list.length > lines.length && (
                        <button className="month-more" onClick={() => openDay(day)}>
                          {tr(`+${list.length - lines.length} altri`, `+${list.length - lines.length} more`)}
                        </button>
                      )}
                    </div>
                  );
                })}
              </div>
            </div>
          ) : (
            <div className="week-grid" style={{ '--cols': days.length } as React.CSSProperties}>
              <div className="week-head">
                <span className="week-gutter"></span>
                {days.map((day) => (
                  <div key={day} className={cn('week-day-head', day === todayKey && 'today')}>
                    <span>{calendarDayLabel(day, { weekday: 'short' })}</span>
                    <strong>{Number(day.slice(-2))}</strong>
                  </div>
                ))}
              </div>
              {hasAllDay && (
                <div className="all-day-row">
                  <span className="week-gutter"></span>
                  {days.map((day) => (
                    <div key={day}>
                      {allDayEvents(day).map((event) => (
                        <button
                          key={eventKey(event)}
                          className={cn('all-day-chip', editing(event) && 'editing')}
                          style={calColor(colorOf(event.calendar))}
                          title={event.title}
                          onClick={() => requestEvent(event)}
                        >
                          {event.title || tr('(Senza titolo)', '(No title)')}
                        </button>
                      ))}
                    </div>
                  ))}
                </div>
              )}
              <div className="week-scroll" ref={scroller}>
                <div className="week-body" style={{ '--hour': `${HOUR_HEIGHT}px` } as React.CSSProperties}>
                  <div className="week-hours" aria-hidden="true">
                    {hours.map((hour) =>
                      hour > 0 ? (
                        <span key={hour} style={{ top: `${hour * HOUR_HEIGHT}px` }}>
                          {hourLabel(hour)}
                        </span>
                      ) : null,
                    )}
                  </div>
                  {days.map((day) => (
                    <div key={day} className={cn('calendar-day', day === todayKey && 'today')}>
                      <button
                        className="day-slot"
                        tabIndex={-1}
                        aria-label={newEventLabel(day)}
                        onPointerDown={(e) => slotDown(day, e)}
                        onPointerMove={(e) => slotMove(day, e)}
                        onPointerUp={slotUp}
                        onPointerCancel={() => (s.drag = null)}
                        onPointerLeave={() => (s.hover = null)}
                        onClick={(e) => {
                          if (e.detail === 0) requestSlot(day, 9 * 60, 10 * 60);
                        }}
                      ></button>
                      {s.hover?.day === day && !s.drag && (
                        <div className="slot-hint" style={blockStyle(s.hover.minute, s.hover.minute + 30)} aria-hidden="true">
                          <Icon name="plus" size={12} />
                          {clockLabel(s.hover.minute)}
                        </div>
                      )}
                      {placedEvents(day).map((item) => (
                        <button
                          key={eventKey(item.event)}
                          className={cn(
                            'calendar-event',
                            item.until - item.from < 45 && 'compact',
                            editing(item.event) && 'editing',
                          )}
                          style={{ ...eventStyle(item), ...calColor(colorOf(item.event.calendar)) }}
                          onClick={() => requestEvent(item.event)}
                        >
                          <strong>{item.event.title || tr('(Senza titolo)', '(No title)')}</strong>
                          <span>
                            {formatDateTime(item.event.start, { hour: 'numeric', minute: '2-digit' })}
                            {item.event.location ? ` · ${item.event.location}` : ''}
                          </span>
                        </button>
                      ))}
                      {draft?.day === day && (
                        <div
                          className={cn('draft-event', draft.until - draft.from < 45 && 'compact')}
                          style={blockStyle(draft.from, draft.until)}
                          aria-hidden="true"
                        >
                          <strong>{s.form && s.title.trim() ? s.title : tr('Nuovo evento', 'New event')}</strong>
                          <span>
                            {clockLabel(draft.from)} – {clockLabel(draft.until)}
                          </span>
                        </div>
                      )}
                      {day === todayKey && (
                        <div className="now-line" style={{ top: `${(nowMinute / 60) * HOUR_HEIGHT}px` }} aria-hidden="true"></div>
                      )}
                    </div>
                  ))}
                </div>
              </div>
            </div>
          )}
        </div>
      </div>

      {s.form && (
        <SidePanel
          // Another event opened by keyboard while the sheet shows one is a new opening: Esc goes back to
          // that event, not to the first one.
          key={s.selected ? eventKey(s.selected) : 'new'}
          narrow
          className="event-sheet"
          aria-labelledby="event-editor-title"
          onOpen={() => titleInput.current?.focus()}
          onClose={closeEditor}
          onScrim={scrimClick}
        >
          <form
            onSubmit={(e) => {
              e.preventDefault();
              void save();
            }}
          >
            <header className="side-panel-bar">
              <h2 id="event-editor-title" className="panel-title">
                {selected ? tr('Modifica evento', 'Edit event') : tr('Nuovo evento', 'New event')}
              </h2>
              <Button type="button" variant="icon" className="panel-close" aria-label={tr('Chiudi', 'Close')} tip={tr('Chiudi', 'Close')} onClick={closeEditor}>
                <Icon name="close" size={18} />
              </Button>
            </header>
            <div className="side-panel-body sheet-body">
              <fieldset disabled={readOnly}>
                <Input
                  ref={titleInput}
                  className="title-input"
                  value={s.title}
                  onChange={(e) => (s.title = e.target.value)}
                  required
                  aria-label={tr('Titolo', 'Title')}
                  placeholder={tr('Aggiungi un titolo', 'Add a title')}
                />
                {readOnly && (
                  <p className="sheet-note">
                    <Icon name="info" size={15} />
                    {tr('Questo calendario è in sola lettura.', 'This calendar is read-only.')}
                  </p>
                )}

                <div className="sheet-grid">
                  {!selected && writableCalendars.length > 1 ? (
                    <>
                      <span className="k" aria-hidden="true">
                        {tr('Calendario', 'Calendar')}
                      </span>
                      <Select
                        value={s.targetKey}
                        onChange={(value) => (s.targetKey = value)}
                        items={calendarItems}
                        aria-label={tr('Calendario', 'Calendar')}
                      />
                    </>
                  ) : (
                    <>
                      <span className="k">{tr('Calendario', 'Calendar')}</span>
                      <span className="v">{formCalendar.name}</span>
                    </>
                  )}

                  <span className="k" aria-hidden="true">
                    {tr('Inizio', 'Start')}
                  </span>
                  <div className={cn('dt', s.allDay && 'date-only')}>
                    <DateField
                      value={startDate()}
                      onChange={setStartDate}
                      aria-label={tr('Data di inizio', 'Start date')}
                      locale={dateLocale}
                      translations={calendarWords}
                    />
                    {!s.allDay && (
                      <Input
                        type="time"
                        aria-label={tr('Ora di inizio', 'Start time')}
                        required
                        value={startTime()}
                        onChange={(e) => setStartTime(e.target.value)}
                      />
                    )}
                  </div>
                  <span className="k" aria-hidden="true">
                    {tr('Fine', 'End')}
                  </span>
                  <div className={cn('dt', s.allDay && 'date-only')}>
                    <DateField
                      value={endDate()}
                      onChange={setEndDate}
                      aria-label={tr('Data di fine', 'End date')}
                      locale={dateLocale}
                      translations={calendarWords}
                    />
                    {!s.allDay && (
                      <Input
                        type="time"
                        aria-label={tr('Ora di fine', 'End time')}
                        required
                        value={endTime()}
                        onChange={(e) => setEndTime(e.target.value)}
                      />
                    )}
                  </div>
                  <span></span>
                  <div className="when-meta">
                    {desktop && (
                      <span className="check">
                        <Checkbox checked={s.allDay} onChange={setAllDay}>
                          {tr('Tutto il giorno', 'All day')}
                        </Checkbox>
                      </span>
                    )}
                    {timeError ? (
                      <span className="field-error" role="alert">
                        {tr('La fine deve essere dopo l’inizio', 'End must be after start')}
                      </span>
                    ) : (
                      <span className="muted">{durationLabel}</span>
                    )}
                  </div>

                  {!formLocal && !isInstance && (
                    <>
                      <span className="k" aria-hidden="true">
                        {tr('Ripeti', 'Repeat')}
                      </span>
                      <Select
                        value={s.recurrence || NO_REPEAT}
                        onChange={(value) => (s.recurrence = value === NO_REPEAT ? '' : value)}
                        items={recurrenceItems}
                        aria-label={tr('Ripeti', 'Repeat')}
                      />
                    </>
                  )}

                  <span className="rule"></span>

                  <label className="k" htmlFor="event-location">
                    {tr('Luogo', 'Location')}
                  </label>
                  <Input
                    id="event-location"
                    value={s.location}
                    onChange={(e) => (s.location = e.target.value)}
                    placeholder={tr('Aggiungi un luogo', 'Add a location')}
                  />

                  <label className="k" htmlFor="event-guests">
                    {tr('Invitati', 'Guests')}
                  </label>
                  <Input
                    id="event-guests"
                    value={s.attendees}
                    onChange={(e) => (s.attendees = e.target.value)}
                    placeholder={tr('nome@esempio.it, …', 'name@example.com, …')}
                    aria-describedby="event-guests-help"
                  />
                  <span className="hint" id="event-guests-help">
                    {tr('Separa gli indirizzi con una virgola.', 'Separate addresses with a comma.')}
                  </span>

                  {formLocal ? (
                    <>
                      <label className="k" htmlFor="event-link">
                        {tr('Meeting', 'Meeting')}
                      </label>
                      <Input
                        id="event-link"
                        value={s.link}
                        onChange={(e) => (s.link = e.target.value)}
                        type="url"
                        placeholder="https://…"
                      />
                    </>
                  ) : !selected?.meeting_link ? (
                    <>
                      <span></span>
                      <span className="check">
                        <Checkbox checked={s.online} onChange={(value) => (s.online = value)}>
                          {tr('Aggiungi videochiamata Google Meet', 'Add Google Meet video call')}
                        </Checkbox>
                      </span>
                    </>
                  ) : null}
                  {selected?.meeting_link && (
                    <>
                      <span className="k">{tr('Meeting', 'Meeting')}</span>
                      <Button
                        type="button"
                        variant="secondary"
                        className="join"
                        onClick={() => call('open_link', { url: selected.meeting_link }).catch((e) => say(errorText(e), true))}
                      >
                        <Icon name="link" size={14} />
                        {tr('Partecipa', 'Join')}
                      </Button>
                      {desktop && (
                        <>
                          <span></span>
                          <Button
                            type="button"
                            variant="secondary"
                            className="join"
                            onClick={() =>
                              call('event_join_record', {
                                url: selected.meeting_link,
                                key: `event:${selected.calendar ? calendarKey(selected.calendar) : ''}:${selected.id}:${selected.start}:${crypto.randomUUID()}`,
                                title: selected.title,
                                eventId: formLocal ? Number(selected.id) || null : null,
                                // A Google event has no numeric local id: the backend finds its local copy.
                                accountId: formLocal ? null : (selected.calendar?.account_id ?? null),
                                googleEventId: formLocal ? null : (selected.raw?.id ?? null),
                              }).then(
                                () => say(tr('Registrazione avviata', 'Recording started')),
                                (e) => say(errorText(e), true),
                              )
                            }
                          >
                            <Icon name="link" size={14} />
                            {tr('Partecipa e registra', 'Join and record')}
                          </Button>
                        </>
                      )}
                    </>
                  )}

                  {!formLocal && (guestCount || selected?.attendees.length) ? (
                    <>
                      <span className="k" aria-hidden="true">
                        {tr('Avvisa', 'Notify')}
                      </span>
                      <Select
                        value={s.sendUpdates}
                        onChange={(value) => (s.sendUpdates = value)}
                        items={updateItems}
                        aria-label={tr('Avvisa', 'Notify')}
                      />
                    </>
                  ) : null}

                  <label className="k top" htmlFor="event-notes">
                    {tr('Note', 'Notes')}
                  </label>
                  <Textarea
                    id="event-notes"
                    rows={4}
                    value={s.description}
                    onChange={(e) => (s.description = e.target.value)}
                    placeholder={tr('Aggiungi una descrizione', 'Add a description')}
                  ></Textarea>
                </div>
              </fieldset>
            </div>
            <footer className="sheet-foot">
              {s.sheetError && (
                <p className="sheet-error" role="alert">
                  <Icon name="info" size={15} />
                  {s.sheetError}
                </p>
              )}
              {isRecurring && !readOnly && (
                <div className="scope">
                  <span aria-hidden="true">{tr('Applica a', 'Apply to')}</span>
                  <Select
                    value={s.scope}
                    onChange={(value) => (s.scope = value)}
                    items={scopeItems}
                    aria-label={tr('Applica a', 'Apply to')}
                  />
                </div>
              )}
              <div className="foot-row">
                {selected &&
                  !readOnly &&
                  (s.confirmingDelete ? (
                    <>
                      <span className="confirm-text">{tr('Eliminare?', 'Delete?')}</span>
                      <Button type="button" variant="primary" className="danger" disabled={s.busy} onClick={() => void remove()}>
                        {tr('Elimina', 'Delete')}
                      </Button>
                      <Button type="button" variant="text" onClick={() => (s.confirmingDelete = false)}>
                        {tr('No', 'No')}
                      </Button>
                    </>
                  ) : (
                    <Button type="button" variant="text" className="danger" disabled={s.busy} onClick={() => (s.confirmingDelete = true)}>
                      <Icon name="trash" size={15} />
                      {tr('Elimina', 'Delete')}
                    </Button>
                  ))}
                <span className="spacer"></span>
                <Button type="button" variant="secondary" onClick={closeEditor}>
                  {readOnly ? tr('Chiudi', 'Close') : tr('Annulla', 'Cancel')}
                </Button>
                {!readOnly && (
                  <Button type="submit" variant="primary" disabled={s.busy || timeError}>
                    {s.busy ? tr('Salvataggio…', 'Saving…') : tr('Salva', 'Save')}
                  </Button>
                )}
              </div>
            </footer>
          </form>
        </SidePanel>
      )}
    </section>
  );
}
