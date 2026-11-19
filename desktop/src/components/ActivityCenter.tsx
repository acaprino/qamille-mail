// The activity center in the sidebar footer (ADR-0007 v3 decision 7, INV-009; centro attività).
// Two surfaces, both inside the sidebar and neither fixed over the mail:
//  - the card: one line for the live work (sync, indexing, model install, contacts sync, the
//    confirmations past their undo window), present only while something works, never flashing
//    (400 ms before it shows, 1 s before it leaves), a quiet activity dims instead of lying;
//  - the panel: the live work and the three newest rows of the history, opened by the card or by
//    the history button of the footer.
// The ring advances with the percent and nothing in the card moves on its own: the sidebar is not
// the place to draw the eye. Errors are history rows, never card content (INV-009).
import { AnimatePresence, motion, useReducedMotion } from 'motion/react';
import { useEffect, useRef, useState } from 'react';
import Count from '../ds/primitives/Count';
import Icon from '../ds/primitives/Icon';
import { backendText, tr } from '../lib/i18n';
import { regionalLocale, regionalTimeZone } from '../lib/regional';
import {
  activityHideDelayMs,
  activityMinVisibleMs,
  activitySeconds,
  displayableActivities,
  isStale,
  primaryActivity,
  withConfirming,
  type Activity,
} from '../lib/activity';
import { activityCenter, closeActivityPanel, toggleActivityPanel } from '../lib/activity-center';
import {
  entryIcon,
  entrySubtitle,
  entryTitle,
  entryTone,
  formatEntryTime,
  liveLabel,
} from '../lib/activity-log';
import { useStore } from '../lib/store';
import { tick } from '../lib/tick';
import { cn } from '../lib/utils';
import type { Account, UndoAction } from '../lib/types';
import { imeComposing } from '../ds/ime';
import './ActivityCenter.css';

export type ActivityCenterProps = {
  /** The event-fed registry (sync, index, model, contacts). */
  activities: Activity[];
  /** The workspace's undo actions: the confirming count is derived from them. */
  undoActions: UndoAction[];
  accounts: Account[];
  /** A row click opens the relevant list or settings page. */
  openActivity: (activity: Activity) => void;
  /** "Vedi tutta l'attività": the full page. */
  showAll: () => void;
};

const words = { tr, backendText };
const icons: Record<Activity['kind'], string> = {
  sync: 'refresh',
  index: 'search',
  history: 'archive',
  model: 'download',
  contacts: 'contacts',
  confirming: 'check',
};
/** The ring of the card: the share of the circle (r = 15) the percent covers. */
const ringLength = 2 * Math.PI * 15;
const ring = (percent: number) =>
  `${(Math.min(Math.max(percent, 0), 100) / 100) * ringLength} ${ringLength}`;
const spring = { type: 'spring', stiffness: 400, damping: 32 } as const;

export default function ActivityCenter({
  activities,
  undoActions,
  accounts,
  openActivity,
  showAll,
}: ActivityCenterProps) {
  const center = useStore(activityCenter);
  const still = useReducedMotion();
  const [now, setNow] = useState(() => Date.now());

  // The event-fed activities are the App's; the confirming entry is the component's own state,
  // fed back through `withConfirming` so its start never moves while the count stands (a value
  // derived in render would rebuild it every tick and it could never reach the 400 ms visibility).
  const [confirming, setConfirming] = useState<Activity[]>([]);
  useEffect(() => {
    setConfirming((current) => withConfirming(current, undoActions, now));
  }, [undoActions, now]);
  const all = [...activities, ...confirming];
  const displayable = displayableActivities(all, now);
  const showing = displayable.length > 0;

  // What is on screen: the displayable activities, kept for `activityHideDelayMs` after the last
  // one ends so consecutive work does not blink.
  const lastShown = useRef<Activity[] | null>(null);
  if (showing) lastShown.current = displayable;
  const [lingering, setLingering] = useState(false);
  const hideTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  useEffect(() => {
    if (showing) {
      clearTimeout(hideTimer.current);
      hideTimer.current = undefined;
      setLingering(true);
      return;
    }
    // Nothing displayable: what is on screen leaves one second after the last activity.
    if (lingering && hideTimer.current === undefined)
      hideTimer.current = setTimeout(() => {
        hideTimer.current = undefined;
        setLingering(false);
      }, activityHideDelayMs);
  }, [showing, lingering]);
  useEffect(() => () => clearTimeout(hideTimer.current), []);
  const visibleList = showing ? displayable : lingering ? lastShown.current : null;
  const primary = visibleList ? primaryActivity(visibleList) : null;
  // The quiet cue is not motion alone: the icon also dims (visible under prefers-reduced-motion)
  // and a screen reader hears the same fact as text.
  const primaryQuiet = primary !== null && isStale(primary, now);

  // The clock runs only while something can change on screen (INV-005): fast while an
  // undo window is about to close or an activity is too young to show, slower while an
  // activity lives (duration, quiet cue), never while idle.
  const anyActivity = all.length > 0;
  const onScreen = visibleList !== null;
  const windowOpen = undoActions.some(
    (action) => action.status === 'pending' && action.expires_at_ms > now,
  );
  const young = all.some((activity) => now - activity.since < activityMinVisibleMs);
  useEffect(() => {
    if (!anyActivity && !onScreen && !windowOpen) return;
    const timer = setInterval(() => setNow(Date.now()), young || windowOpen ? 250 : 1000);
    return () => clearInterval(timer);
  }, [anyActivity, onScreen, windowOpen, young]);

  /** One short line per kind; the percent is part of the label when it is known. */
  const labelOf = (activity: Activity): string => liveLabel(activity, words);
  const accountName = (id: number): string | null =>
    accounts.find((account) => account.id === id)?.name ?? null;
  function accountOf(activity: Activity): string {
    if (activity.accountId !== null)
      return accountName(activity.accountId) ?? `#${activity.accountId}`;
    return activity.kind === 'contacts' ? tr('Tutti gli account', 'All accounts') : '—';
  }
  function durationOf(activity: Activity): string {
    const seconds = activitySeconds(activity, now);
    if (seconds < 60) return `${seconds}s`;
    const minutes = Math.floor(seconds / 60);
    if (minutes < 60) return `${minutes}m ${String(seconds % 60).padStart(2, '0')}s`;
    return `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
  }

  // Keyboard: the panel sits above the footer in the DOM, so opening it hands it the focus (the
  // next Tab lands on its own button), and closing it gives the focus back to the history button
  // when it was inside — not when the user clicked somewhere else.
  const panelEl = useRef<HTMLDivElement>(null);
  const restoreFocus = useRef(false);
  const wasOpen = useRef(false);
  /** Closes the panel from the keyboard or from one of its own buttons: the focus goes back to the toggle. */
  function closeFromInside() {
    restoreFocus.current = !!panelEl.current?.contains(document.activeElement);
    closeActivityPanel();
  }
  function openFor(activity: Activity) {
    closeFromInside();
    openActivity(activity);
  }
  function showEverything() {
    closeFromInside();
    showAll();
  }
  const open = center.open;
  useEffect(() => {
    if (open && !wasOpen.current) void tick().then(() => panelEl.current?.focus());
    if (!open && wasOpen.current && restoreFocus.current)
      document.querySelector<HTMLElement>('.activity-toggle')?.focus();
    if (!open) restoreFocus.current = false;
    wasOpen.current = open;
  }, [open]);

  useEffect(() => {
    function outside(event: PointerEvent) {
      if (
        activityCenter.open &&
        !(
          event.target instanceof Element &&
          event.target.closest('.activity-panel, .activity-indicator, .activity-toggle')
        )
      )
        closeActivityPanel();
    }
    function onKey(event: KeyboardEvent) {
      if (event.key === 'Escape' && activityCenter.open && !imeComposing(event)) closeFromInside();
    }
    window.addEventListener('pointerdown', outside);
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('pointerdown', outside);
      window.removeEventListener('keydown', onKey);
    };
  }, []);

  const zone = { locale: regionalLocale(), timeZone: regionalTimeZone() };

  return (
    <>
      <AnimatePresence>
        {open && (
          <motion.div
            key="activity-panel"
            className="activity-panel"
            id="activity-panel"
            role="region"
            aria-label={tr('Attività', 'Activity')}
            tabIndex={-1}
            ref={panelEl}
            style={{ transformOrigin: 'bottom left' }}
            initial={still ? false : { opacity: 0, y: 10, scale: 0.97 }}
            animate={{ opacity: 1, y: 0, scale: 1 }}
            exit={
              still
                ? { opacity: 0, transition: { duration: 0 } }
                : { opacity: 0, y: 6, scale: 0.98 }
            }
            transition={spring}
          >
            <div className="activity-heading">{tr('Attività', 'Activity')}</div>
            {visibleList && visibleList.length > 0 && (
              <>
                <div className="activity-section">{tr('In corso', 'In progress')}</div>
                {visibleList.map((activity) =>
                  activity.kind === 'confirming' ? (
                    <div
                      key={activity.id}
                      className={cn('activity-row', isStale(activity, now) && 'quiet')}
                      data-activity-id={activity.id}
                    >
                      <span className="activity-chip">
                        <Icon name={icons[activity.kind]} size={13} />
                      </span>
                      <span className="activity-text">
                        <strong className="activity-phase">{labelOf(activity)}</strong>
                        <small className="activity-account">{accountOf(activity)}</small>
                      </span>
                      <span className="activity-meta">
                        <span className="activity-duration">{durationOf(activity)}</span>
                      </span>
                    </div>
                  ) : (
                    <button
                      key={activity.id}
                      className={cn('activity-row', isStale(activity, now) && 'quiet')}
                      data-activity-id={activity.id}
                      onClick={() => openFor(activity)}
                      title={tr('Apri la pagina collegata', 'Open the related page')}
                    >
                      <span className="activity-chip">
                        <span
                          className={cn(
                            'activity-glyph',
                            activity.percent === null && !isStale(activity, now) && 'spinning',
                          )}
                        >
                          <Icon name={icons[activity.kind]} size={13} />
                        </span>
                      </span>
                      <span className="activity-text">
                        <strong className="activity-phase">{labelOf(activity)}</strong>
                        <small className="activity-account">{accountOf(activity)}</small>
                      </span>
                      <span className="activity-meta">
                        {activity.percent !== null &&
                          activity.kind !== 'index' &&
                          activity.kind !== 'model' && (
                            <span className="activity-percent">
                              <Count value={activity.percent} />%
                            </span>
                          )}
                        <span className="activity-duration">{durationOf(activity)}</span>
                      </span>
                    </button>
                  ),
                )}
              </>
            )}
            <div className="activity-section">{tr('Recenti', 'Recent')}</div>
            {center.failed ? (
              <div className="activity-empty">
                {tr(
                  'La cronologia non è disponibile in questo momento.',
                  'History is unavailable right now.',
                )}
              </div>
            ) : !center.available ? (
              <div className="activity-empty">
                {tr(
                  'La cronologia non è disponibile in questa modalità.',
                  'History is not available in this mode.',
                )}
              </div>
            ) : center.recent.length === 0 ? (
              <div className="activity-empty">{tr('Niente da segnalare', 'Nothing to report')}</div>
            ) : (
              // A row that arrives while the panel is open slides in; the rows it pushes out leave.
              <AnimatePresence initial={false}>
                {center.recent.map((entry) => {
                  const subtitle = entrySubtitle(entry, words, accountName);
                  return (
                    <motion.div
                      key={entry.seq}
                      className={cn(
                        'activity-recent',
                        `tone-${entryTone(entry)}`,
                        entry.outcome === 'undone' && 'undone',
                      )}
                      data-activity-seq={entry.seq}
                      layout={still ? false : 'position'}
                      initial={still ? false : { opacity: 0, y: -8, scale: 0.97 }}
                      animate={{ opacity: 1, y: 0, scale: 1 }}
                      exit={
                        still
                          ? { opacity: 0, transition: { duration: 0 } }
                          : { opacity: 0, scale: 0.96 }
                      }
                      transition={spring}
                    >
                      <span className="activity-chip">
                        <Icon name={entryIcon(entry)} size={13} />
                      </span>
                      <span className="activity-text">
                        <strong>{entryTitle(entry, words)}</strong>
                        {subtitle && <small>{subtitle}</small>}
                      </span>
                      <span className="activity-time">
                        {formatEntryTime(entry.at_ms, Date.now(), zone)}
                      </span>
                    </motion.div>
                  );
                })}
              </AnimatePresence>
            )}
            <button className="activity-all" onClick={showEverything}>
              {tr('Vedi tutta l’attività', 'See all activity')}
              <Icon name="arrow" size={13} />
            </button>
          </motion.div>
        )}
      </AnimatePresence>
      {visibleList && visibleList.length > 0 && primary && (
        <div className="activity-indicator">
          <button
            className="activity-line"
            aria-expanded={open}
            aria-controls="activity-panel"
            aria-label={tr('Attività in corso: apri l’elenco', 'App activity: open the list')}
            onClick={toggleActivityPanel}
          >
            <span className="activity-chip">
              <svg className="activity-ring" viewBox="0 0 36 36" aria-hidden="true">
                <circle className="track" cx="18" cy="18" r="15" />
                {primary.percent !== null && (
                  <circle
                    className="value"
                    cx="18"
                    cy="18"
                    r="15"
                    strokeDasharray={ring(primary.percent)}
                  />
                )}
              </svg>
              <span className={cn('activity-icon', primaryQuiet && 'quiet')} aria-hidden="true">
                <Icon name={icons[primary.kind]} size={14} />
              </span>
            </span>
            <span className="activity-card-text">
              <span role="status">
                {labelOf(primary)}
                {primaryQuiet && (
                  <span className="sr-only">
                    , {tr('nessun aggiornamento da alcuni minuti', 'no update for several minutes')}
                  </span>
                )}
              </span>
              <small>{accountOf(primary)}</small>
            </span>
            {visibleList.length > 1 && (
              <span className="activity-more">+{visibleList.length - 1}</span>
            )}
          </button>
        </div>
      )}
    </>
  );
}
