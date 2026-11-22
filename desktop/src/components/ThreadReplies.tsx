import { useState } from 'react';
import Icon from '../ui/Icon';
import { fullDate, shortDate } from '../lib/api';
import { conversationOf, repliesLeft, repliesOf, threadState } from '../lib/groups';
import { fixedBackendText, tr } from '../lib/i18n';
import type { PersonIndex } from '../lib/people';
import { useStore } from '../lib/store';
import { answerTarget, visibleReplies } from '../lib/thread-tree';
import { cn } from '../lib/utils';
import type { ConversationReply, EmailSummary } from '../lib/types';

export type ThreadRepliesProps = {
  row: EmailSummary;
  index: PersonIndex;
  self: Set<string>;
  selectedId: number | null;
  /** The one row of the list that takes the Tab key; the arrow keys move between the rest. */
  tabId: number | null;
  open: (id: number) => void;
  more: (offset: number) => Promise<ConversationReply[]>;
};

// The fold button keeps one name; what the next press does, how many replies it hides and
// whether one of them is unread travel in its description (and its tooltip), never in color alone.
function foldTip(item: { folded: boolean; hidden: number; unreadHidden: boolean }) {
  if (!item.folded) return tr('Nascondi le risposte a questo messaggio', 'Hide the replies to this message');
  const shown = tr(`Mostra le ${item.hidden} risposte nascoste`, `Show the ${item.hidden} hidden replies`);
  return item.unreadHidden ? `${shown}${tr(', con messaggi non letti', ', with unread messages')}` : shown;
}
// Names of everyone it was sent to, at most twelve, for the tooltip.
function tip(names: string[], rest: number) {
  return rest > 0 ? `${names.join(', ')}, ${tr(`e altri ${rest}`, `and ${rest} more`)}` : names.join(', ');
}

export default function ThreadReplies({ row, index, self, selectedId, tabId, open, more }: ThreadRepliesProps) {
  useStore(threadState);
  const key = conversationOf(row);
  const all = repliesOf(row);
  const rows = visibleReplies(all, threadState.folded[key] ?? []);
  const left = repliesLeft(row);
  const [loading, setLoading] = useState(false);
  const you = tr('te', 'you');

  function toggleFold(id: number) {
    const folded = threadState.folded[key] ?? [];
    threadState.folded[key] = folded.includes(id) ? folded.filter((value) => value !== id) : [...folded, id];
  }
  async function showMore() {
    if (loading) return;
    setLoading(true);
    try {
      const next = await more(repliesOf(row).length);
      threadState.more[key] = [...(threadState.more[key] ?? []), ...next];
    } catch {
      // Nothing was added: the button stays for another try.
    } finally {
      setLoading(false);
    }
  }

  return (
    <div
      className="thread-replies"
      role="group"
      aria-label={tr('Risposte in questa conversazione', 'Replies in this conversation')}
      data-conversation={key}
    >
      {rows.map((item) => {
        const reply = item.reply;
        const when = shortDate(reply.date);
        const target = answerTarget(reply, all, row.from_addr, index, self, you);
        return (
          <div
            key={reply.id}
            className={cn(
              'reply-row',
              selectedId === reply.id && 'selected',
              !reply.read && 'unread',
              item.hasChildren && 'has-kids',
            )}
            style={{ '--time-ch': when.length } as React.CSSProperties}
            data-reply-id={reply.id}
          >
            <button
              className="reply-open"
              tabIndex={tabId === reply.id ? 0 : -1}
              aria-pressed={selectedId === reply.id}
              title={fullDate(reply.date)}
              onClick={() => open(reply.id)}
            >
              <span className="visually-hidden">
                {tr('Apri', 'Open')}
                {!reply.read && tr(', non letto', ', unread')}
              </span>{' '}
              <span className="reply-content">
                <span className="reply-head">
                  <strong>{reply.from_name || reply.from_addr}</strong>{' '}
                  {target && (
                    <span className={cn('reply-to', target.you && 'to-you')} title={tip(target.names, target.rest)}>
                      <span className="to-label">{tr('a', 'to')}</span> <span className="to-first">{target.name}</span>
                      {target.more > 0 && (
                        <>
                          {' '}
                          <span className="to-more">+{target.more}</span>
                        </>
                      )}
                    </span>
                  )}{' '}
                  <time>{when}</time>
                </span>{' '}
                <span className="reply-snippet">{fixedBackendText(reply.snippet)}</span>
              </span>{' '}
              {!reply.read && <span className="unread-dot" aria-hidden="true"></span>}
            </button>{' '}
            {item.hasChildren && (
              <button
                className={cn('reply-fold', item.unreadHidden && 'has-unread')}
                tabIndex={tabId === reply.id ? 0 : -1}
                aria-expanded={!item.folded}
                aria-label={tr('Risposte a questo messaggio', 'Replies to this message')}
                title={foldTip(item)}
                onClick={() => toggleFold(reply.id)}
              >
                <Icon name="down" size={14} />
                {item.folded && <span className="fold-count">{item.hidden}</span>}
              </button>
            )}
          </div>
        );
      })}
      {left > 0 && (
        <button className="reply-more" disabled={loading} onClick={() => void showMore()}>
          {left === 1
            ? tr('Mostra un’altra risposta', 'Show 1 more reply')
            : tr(`Mostra altre ${left} risposte`, `Show ${left} more replies`)}
        </button>
      )}
    </div>
  );
}
