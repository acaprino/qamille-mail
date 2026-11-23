// The head of the detail: the title and, under it, when, how long and who (the guests of the
// linked calendar event), with the "more" menu on the right. A meeting being recorded or
// processed has no menu (there is nothing to rename or delete yet), and one to recover says so.
import Icon from '../../ui/Icon';
import { tr } from '../../lib/i18n';
import type { MeetingDetail } from '../../lib/meetings';
import { formatDateTime } from '../../lib/regional';
import type { CalendarEvent } from '../../lib/types';
import MeetingMenu from './MeetingMenu';
import './MeetingHeader.css';

export type MeetingHeaderProps = {
  detail: MeetingDetail;
  /** The calendar event the meeting is linked to: the guests, and a name when it has none. */
  event?: CalendarEvent | null;
  menu: boolean;
  toRecover?: boolean;
  notify: (message: string, error?: boolean) => void;
  onchange: (detail: MeetingDetail) => void;
  ondeleted: () => void;
};

function whenOf(seconds: number): string {
  const date = new Date(seconds * 1000);
  const sameYear = date.getFullYear() === new Date().getFullYear();
  const day = formatDateTime(seconds, {
    weekday: 'long',
    day: 'numeric',
    month: 'long',
    ...(sameYear ? {} : { year: 'numeric' }),
  });
  const time = formatDateTime(seconds, { hour: '2-digit', minute: '2-digit' });
  return `${day.charAt(0).toLocaleUpperCase()}${day.slice(1)}, ${time}`;
}

function lengthOf(durationSecs: number): string | null {
  if (durationSecs <= 0) return null;
  const minutes = Math.max(1, Math.round(durationSecs / 60));
  if (minutes < 60) return `${minutes} min`;
  return minutes % 60 ? `${Math.floor(minutes / 60)} h ${minutes % 60} min` : `${minutes / 60} h`;
}

/** The guests by the part of the address before the @: a few of them, and a count for the rest. */
function guestsOf(event: CalendarEvent | null): string {
  const names = (event?.attendees ?? []).map((who) => who.split('@')[0]);
  if (names.length <= 4) return names.join(', ');
  const more = names.length - 3;
  return `${names.slice(0, 3).join(', ')} ${tr(`e altri ${more}`, `and ${more} more`)}`;
}

export default function MeetingHeader({
  detail,
  event = null,
  menu,
  toRecover = false,
  notify,
  onchange,
  ondeleted,
}: MeetingHeaderProps) {
  const title = detail.meeting.title.trim() || event?.title || tr('Riunione senza nome', 'Untitled meeting');
  const when = whenOf(detail.meeting.date);
  const length = lengthOf(detail.meeting.duration_secs);
  const guests = guestsOf(event);

  return (
    <header className="detail-head">
      <div className="detail-top">
        <h2 className="detail-title">{title}</h2>
        {menu && <MeetingMenu detail={detail} title={title} notify={notify} onchange={onchange} ondeleted={ondeleted} />}
      </div>
      <div className="detail-meta">
        <span>
          <Icon name="calendar" size={15} />
          {when}
        </span>
        {length && (
          <span>
            <Icon name="clock" size={15} />
            {length}
          </span>
        )}
        {guests && (
          <span>
            <Icon name="contacts" size={15} />
            {guests}
          </span>
        )}
        {toRecover && <span className="tag">{tr('Da recuperare', 'To recover')}</span>}
      </div>
    </header>
  );
}
