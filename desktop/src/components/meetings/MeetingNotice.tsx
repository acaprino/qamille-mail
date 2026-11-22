// A state of the meeting detail that has something to say and, at most, a thing to do: the
// recording was interrupted, there is no transcript yet (and why), the transcript came out empty.
// A title, then the words (`text`), a boxed `message` (info, or danger for an error) and whatever
// else the state draws (`children`), then the buttons (`actions`) and a quiet `note` under them. The
// progress of a job is the same card with a bar in it (MeetingProgress).
import type { ReactNode } from 'react';
import { useAppear } from '../../ui/motion';
import Icon from '../../ui/Icon';
import { cn } from '../../lib/utils';
import './MeetingNotice.css';

export type MeetingNoticeProps = {
  title: string;
  text?: string;
  message?: string;
  tone?: 'info' | 'danger';
  note?: string;
  children?: ReactNode;
  actions?: ReactNode;
  /** Extra classes on the card (MeetingProgress scopes its own rules with one). */
  className?: string;
};

export default function MeetingNotice({
  title,
  text,
  message,
  tone = 'info',
  note,
  children,
  actions,
  className,
}: MeetingNoticeProps) {
  const appear = useAppear();
  return (
    <div ref={appear} className={cn('state meeting-notice', className)}>
      <h3>{title}</h3>
      {text && <p className="state-text">{text}</p>}
      {message && (
        <div className={cn('notice', tone)} role={tone === 'danger' ? 'alert' : undefined}>
          <Icon name={tone === 'danger' ? 'alert' : 'info'} size={16} />
          <span>{message}</span>
        </div>
      )}
      {children}
      {actions && <div className="state-actions">{actions}</div>}
      {note && <p className="state-note">{note}</p>}
    </div>
  );
}
