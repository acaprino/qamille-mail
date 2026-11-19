import { dateInput, parseLocalDateTime, regionalTimeZone } from './regional.ts';

export interface EventTime {
  dateTime?: string;
  date?: string;
  timeZone?: string;
}
export interface EventSchedule { start: EventTime; end: EventTime }
export interface CalendarForm {
  title: string;
  start: string;
  end: string;
  allDay: boolean;
  location: string;
  description: string;
  attendees: string;
  recurrence: string;
  link: string;
}

const addresses = (value: string) => value.split(',').map(s => s.trim()).filter(Boolean);
const wallMillis = (value: string) => Date.parse(`${value.length === 10 ? `${value}T00:00` : value}:00Z`);

// Updates contain only edited fields. A title edit must not rewrite an event's
// recurrence, attendee responses, original time zone, or instance exceptions.
export function calendarFormChanges(before: CalendarForm, after: CalendarForm, local: boolean): Record<string, unknown> {
  const changes: Record<string, unknown> = {};
  for (const field of ['title', 'location', 'description'] as const) {
    if (before[field] !== after[field]) changes[field] = after[field];
  }
  if (JSON.stringify(addresses(before.attendees)) !== JSON.stringify(addresses(after.attendees)))
    changes.attendees = addresses(after.attendees);
  if (local && before.link !== after.link) changes.meeting_link = after.link || null;
  if (!local && before.recurrence !== after.recurrence)
    changes.recurrence = after.recurrence ? [after.recurrence] : [];
  return changes;
}

export function calendarTimeChanges(
  before: CalendarForm,
  after: CalendarForm,
  occurrence: EventSchedule,
  master?: EventSchedule,
  calendarZone = regionalTimeZone(),
  displayZone = regionalTimeZone(),
): Record<string, unknown> {
  const changes: Record<string, unknown> = {};
  const typeChanged = before.allDay !== after.allDay;
  const seriesZone = master?.start.timeZone || master?.end.timeZone || calendarZone;
  for (const boundary of ['start', 'end'] as const) {
    if (!typeChanged && before[boundary] === after[boundary]) continue;
    let edited = after[boundary];
    if (master) {
      const original = occurrence[boundary];
      const first = master[boundary];
      if (!typeChanged && !after.allDay) {
        // Use the recurrence zone's civil delta, not elapsed seconds: the chosen
        // occurrence and the master can be on opposite sides of a DST transition.
        const originalWall = dateInput(Date.parse(original.dateTime!) / 1000, seriesZone);
        const editedWall = dateInput(parseLocalDateTime(edited, displayZone), seriesZone);
        const firstWall = dateInput(Date.parse(first.dateTime!) / 1000, seriesZone);
        const shifted = wallMillis(firstWall) + wallMillis(editedWall) - wallMillis(originalWall);
        edited = new Date(shifted).toISOString().slice(0, 16);
      } else {
        // Date-only moves and type conversions preserve the master's calendar
        // date relative to the selected occurrence; all-day ends stay exclusive.
        const originalDay = before.allDay ? original.date! : dateInput(Date.parse(original.dateTime!) / 1000, displayZone).slice(0, 10);
        const firstDay = before.allDay ? first.date! : dateInput(Date.parse(first.dateTime!) / 1000, displayZone).slice(0, 10);
        const shifted = wallMillis(firstDay) + wallMillis(edited.slice(0, 10)) - wallMillis(originalDay);
        edited = `${new Date(shifted).toISOString().slice(0, 10)}${after.allDay ? '' : edited.slice(10)}`;
      }
    }
    const zone = master && !typeChanged ? seriesZone : displayZone;
    changes[boundary] = after.allDay ? edited : new Date(parseLocalDateTime(edited, zone) * 1000).toISOString();
    changes.time_zone = zone;
  }
  if (Object.keys(changes).length) changes.all_day = after.allDay;
  return changes;
}
