// The left column of the Meetings page: the title with "Registra ora", the search, and below
// either the groups of rows (In corso, In elaborazione, Prossime, Passate: an empty group is not
// drawn) or, while there is something in the search field, the results in their place. It holds
// no data of its own: the shell loads, selects and searches, this draws and reports.
//
// The rows of a group arrive and leave with Sigrid's motion (`Presence` and `Item` of ui/motion).
import { Presence } from '../../ui/motion';
import { Fragment, type ReactNode } from 'react';
import Button from '../../ui/Button';
import Icon from '../../ui/Icon';
import { desktop } from '../../lib/api';
import { tr } from '../../lib/i18n';
import type { CaptureSnapshot, MeetingHit } from '../../lib/meetings';
import { fragmentAround, markedParts, type MarkedPart } from '../../lib/meetings-highlight';
import { GROUP_ORDER, clockLabel, type MeetingGroup, type MeetingRow } from '../../lib/meetings-view';
import { cn } from '../../lib/utils';
import MeetingRowItem from './MeetingRowItem';
import './MeetingList.css';
import { Input } from '../../ui/Input';

export type MeetingListProps = {
  groups: Record<MeetingGroup, MeetingRow[]>;
  selectedKey: string | null;
  query: string;
  /** `null`: the search is not back yet (or there is no search). */
  hits: MeetingHit[] | null;
  /** The words `hits` were found for. They differ from `query` while the user types on: the results stay. */
  hitsQuery: string;
  onselect: (row: MeetingRow) => void;
  onjoin: (row: MeetingRow) => void;
  onquery: (value: string) => void;
  onopenhit: (hit: MeetingHit) => void;
  /** Present when there are more meetings than the page shows. */
  onmore?: () => void;
  live: CaptureSnapshot;
  /** Unix seconds the rows were built with. */
  now: number;
  status?: 'loading' | 'ready' | 'failed';
  onretry?: () => void;
  searchFailed?: boolean;
  /** The result the detail was opened from, to mark it as the current one. */
  openedHit?: MeetingHit | null;
  /** The next meetings are being read: "Mostra altre riunioni" waits, and keeps the focus. */
  moreBusy?: boolean;
  /** A join is on its way: no row offers another one until it ends. */
  joinBusy?: boolean;
  /** Why the engine cannot transcribe now, or `null`: a recording waiting for its transcript says it. */
  blocker?: string | null;
  /** A "Riprova" of a row is on its way: no row offers another one until it ends. */
  transcribeBusy?: boolean;
  /** Opens Settings on Meetings. */
  onsettings: () => void;
  /** "Riprova" on a row whose transcription failed: a new transcription with the engine of now. */
  ontranscribe: (row: MeetingRow) => void;
  /** Next to the title: "Registra ora". */
  actions?: ReactNode;
};

const marked = (parts: MarkedPart[]) =>
  parts.map((part, i) => (part.hit ? <mark key={i}>{part.text}</mark> : <Fragment key={i}>{part.text}</Fragment>));

export default function MeetingList({
  groups,
  selectedKey,
  query,
  hits,
  hitsQuery,
  onselect,
  onjoin,
  onquery,
  onopenhit,
  onmore = undefined,
  live,
  now,
  status = 'ready',
  onretry = undefined,
  searchFailed = false,
  openedHit = null,
  moreBusy = false,
  joinBusy = false,
  blocker = null,
  transcribeBusy = false,
  onsettings,
  ontranscribe,
  actions = undefined,
}: MeetingListProps) {
  const labels: Record<MeetingGroup, string> = {
    live: tr('In corso', 'In progress'),
    processing: tr('In elaborazione', 'Processing'),
    upcoming: tr('Prossime', 'Upcoming'),
    past: tr('Passate', 'Past'),
  };
  const searching = query.trim() !== '';
  const empty = GROUP_ORDER.every((name) => groups[name].length === 0);
  /** The words typed are not the ones the results on screen are for: the new ones are on their way. */
  const stale = hits !== null && hitsQuery !== query.trim();

  /** The count of the results on screen, for the words they were found for (not the ones typed since). */
  function resultsHeading(): string {
    const text = hitsQuery;
    if (searchFailed) return tr('La ricerca non è riuscita', 'The search did not work');
    if (hits === null) return tr('Cerco…', 'Searching…');
    if (hits.length === 0) return tr(`Nessun risultato per «${text}»`, `No results for “${text}”`);
    return tr(
      `${hits.length} ${hits.length === 1 ? 'risultato' : 'risultati'} per «${text}»`,
      `${hits.length} ${hits.length === 1 ? 'result' : 'results'} for “${text}”`,
    );
  }
  /** The title of the meeting, with the match marked when the match is the title. */
  const titleParts = (hit: MeetingHit): MarkedPart[] =>
    hit.kind === 'title' ? markedParts(hit.title, hitsQuery) : [{ text: hit.title, hit: false }];
  /** The words around the match, on one line, with the match marked. */
  const fragmentParts = (hit: MeetingHit): MarkedPart[] => markedParts(fragmentAround(hit.text, hitsQuery), hitsQuery);
  /** Where in the meeting the hit is: the moment of a line of the transcript, else the part. */
  function where(hit: MeetingHit): string {
    if (hit.kind === 'transcript') return hit.at_ms === null ? tr('Trascrizione', 'Transcript') : clockLabel(hit.at_ms);
    return hit.kind === 'title' ? tr('Titolo', 'Title') : tr('Sintesi', 'Summary');
  }

  let content: ReactNode;
  if (searching) {
    content = (
      <div role="group" aria-label={tr('Risultati della ricerca', 'Search results')} aria-busy={stale ? 'true' : undefined}>
        <h2 className="group-label results">
          <span role="status">{resultsHeading()}</span>
        </h2>
        {hits && hits.length > 0 && (
          <ul className="hits">
            {hits.map((hit) => (
              <li key={`${hit.meeting_id}:${hit.kind}:${hit.segment_id ?? ''}:${hit.version ?? ''}`}>
                <button
                  type="button"
                  className={cn('hit-item', openedHit === hit && 'selected')}
                  aria-current={openedHit === hit ? 'true' : undefined}
                  onClick={() => onopenhit(hit)}
                >
                  <span className="hit-head">
                    <span className="hit-title">{marked(titleParts(hit))}</span>
                    <span className="hit-where">{where(hit)}</span>
                  </span>
                  {hit.kind !== 'title' && <span className="hit-fragment">{marked(fragmentParts(hit))}</span>}
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>
    );
  } else if (status === 'loading') {
    content = <p className="list-note">{tr('Carico le riunioni…', 'Loading meetings…')}</p>;
  } else if (status === 'failed') {
    content = (
      <div className="list-note">
        <p>{tr('Non riesco a leggere le riunioni.', 'Meetings could not be read.')}</p>
        {onretry && (
          <Button variant="secondary" onClick={onretry}>
            <Icon name="refresh" size={14} />
            {tr('Riprova', 'Try again')}
          </Button>
        )}
      </div>
    );
  } else {
    content = (
      <>
        {GROUP_ORDER.map((name) =>
          groups[name].length ? (
            <div key={name} role="group" data-group={name} aria-labelledby={`meetings-group-${name}`}>
              <h2 className="group-label" id={`meetings-group-${name}`}>
                {name === 'live' && <span className="group-dot" aria-hidden="true"></span>}
                {labels[name]}
              </h2>
              <ul className="items">
                <Presence>
                  {groups[name].map((row) => (
                    <MeetingRowItem
                      key={row.key}
                      row={row}
                      selected={row.key === selectedKey}
                      live={live}
                      now={now}
                      canJoin={desktop}
                      joinBusy={joinBusy}
                      blocker={blocker}
                      transcribeBusy={transcribeBusy}
                      onselect={onselect}
                      onjoin={onjoin}
                      onsettings={onsettings}
                      ontranscribe={ontranscribe}
                    />
                  ))}
                </Presence>
              </ul>
            </div>
          ) : null,
        )}
        {empty && (
          <p className="list-note">
            {tr(
              'Qui compaiono le riunioni del calendario e le tue registrazioni. Per una riunione che non è in calendario, premi «Registra ora».',
              'Calendar meetings and your recordings show up here. For a meeting that is not in the calendar, press “Record now”.',
            )}
          </p>
        )}
        {onmore && (
          <div className="list-more">
            <Button variant="text" aria-disabled={moreBusy ? 'true' : undefined} onClick={onmore}>
              {moreBusy
                ? tr('Carico altre riunioni…', 'Loading more meetings…')
                : tr('Mostra altre riunioni', 'Show more meetings')}
            </Button>
          </div>
        )}
      </>
    );
  }

  return (
    <section
      className="meetings-column"
      aria-label={tr('Elenco riunioni', 'Meetings list')}
      data-list-ready={status === 'ready' ? '' : undefined}
    >
      <header className="list-top">
        <div className="list-top-row">
          <h1>{tr('Riunioni', 'Meetings')}</h1>
          {actions}
        </div>
        <div className="list-search">
          <span className="list-search-icon">
            <Icon name="search" size={15} />
          </span>
          <Input
            type="search"
            value={query}
            onChange={(event) => onquery(event.currentTarget.value)}
            aria-label={tr('Cerca nelle trascrizioni', 'Search transcripts')}
            placeholder={tr('Cerca nelle trascrizioni', 'Search transcripts')}
            autoComplete="off"
          />
        </div>
      </header>

      <div className="list-scroll">{content}</div>
    </section>
  );
}
