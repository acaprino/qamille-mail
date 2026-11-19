// What the list shows of a conversation (spec 2026-11-07 §4.3). The replies arrive in tree order
// with the id of the message each one answers; the list draws them as plain rows, with no
// indentation, so the tree only matters for folding a reply and for naming who it answers.
import { addressOf, displayName, recipientView, type PersonIndex } from './people.ts';
import type { ConversationReply } from './types';

export interface ReplyRow {
  reply: ConversationReply;
  /** Other replies sit under this one, so it can be folded. */
  hasChildren: boolean;
  folded: boolean;
  /** While folded: how many replies it hides, and whether one of them is unread. */
  hidden: number;
  unreadHidden: boolean;
}

/** The replies on screen: all of them except what lies under a folded reply. */
export function visibleReplies(replies: ConversationReply[], folded: number[]): ReplyRow[] {
  const known = new Set(replies.map((reply) => reply.id));
  const children = new Map<number, ConversationReply[]>();
  for (const reply of replies) {
    // A parent that is not loaded yet leaves the reply a root.
    if (reply.parent_id === null || !known.has(reply.parent_id)) continue;
    children.set(reply.parent_id, [...(children.get(reply.parent_id) ?? []), reply]);
  }
  const under = (id: number): ConversationReply[] => {
    const found: ConversationReply[] = [];
    const seen = new Set<number>([id]);
    const pending = [...(children.get(id) ?? [])];
    while (pending.length) {
      const next = pending.pop()!;
      if (seen.has(next.id)) continue;
      seen.add(next.id);
      found.push(next);
      pending.push(...(children.get(next.id) ?? []));
    }
    return found;
  };
  const foldedIds = new Set(folded.filter((id) => known.has(id)));
  const hidden = new Set<number>();
  const beneath = new Map<number, ConversationReply[]>();
  for (const id of foldedIds) {
    const list = under(id);
    beneath.set(id, list);
    for (const reply of list) hidden.add(reply.id);
  }
  return replies
    .filter((reply) => !hidden.has(reply.id))
    .map((reply) => {
      const below = beneath.get(reply.id) ?? [];
      return {
        reply,
        hasChildren: children.has(reply.id),
        folded: foldedIds.has(reply.id) && children.has(reply.id),
        hidden: foldedIds.has(reply.id) ? below.length : 0,
        unreadHidden: foldedIds.has(reply.id) && below.some((item) => !item.read),
      };
    });
}

export interface AnswerTarget {
  /** The person answered, or the reader's own name for themself ("you"). */
  name: string;
  you: boolean;
  /** The other recipients of "To" (never Cc). */
  more: number;
  /** For the tooltip: up to twelve names of the recipients, and how many are left out. */
  names: string[];
  rest: number;
}

/**
 * Who a reply is written to: the author of the message it answers when they are among its
 * recipients, else its first recipient. A reply that answers the main message, or whose parent is
 * not loaded, is read against `mainAuthor` or, failing that, its first recipient.
 */
export function answerTarget(
  reply: ConversationReply,
  replies: ConversationReply[],
  mainAuthor: string,
  index: PersonIndex,
  self: Set<string>,
  youLabel: string,
): AnswerTarget | null {
  const seen = new Set<string>();
  const recipients: { value: string; address: string }[] = [];
  for (const value of reply.to) {
    const address = addressOf(value);
    if (address && !seen.has(address)) {
      seen.add(address);
      recipients.push({ value, address });
    }
  }
  if (!recipients.length) return null;
  const parent = reply.parent_id === null ? null : replies.find((item) => item.id === reply.parent_id);
  const author = parent ? addressOf(parent.from_addr) : reply.parent_id === null ? addressOf(mainAuthor) : '';
  const chosen = recipients.find((recipient) => recipient.address === author) ?? recipients[0];
  const you = self.has(chosen.address);
  const view = recipientView(reply.to, index, self, youLabel, 12);
  return {
    name: you ? youLabel : displayName(index, chosen.value),
    you,
    more: recipients.length - 1,
    names: view.shown,
    rest: view.more,
  };
}
