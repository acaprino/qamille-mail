// A state of the meeting detail that has something to say and, at most, a thing to do: the
// recording was interrupted, there is no transcript yet (and why), the transcript came out empty.
// A title, then the words (`text`), a boxed `message` (info, or danger for an error) and whatever
// else the state draws (`children`), then the buttons (`actions`) and a quiet `note` under them. The
// progress of a job is the same card with a bar in it (MeetingProgress). The card fades and rises
// in with a spring when the state appears (still under prefers-reduced-motion).
import { motion, useReducedMotion } from 'motion/react';
import type { ReactNode } from 'react';
import Icon from '../../ds/primitives/Icon';
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
  const still = useReducedMotion();
  return (
    <motion.div
      className={cn('state meeting-notice', className)}
      initial={still ? false : { opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ type: 'spring', stiffness: 400, damping: 32 }}
    >
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
    </motion.div>
  );
}
