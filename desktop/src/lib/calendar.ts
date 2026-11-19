import { call, desktop } from './api';
import { parseLocalDateTime } from './regional';
import type { CalendarEvent } from './types';

export interface CalendarChoice { id: string; provider_id: string; account_id?: number; name: string; time_zone: string; writable: boolean }
export interface ProviderEvent {
  id: string; summary?: string; title?: string; revision: string; etag?: string;
  start: { dateTime?: string; date?: string; timeZone?: string }; end: { dateTime?: string; date?: string; timeZone?: string };
  location?: string; description?: string; attendees?: { email: string; responseStatus?: string; self?: boolean }[];
  hangoutLink?: string; recurringEventId?: string; recurrence?: string[];
}
export interface CalendarListError { account_id?: number; code?: string; message: string }
export interface VisibleCalendarEvent extends Omit<CalendarEvent, 'id'> { id: string; raw?: ProviderEvent; calendar?: CalendarChoice }
export const localCalendar: CalendarChoice = { id: 'local', provider_id: 'local', name: 'Qamille', time_zone: '', writable: true };
export const allCalendar: CalendarChoice = { id: 'all', provider_id: 'all', name: 'All', time_zone: '', writable: false };
export function calendarKey(calendar: CalendarChoice) { return `${calendar.account_id ?? ''}:${calendar.id}`; }
export const allCalendarKey = calendarKey(allCalendar);
export const eventKey = (event: VisibleCalendarEvent) => `${event.calendar ? calendarKey(event.calendar) : ''}|${event.id}`;
// Reads every connected Google calendar into the local cache, one at a time: calendar metadata is a single shared file.
// Runs queue behind each other, so the startup sync, the timer and the button never overlap.
export interface CalendarSyncResult { synced: number; errors: unknown[] }
let syncQueue: Promise<unknown> = Promise.resolve();
export function syncGoogleCalendars(known?: CalendarChoice[]): Promise<CalendarSyncResult> {
  const run = async (): Promise<CalendarSyncResult> => {
    const result: CalendarSyncResult = { synced: 0, errors: [] };
    if (!desktop) return result;
    let items: CalendarChoice[];
    try { items = known ?? (await loadCalendarChoices()).items; } catch (e) { result.errors.push(e); return result; }
    for (const calendar of items) {
      if (calendar.provider_id === 'local') continue;
      try { await calendarCall('sync_calendar', calendarArgs(calendar)); result.synced++; } catch (e) {
        // A missing sign-in or consent is shown in the calendar list, not as a notice.
        const code = (e as { code?: string } | null)?.code;
        if (code !== 'sign_in_required' && code !== 'calendar_consent_required') result.errors.push(e);
      }
    }
    return result;
  };
  const next = syncQueue.then(run, run);
  syncQueue = next;
  return next;
}
export function calendarArgs(calendar: CalendarChoice) { return { calendar_id: calendar.id, ...(calendar.account_id === undefined ? {} : { account_id: String(calendar.account_id) }) }; }
export async function calendarCall<T>(action: string, args: Record<string, unknown> = {}): Promise<T> { return call<T>('calendar_mcp', { action, args }); }
export function eventTimestamp(date: { dateTime?: string; date?: string }) { return date.dateTime ? Date.parse(date.dateTime) / 1000 : parseLocalDateTime(`${date.date}T00:00`); }
export function visibleEvent(raw: ProviderEvent): VisibleCalendarEvent {
  return { id: raw.id, title: raw.summary || raw.title || '', start: eventTimestamp(raw.start), end: eventTimestamp(raw.end), location: raw.location || '', attendees: raw.attendees?.map(a => a.email) || [], meeting_link: raw.hangoutLink || null, conference: raw.hangoutLink ? 'googleMeet' : '', notes: raw.description || '', color: [65, 106, 219], rsvp: 'None', raw };
}
export async function loadCalendarChoices(): Promise<{ items: CalendarChoice[]; errors: CalendarListError[] }> {
  if (!desktop) return { items: [localCalendar], errors: [] };
  const items: CalendarChoice[] = []; let errors: CalendarListError[] = []; let cursor: string | null = null;
  do { const page: { items: CalendarChoice[]; errors: CalendarListError[]; next_cursor?: string | null } = await calendarCall('list_calendars', { limit: 100, ...(cursor ? { cursor } : {}) }); items.push(...page.items); errors = page.errors; cursor = page.next_cursor || null; } while (cursor);
  return { items, errors };
}
export async function calendarEvents(calendar: CalendarChoice, from: string, to: string, cached = false): Promise<VisibleCalendarEvent[]> {
  if (!desktop) return (await call<CalendarEvent[]>('list_events')).map(event => ({ ...event, id: String(event.id) }));
  const items: VisibleCalendarEvent[] = []; let cursor: string | null = null;
  do { const page: { items: ProviderEvent[]; next_cursor?: string | null } = await calendarCall('list_events', { ...calendarArgs(calendar), from, to, limit: 100, ...(cached ? { cached: true } : {}), ...(cursor ? { cursor } : {}) }); items.push(...page.items.map(visibleEvent)); cursor = page.next_cursor || null; if (items.length > 10000) throw new Error('Choose a smaller calendar range'); } while (cursor);
  return items;
}
export function eventDateTime(value: string, allDay: boolean) { return allDay ? value : new Date(parseLocalDateTime(value) * 1000).toISOString(); }
