// The Activity page (centro attività): the whole history of what the app did, for the user or on
// their behalf, newest first and grouped by day, with the live work on top. The sidebar panel is
// the glance, this is the record: filters, a "load more" that never moves what is on screen, and
// the same words, tones and icons as the panel (`lib/activity-log.ts`).
import { Fragment, useEffect, useRef } from 'react';
import Button from '../ds/primitives/Button';
import Icon from '../ds/primitives/Icon';
import Tabs from '../ds/primitives/Tabs';
import { call } from '../lib/api';
import { displayableActivities, activitySeconds, isStale, type Activity } from '../lib/activity';
import { activityCenter } from '../lib/activity-center';
import {
  coalesceEntries,
  entryIcon,
  entrySubtitle,
  entryTitle,
  entryTone,
  formatEntryTime,
  groupByDay,
  liveLabel,
} from '../lib/activity-log';
import { backendText, folderLabel, tr } from '../lib/i18n';
import { regionalLocale, regionalTimeZone } from '../lib/regional';
import { useReactive, useStore } from '../lib/store';
import { cn } from '../lib/utils';
import type { Account, ActivityEntry, ActivityFilter } from '../lib/types';
import './ActivityPage.css';

export type ActivityPageProps = { activities: Activity[]; accounts: Account[] };

/** Rows per request; a shorter answer is the end of the history. */
const PAGE = 100;
const words = { tr, backendText };
const liveIcons: Record<Activity['kind'], string> = {
  sync: 'refresh',
  index: 'search',
  history: 'archive',
  model: 'download',
  contacts: 'contacts',
  confirming: 'check',
};

export default function ActivityPage({ activities, accounts }: ActivityPageProps) {
  const center = useStore(activityCenter);
  const state = useReactive(() => ({
    filter: 'all' as ActivityFilter,
    entries: [] as ActivityEntry[],
    more: false,
    loading: false,
    loaded: false,
    // The last read failed: an unreadable history must not look like an empty one.
    failed: false,
    now: Date.now(),
  }));
  // Whether the user loaded past the first page: new rows then wait, the list stays where they read.
  const paged = useRef(false);
  const lastFilter = useRef<ActivityFilter | null>(null);
  const sequence = useRef(0);

  const zone = () => ({ locale: regionalLocale(), timeZone: regionalTimeZone() });
  const groups = groupByDay(coalesceEntries(state.entries), state.now, words, zone());
  const live = displayableActivities(activities, state.now);
  const accountName = (id: number): string | null =>
    accounts.find((account) => account.id === id)?.name ?? null;
  const timeOf = (atMs: number) => formatEntryTime(atMs, atMs, zone());

  const filters = [
    { value: 'all', label: tr('Tutto', 'All') },
    { value: 'action', label: tr('Azioni', 'Actions') },
    { value: 'event', label: tr('Eventi', 'Events') },
    { value: 'error', label: tr('Errori', 'Errors') },
  ];

  async function loadFirst() {
    const mine = ++sequence.current;
    state.loading = true;
    try {
      const rows = await call<ActivityEntry[]>('activity_page', {
        beforeSeq: null,
        limit: PAGE,
        filter: state.filter,
      });
      if (mine !== sequence.current) return;
      state.entries = rows;
      state.more = rows.length === PAGE;
      paged.current = false;
      state.failed = false;
      // Looking at the page is the "seen".
      activityCenter.unseen = 0;
      void call('activity_mark_seen').catch(() => {});
    } catch {
      if (mine === sequence.current) state.failed = true;
    } finally {
      if (mine === sequence.current) {
        state.loading = false;
        state.loaded = true;
      }
    }
  }

  async function loadMore() {
    const last = state.entries[state.entries.length - 1];
    if (!last || state.loading) return;
    const mine = ++sequence.current;
    state.loading = true;
    try {
      const rows = await call<ActivityEntry[]>('activity_page', {
        beforeSeq: last.seq,
        limit: PAGE,
        filter: state.filter,
      });
      if (mine !== sequence.current) return;
      state.entries = [...state.entries, ...rows];
      state.more = rows.length === PAGE;
      paged.current = true;
      state.failed = false;
    } catch {
      // The rows already shown stay; the page says the next ones could not be read.
      if (mine === sequence.current) state.failed = true;
    } finally {
      if (mine === sequence.current) state.loading = false;
    }
  }

  // A new filter, or a row announced by the backend, reads the first page again — unless the user
  // has scrolled on to older rows: then the list stays exactly where it is.
  const filter = state.filter;
  const version = center.version;
  useEffect(() => {
    if (filter !== lastFilter.current) paged.current = false;
    lastFilter.current = filter;
    if (paged.current) return;
    void loadFirst();
  }, [filter, version]);

  // The clock runs only while live work is listed (its duration), never on an idle page.
  const working = activities.length > 0;
  useEffect(() => {
    if (!working) return;
    const timer = setInterval(() => (state.now = Date.now()), 1000);
    return () => clearInterval(timer);
  }, [working, state]);

  function durationOf(activity: Activity): string {
    const seconds = activitySeconds(activity, state.now);
    if (seconds < 60) return `${seconds}s`;
    const minutes = Math.floor(seconds / 60);
    return minutes < 60
      ? `${minutes}m ${String(seconds % 60).padStart(2, '0')}s`
      : `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
  }

  return (
    <section className="activity-page" aria-label={folderLabel('Activity')}>
      <div className="page-heading">
        <div>
          <span className="page-kicker">
            {tr('Cosa è successo e cosa sta succedendo', 'What happened and what is going on')}
          </span>
          <h1>{folderLabel('Activity')}</h1>
        </div>
      </div>
      <div className="feed-filters">
        <Tabs
          value={state.filter}
          onChange={(value) => (state.filter = value as ActivityFilter)}
          items={filters}
          aria-label={tr('Filtra l’attività', 'Filter activity')}
        />
      </div>
      <div className="feed">
        {live.length > 0 && (
          <>
            <h2 className="feed-day">{tr('In corso', 'In progress')}</h2>
            <ul className="feed-list">
              {live.map((activity) => (
                <li
                  key={activity.id}
                  className={cn('feed-row tone-event', isStale(activity, state.now) && 'quiet')}
                  data-activity-id={activity.id}
                >
                  <span className="feed-chip">
                    <Icon name={liveIcons[activity.kind]} size={15} />
                  </span>
                  <span className="feed-text">
                    <strong>{liveLabel(activity, words)}</strong>
                    <small>
                      {activity.accountId === null
                        ? tr('Tutti gli account', 'All accounts')
                        : (accountName(activity.accountId) ?? `#${activity.accountId}`)}
                    </small>
                  </span>
                  <span className="feed-time">{durationOf(activity)}</span>
                </li>
              ))}
            </ul>
          </>
        )}
        {state.failed ? (
          <p className="feed-empty">
            {tr(
              'La cronologia non è disponibile in questo momento.',
              'History is unavailable right now.',
            )}
          </p>
        ) : !center.available ? (
          <p className="feed-empty">
            {tr(
              'La cronologia non è disponibile in questa modalità.',
              'History is not available in this mode.',
            )}
          </p>
        ) : (
          <>
            {groups.map((group) => (
              <Fragment key={group.label}>
                <h2 className="feed-day">{group.label}</h2>
                <ul className="feed-list">
                  {group.entries.map((entry) => {
                    const subtitle = entrySubtitle(entry, words, accountName);
                    return (
                      <li
                        key={entry.seq}
                        className={cn(
                          'feed-row',
                          `tone-${entryTone(entry)}`,
                          entry.outcome === 'undone' && 'undone',
                        )}
                        data-activity-seq={entry.seq}
                      >
                        <span className="feed-chip">
                          <Icon name={entryIcon(entry)} size={15} />
                        </span>
                        <span className="feed-text">
                          <strong>
                            {entryTitle(entry, words)}
                            {entry.source === 'assistant' && (
                              <span
                                className="assistant-mark"
                                title={tr('Fatto dall’assistente', 'Done by the assistant')}
                                role="img"
                                aria-label={tr('Fatto dall’assistente', 'Done by the assistant')}
                              >
                                <Icon name="spark" size={12} />
                              </span>
                            )}
                          </strong>
                          {subtitle && <small>{subtitle}</small>}
                        </span>
                        <time className="feed-time" dateTime={new Date(entry.at_ms).toISOString()}>
                          {timeOf(entry.at_ms)}
                        </time>
                      </li>
                    );
                  })}
                </ul>
              </Fragment>
            ))}
            {state.more && (
              <div className="feed-more">
                <Button
                  variant="secondary"
                  onClick={() => void loadMore()}
                  disabled={state.loading}
                >
                  {tr('Carica altri', 'Load more')}
                </Button>
              </div>
            )}
            {state.loaded && !state.entries.length && !live.length && (
              <p className="feed-empty">
                {state.filter === 'all'
                  ? tr(
                      'Qui vedrai quello che l’app fa per te: messaggi archiviati, inviati, sincronizzazioni, errori.',
                      'Here you will see what the app does for you: archived and sent messages, syncs, errors.',
                    )
                  : tr('Niente in questa categoria.', 'Nothing in this category.')}
              </p>
            )}
          </>
        )}
      </div>
    </section>
  );
}
