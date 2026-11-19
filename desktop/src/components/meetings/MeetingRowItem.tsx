// One row of the Meetings list. Two lines: when and title, then what there is to know (length and
// guests, the call service or place, how far the transcription is). On the right: the red dot and
// the time for the recording going on, a tag for what needs doing, "Partecipa e registra" for an
// event that has a call. A recording opens; an event does not (nothing is recorded to open), and
// one nobody recorded is grey. A recording that waits for its transcript while the engine cannot
// run says why, with a link to Settings; one whose transcription failed says so, with "Riprova".
//
// The row is a Motion <li>: inside the list's AnimatePresence it fades in when it arrives, slides
// (layout) when the rows above it change, and fades out when it leaves. While it fades out it is
// only a picture: no `data-row-key`, inert and hidden from the accessibility tree, so the row that
// took its place (the same meeting in another group) is the only one the page and its tests find.
import { motion, useIsPresent, useReducedMotion } from 'motion/react';
import type { ReactNode } from 'react';
import Button from '../../ds/primitives/Button';
import Icon from '../../ds/primitives/Icon';
import { backendText, tr } from '../../lib/i18n';
import { captureActive, stateLabel, type CaptureSnapshot } from '../../lib/meetings';
import { clockLabel, type MeetingRow } from '../../lib/meetings-view';
import { dateKey, formatDateTime } from '../../lib/regional';
import { cn } from '../../lib/utils';
import './MeetingRowItem.css';

export type MeetingRowItemProps = {
  row: MeetingRow;
  selected: boolean;
  live: CaptureSnapshot;
  /** Unix seconds, the same `now` the rows were built with. */
  now: number;
  canJoin: boolean;
  /** A join is on its way (this row's or another's): the button waits, and keeps the focus. */
  joinBusy?: boolean;
  /** Why the engine cannot transcribe now (from Settings), or `null`. */
  blocker?: string | null;
  /** A "Riprova" is on its way (this row's or another's): the button waits, and keeps the focus. */
  transcribeBusy?: boolean;
  onselect: (row: MeetingRow) => void;
  onjoin: (row: MeetingRow) => void;
  /** Opens Settings on Meetings. */
  onsettings: () => void;
  /** Starts a new transcription with the engine Settings says now. */
  ontranscribe: (row: MeetingRow) => void;
};

const spring = { type: 'spring', stiffness: 400, damping: 32 } as const;

function lengthOf(seconds: number | null): string | null {
  if (!seconds) return null;
  const minutes = Math.max(1, Math.round(seconds / 60));
  if (minutes < 60) return `${minutes} min`;
  return minutes % 60 ? `${Math.floor(minutes / 60)} h ${minutes % 60} min` : `${minutes / 60} h`;
}
const guests = (count: number | null) =>
  count ? tr(count === 1 ? '1 partecipante' : `${count} partecipanti`, count === 1 ? '1 guest' : `${count} guests`) : null;
/** The call service from its link; a link we do not know is a plain video call. */
function service(link: string | null): string | null {
  if (!link) return null;
  let host = '';
  try {
    host = new URL(link).hostname.toLowerCase();
  } catch {
    return tr('Videochiamata', 'Video call');
  }
  const is = (domain: string) => host === domain || host.endsWith(`.${domain}`);
  if (is('meet.google.com')) return 'Google Meet';
  if (is('zoom.us') || is('zoom.com')) return 'Zoom';
  if (is('teams.microsoft.com') || is('teams.live.com')) return 'Microsoft Teams';
  if (is('meet.jit.si') || is('jitsi.org')) return 'Jitsi';
  return tr('Videochiamata', 'Video call');
}
function startedAgo(seconds: number): string {
  const minutes = Math.max(1, Math.floor(seconds / 60));
  const text = minutes < 60 ? `${minutes} min` : lengthOf(minutes * 60);
  return tr(`Iniziata da ${text}`, `Started ${text} ago`);
}
const joinParts = (parts: (string | null)[]) => parts.filter(Boolean).join(' · ');

export default function MeetingRowItem({
  row,
  selected,
  live,
  now,
  canJoin,
  joinBusy = false,
  blocker = null,
  transcribeBusy = false,
  onselect,
  onjoin,
  onsettings,
  ontranscribe,
}: MeetingRowItemProps) {
  const still = useReducedMotion();
  const present = useIsPresent();

  /** The name of a meeting that has none is the same words the header of its detail uses. */
  const title = row.title.trim() || tr('Riunione senza nome', 'Untitled meeting');
  const mine = live.meeting_id === row.meetingId && captureActive(live);
  const when = (() => {
    if (row.group === 'live' || dateKey(row.start) === dateKey(now))
      return formatDateTime(row.start, { hour: '2-digit', minute: '2-digit' });
    // A past row from another year says which: "3 nov 2025" is not this November.
    const otherYear = dateKey(row.start).slice(0, 4) !== dateKey(now).slice(0, 4);
    return formatDateTime(
      row.start,
      row.group === 'upcoming'
        ? { weekday: 'short', day: 'numeric' }
        : { day: 'numeric', month: 'short', ...(otherYear ? { year: 'numeric' } : {}) },
    );
  })();
  const elapsed = clockLabel(mine ? live.elapsed_ms : (row.durationSecs ?? 0) * 1000);

  const sub = ((): string => {
    const length = lengthOf(row.durationSecs);
    switch (row.status) {
      case 'recording':
        return mine ? stateLabel(live.state) : tr('Registrazione in corso', 'Recording');
      case 'processing':
        if (!row.job) return tr('In elaborazione', 'Processing');
        return row.job.kind === 'transcribe'
          ? tr(`Trascrizione ${row.job.progress}%`, `Transcribing ${row.job.progress}%`)
          : tr('Sintesi…', 'Summarizing…');
      case 'upcoming':
        return joinParts([
          row.startedSecsAgo === null ? null : startedAgo(row.startedSecsAgo),
          row.meetingLink ? service(row.meetingLink) : row.place,
          guests(row.attendees),
        ]);
      case 'unrecorded':
        return tr('Non registrata', 'Not recorded');
      case 'interrupted':
        return tr('Registrazione interrotta', 'Recording interrupted');
      case 'to_transcribe':
        // The reason the engine cannot run is the line (Settings tells how to fix it).
        return blocker
          ? joinParts([length, backendText(blocker)])
          : joinParts([length, tr('audio da trascrivere', 'audio to transcribe')]);
      case 'empty':
        return joinParts([length, tr('trascrizione vuota', 'empty transcript')]);
      case 'failed':
        return tr('Trascrizione non riuscita', 'Transcription failed');
      default:
        return joinParts([length, guests(row.attendees)]);
    }
  })();
  const tag =
    row.status === 'interrupted'
      ? { text: tr('Da recuperare', 'To recover'), tone: 'danger' }
      : row.status === 'to_transcribe'
        ? { text: tr('Da trascrivere', 'To transcribe'), tone: 'warn' }
        : null;
  const showJoin = canJoin && row.status === 'upcoming' && Boolean(row.meetingLink);
  const showSettings = row.status === 'to_transcribe' && Boolean(blocker);
  const showRetry = row.status === 'failed';

  const body: ReactNode = (
    <>
      <span className="item-when">{when}</span>
      <span className="item-text">
        <span className="item-title">{title}</span>
        {sub && <span className={cn('item-sub', row.status === 'failed' && 'danger')}>{sub}</span>}
        {row.job && (
          <span
            className="item-bar"
            role="progressbar"
            aria-label={row.job.kind === 'transcribe' ? tr('Trascrizione', 'Transcription') : tr('Sintesi', 'Summary')}
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={row.job.progress}
          >
            <span style={{ width: `${row.job.progress}%` }}></span>
          </span>
        )}
      </span>
      {row.status === 'recording' ? (
        <span className="item-live">
          <span className="item-dot" aria-hidden="true"></span>
          <span className="item-elapsed">{elapsed}</span>
        </span>
      ) : tag ? (
        <span className={`item-tag ${tag.tone}`}>{tag.text}</span>
      ) : null}
    </>
  );

  return (
    <motion.li
      className={cn('item', selected && 'selected', row.status === 'unrecorded' && 'unrecorded')}
      data-row-key={present ? row.key : undefined}
      inert={!present || undefined}
      aria-hidden={present ? undefined : 'true'}
      layout={still ? false : 'position'}
      initial={still ? false : { opacity: 0, y: -6 }}
      animate={{ opacity: 1, y: 0 }}
      exit={still ? undefined : { opacity: 0, transition: { duration: 0.12 } }}
      transition={spring}
    >
      {row.openable ? (
        <button
          type="button"
          className="item-main"
          aria-current={selected ? 'true' : undefined}
          onClick={() => onselect(row)}
        >
          {body}
        </button>
      ) : (
        <div className="item-main">{body}</div>
      )}
      {showJoin ? (
        <div className="item-action">
          <Button
            variant={row.startedSecsAgo === null ? 'secondary' : 'primary'}
            className="item-join"
            aria-disabled={joinBusy ? 'true' : undefined}
            aria-label={tr(`Partecipa e registra: ${title}`, `Join and record: ${title}`)}
            onClick={() => onjoin(row)}
          >
            <Icon name="link" size={14} />
            {tr('Partecipa e registra', 'Join and record')}
          </Button>
        </div>
      ) : showSettings ? (
        <div className="item-action">
          <Button
            variant="text"
            aria-label={tr(`Apri Impostazioni › Riunioni: ${title}`, `Open Settings › Meetings: ${title}`)}
            onClick={onsettings}
          >
            {tr('Impostazioni', 'Settings')}
          </Button>
        </div>
      ) : showRetry ? (
        <div className="item-action">
          <Button
            variant="secondary"
            aria-disabled={transcribeBusy ? 'true' : undefined}
            aria-label={tr(`Riprova la trascrizione: ${title}`, `Try the transcription again: ${title}`)}
            onClick={() => ontranscribe(row)}
          >
            <Icon name="refresh" size={14} />
            {tr('Riprova', 'Try again')}
          </Button>
        </div>
      ) : null}
    </motion.li>
  );
}
