import { useEffect } from 'react';
import { motion, useReducedMotion } from 'motion/react';
import { listen } from '@tauri-apps/api/event';
import { call, errorText } from '../lib/api';
import { setLanguageLocale, tr, useLanguage } from '../lib/i18n';
import { appearancePreference, resolvedAppearance, systemPrefersDark, type Appearance } from '../lib/appearance';
import { useReactive } from '../lib/store';
// Not Reminder.css: on Windows that name is the same file as the page's reminder.css.
import './Reminder.chips.css';

type Relative = { kind: 'in' | 'since'; minutes: number } | { kind: 'now' };
interface Chip {
  key: string;
  title: string;
  start: number;
  end: number;
  relative: Relative;
}
interface ReminderView {
  chips: Chip[];
  recording: boolean;
  dark: boolean;
  locale: string;
  time_zone: string;
}

export type ReminderProps = Record<string, never>;

/** A failed command shows its error in place of the commands for this long. */
const ERROR_MS = 6000;
/** The chip can appear under a click meant for another window: ignore it. */
const ARMED_AFTER_MS = 600;
const appeared = new Map<string, number>();
const armed = (chip: Chip) => Date.now() - (appeared.get(chip.key) ?? 0) >= ARMED_AFTER_MS;

/** Google and local events can have no title. */
const name = (chip: Chip) => chip.title || tr('Riunione senza titolo', 'Untitled meeting');

function when(relative: Relative): string {
  if (relative.kind === 'now') return tr('adesso', 'now');
  const m = relative.minutes;
  return relative.kind === 'in' ? tr(`tra ${m} min`, `in ${m} min`) : tr(`iniziata da ${m} min`, `started ${m} min ago`);
}

export default function Reminder() {
  useLanguage();
  const reduceMotion = useReducedMotion();
  const s = useReactive(() => ({
    view: { chips: [], recording: false, dark: false, locale: '', time_zone: '' } as ReminderView,
    appearance: appearancePreference() as Appearance,
    systemDark: systemPrefersDark(),
    busy: null as string | null,
    errors: {} as Record<string, string>,
    // Read by Narrator when a chip appears: the window never takes the focus (INV-004).
    announcement: '',
  }));

  function apply(next: ReminderView) {
    const fresh = next.chips.filter((chip) => !appeared.has(chip.key));
    for (const chip of fresh) appeared.set(chip.key, Date.now());
    s.view = next;
    setLanguageLocale(next.locale || navigator.language);
    if (fresh.length)
      s.announcement = fresh
        .map((chip) => tr(`Riunione ${name(chip)} ${when(chip.relative)}`, `Meeting ${name(chip)} ${when(chip.relative)}`))
        .join('. ');
  }

  function clock(seconds: number): string {
    try {
      return new Intl.DateTimeFormat(s.view.locale || undefined, {
        hour: '2-digit',
        minute: '2-digit',
        timeZone: s.view.time_zone || undefined,
      }).format(seconds * 1000);
    } catch {
      return new Date(seconds * 1000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
    }
  }

  async function join(chip: Chip, record: boolean) {
    if (s.busy || !armed(chip)) return;
    s.busy = chip.key;
    try {
      apply(await call<ReminderView>('reminder_join', { key: chip.key, record }));
    } catch (e) {
      s.errors[chip.key] = errorText(e);
      setTimeout(() => {
        delete s.errors[chip.key];
        void close(chip);
      }, ERROR_MS);
    } finally {
      s.busy = null;
    }
  }

  async function close(chip: Chip, fromClick = false) {
    if (fromClick && !armed(chip)) return;
    try {
      apply(await call<ReminderView>('reminder_dismiss', { key: chip.key }));
    } catch {
      // The engine drops the chip on its next check anyway.
    }
  }

  useEffect(() => {
    const colorScheme = window.matchMedia('(prefers-color-scheme: dark)');
    const paint = () => {
      s.systemDark = colorScheme.matches;
      document.documentElement.dataset.theme = resolvedAppearance(s.appearance, s.systemDark);
    };
    const appearanceChanged = (event: StorageEvent) => {
      if (event.key === 'qamille:appearance') {
        s.appearance = appearancePreference();
        paint();
      }
    };
    paint();
    colorScheme.addEventListener('change', paint);
    window.addEventListener('storage', appearanceChanged);
    let stop: (() => void) | undefined;
    let alive = true;
    void listen<ReminderView>('reminder-changed', (event) => apply(event.payload)).then((unlisten) => {
      if (alive) stop = unlisten;
      else unlisten();
    });
    void call<ReminderView>('reminder_view').then(apply).catch(() => {});
    // Esc on a chip the user has clicked into is "later", like its ×.
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || !s.view.chips.length) return;
      const focused = document.activeElement?.closest<HTMLElement>('[data-chip]')?.dataset.chip;
      const chip = s.view.chips.find((c) => c.key === focused) ?? s.view.chips[0];
      event.preventDefault();
      void close(chip);
    };
    window.addEventListener('keydown', onKeyDown);
    return () => {
      alive = false;
      colorScheme.removeEventListener('change', paint);
      window.removeEventListener('storage', appearanceChanged);
      window.removeEventListener('keydown', onKeyDown);
      stop?.();
    };
  }, []);

  return (
    <>
      <p className="announce" role="alert">
        {s.announcement}
      </p>
      {s.view.chips.map((chip) => (
        // The chip drops in on a short spring; it moves, it does not scale, so its 52px stay put.
        <motion.section
          key={chip.key}
          className="chip"
          data-chip={chip.key}
          aria-label={tr(`Promemoria: ${name(chip)}`, `Reminder: ${name(chip)}`)}
          initial={reduceMotion ? false : { opacity: 0, y: -6 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ type: 'spring', stiffness: 400, damping: 32 }}
        >
          <span className="live" aria-hidden="true"></span>
          <div className="text">
            <strong className="title" title={name(chip)}>
              {name(chip)}
            </strong>
            <span className="time">
              {clock(chip.start)}–{clock(chip.end)} · {when(chip.relative)}
            </span>
          </div>
          {s.errors[chip.key] ? (
            <p className="error" role="alert">
              {s.errors[chip.key]}
            </p>
          ) : (
            <div className="actions">
              <button className="join" disabled={s.busy === chip.key} onClick={() => join(chip, false)}>
                {tr('Entra', 'Join')}
              </button>
              <button
                className="record"
                disabled={s.busy === chip.key || s.view.recording}
                title={s.view.recording ? tr('Registrazione già in corso', 'A recording is already in progress') : undefined}
                onClick={() => join(chip, true)}
              >
                <span className="dot" aria-hidden="true"></span>
                {tr('Entra e registra', 'Join and record')}
              </button>
              <button
                className="close"
                aria-label={tr('Chiudi', 'Close')}
                title={tr('Chiudi', 'Close')}
                disabled={s.busy === chip.key}
                onClick={() => close(chip, true)}
              >
                <svg
                  width="16"
                  height="16"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                  strokeLinecap="round"
                  aria-hidden="true"
                >
                  <path d="m6 6 12 12M6 18 18 6" />
                </svg>
              </button>
            </div>
          )}
        </motion.section>
      ))}
    </>
  );
}
