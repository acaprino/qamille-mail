import type { Settings } from './types';

// The app replaces this with the chosen interface language. This module stays free of
// Svelte runes so plain Node tests can import it.
let wording = (it: string, en: string) => (regionalLocale().startsWith('it') ? it : en);
export function configureRegionalWording(translate: (it: string, en: string) => string): void {
  wording = translate;
}

let configuredZone = '';
let configuredLocale = '';
let resolvedZone = '';
let resolvedLocale = '';

export function configureRegional(
  settings: Pick<Settings, 'time_zone' | 'locale'>,
  resolved?: { time_zone: string; locale: string },
): void {
  const previous = `${regionalTimeZone()}|${regionalLocale()}`;
  configuredZone = settings.time_zone || '';
  configuredLocale = settings.locale || '';
  resolvedZone = resolved?.time_zone || '';
  resolvedLocale = resolved?.locale || '';
  if (previous !== `${regionalTimeZone()}|${regionalLocale()}` && typeof window !== 'undefined') {
    window.dispatchEvent(new Event('regionalchange'));
  }
}

export function regionalLocale(): string {
  const value =
    configuredLocale ||
    resolvedLocale ||
    (typeof navigator !== 'undefined' ? navigator.language : 'en-US');
  try {
    return Intl.getCanonicalLocales(value)[0] || 'en-US';
  } catch {
    return 'en-US';
  }
}

// Building an Intl.DateTimeFormat is the costly part of formatting a date, and a list refresh formats
// hundreds of them (ADR-0007): keep one per (locale, options) and reuse it.
const formatters = new Map<string, Intl.DateTimeFormat>();
function formatter(locale: string, options: Intl.DateTimeFormatOptions): Intl.DateTimeFormat {
  const key = `${locale}|${JSON.stringify(options)}`;
  let found = formatters.get(key);
  if (!found) {
    found = new Intl.DateTimeFormat(locale, options);
    formatters.set(key, found);
  }
  return found;
}

// The canonical name of a zone, or null when it is not one: asked once per distinct value.
const zones = new Map<string, string | null>();
function canonicalZone(value: string): string | null {
  let found = zones.get(value);
  if (found === undefined) {
    try {
      found = formatter('en', { timeZone: value }).resolvedOptions().timeZone;
    } catch {
      found = null;
    }
    zones.set(value, found);
  }
  return found;
}

// The zone of the system, asked once: building a DateTimeFormat for it on every call made each date
// of a list render cost a formatter, and React renders the whole list again on every change.
let systemZone: string | undefined;
const systemTimeZone = () => (systemZone ??= Intl.DateTimeFormat().resolvedOptions().timeZone);

export function regionalTimeZone(): string {
  const value = configuredZone || resolvedZone || systemTimeZone();
  return canonicalZone(value) ?? 'UTC';
}

export function regionalWarning(): string | null {
  if (!configuredZone) return null;
  if (canonicalZone(configuredZone) !== null) return null;
  return regionalLocale().startsWith('it')
    ? 'Fuso non riconosciuto: uso UTC'
    : 'Unknown time zone: using UTC';
}

export function formatDateTime(
  timestamp: number,
  options: Intl.DateTimeFormatOptions = {},
): string {
  return formatter(regionalLocale(), {
    ...options,
    timeZone: regionalTimeZone(),
  }).format(new Date(timestamp * 1000));
}

function parts(timestamp: number, timeZone = regionalTimeZone()): Record<string, string> {
  return Object.fromEntries(
    formatter('en-CA', {
      timeZone,
      year: 'numeric',
      month: '2-digit',
      day: '2-digit',
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
      hourCycle: 'h23',
    })
      .formatToParts(new Date(timestamp * 1000))
      .map((part) => [part.type, part.value]),
  );
}

export function dateKey(timestamp: number): string {
  const value = parts(timestamp);
  return `${value.year}-${value.month}-${value.day}`;
}

export function dateInput(timestamp: number, timeZone = regionalTimeZone()): string {
  const value = parts(timestamp, timeZone);
  return `${value.year}-${value.month}-${value.day}T${value.hour}:${value.minute}`;
}

export function shortDate(timestamp: number, missing = false): string {
  if (missing)
    return wording('Data non disponibile', 'Date unavailable');
  return formatDateTime(
    timestamp,
    dateKey(timestamp) === dateKey(Date.now() / 1000)
      ? { hour: '2-digit', minute: '2-digit' }
      : { day: 'numeric', month: 'short', year: 'numeric' },
  );
}

export function fullDate(timestamp: number, missing = false): string {
  if (missing)
    return wording('Data non disponibile', 'Date unavailable');
  return formatDateTime(timestamp, {
    weekday: 'short',
    month: 'short',
    day: 'numeric',
    year: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
    timeZoneName: 'short',
  });
}

/** Interpret datetime-local in the configured zone, choosing the first repeated
 * instant. An impossible local time is rejected instead of silently shifting it. */
export function parseLocalDateTime(value: string, timeZone = regionalTimeZone()): number {
  if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}$/.test(value))
    throw new Error(wording('Inserisci data e ora complete', 'Enter a complete date and time'));
  const wall = Date.parse(`${value}:00Z`) / 1000;
  if (!Number.isFinite(wall) || new Date(wall * 1000).toISOString().slice(0, 16) !== value)
    throw new Error(wording('Data e ora non valide', 'Invalid date and time'));
  const candidates = new Set<number>();
  for (const delta of [-172800, -86400, 0, 86400, 172800]) {
    const probe = wall + delta;
    const p = parts(probe, timeZone);
    const local =
      Date.parse(`${p.year}-${p.month}-${p.day}T${p.hour}:${p.minute}:${p.second}Z`) / 1000;
    const candidate = wall - (local - probe);
    if (dateInput(candidate, timeZone) === value) candidates.add(candidate);
  }
  if (!candidates.size)
    throw new Error(
      regionalLocale().startsWith('it')
        ? 'Questo orario non esiste nel fuso selezionato. Scegli un altro orario.'
        : 'This time does not exist in the selected time zone. Choose another time.',
    );
  return Math.min(...candidates);
}

export function firstWeekday(): number {
  const locale = new Intl.Locale(regionalLocale()) as Intl.Locale & {
    weekInfo?: { firstDay: number };
    getWeekInfo?: () => { firstDay: number };
  };
  return (
    (locale.getWeekInfo?.().firstDay ??
      locale.weekInfo?.firstDay ??
      (/-US$/.test(regionalLocale()) ? 7 : 1)) % 7
  );
}

export function calendarDayLabel(key: string, options: Intl.DateTimeFormatOptions): string {
  return new Intl.DateTimeFormat(regionalLocale(), { ...options, timeZone: 'UTC' }).format(
    new Date(`${key}T12:00:00Z`),
  );
}

const cityNames: Record<string, string> = {
  'Europe/Rome': 'Roma · Rome',
  'Europe/London': 'Londra · London',
  'Europe/Paris': 'Parigi · Paris',
  'Europe/Berlin': 'Berlino · Berlin',
  'America/Havana': 'L’Avana · Havana',
  'America/New_York': 'New York',
  'America/Los_Angeles': 'Los Angeles',
  'Asia/Tokyo': 'Tokyo',
  UTC: 'UTC',
};

/** Browser-bundled IANA names: no geolocation, permission, or network needed. */
export function timeZoneChoices(): { value: string; label: string }[] {
  const intl = Intl as typeof Intl & { supportedValuesOf?: (key: string) => string[] };
  const values = intl.supportedValuesOf?.('timeZone') || Object.keys(cityNames);
  return [...new Set(['UTC', ...values, ...Object.keys(cityNames)])]
    .map((value) => ({
      value,
      label: `${cityNames[value] || value.split('/').at(-1)!.replaceAll('_', ' ')} · ${value}`,
    }))
    .sort((a, b) => a.label.localeCompare(b.label, regionalLocale()));
}
