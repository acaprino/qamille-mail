import { reactive } from './store';
import type { ConversationReply, EmailSummary } from './types';

// Which inbox groups are closed. It lives here, not in the list component, so what the reader
// opened or closed survives leaving the mail and coming back, and a change of folder or page.
// The quiet groups start closed; a closed group still shows if it holds something unread.
export const groupState = reactive<{ collapsed: string[] }>({
  collapsed: ['Notifications', 'Newsletters', 'Seen'],
});

// What the reader did with the conversations in the list, keyed by `conversation_id` so that new
// mail (which changes the message a row stands for, not the conversation) leaves it all alone.
export const threadState = reactive<{
  /** Conversations opened with their arrow; they start closed. */
  open: Record<number, boolean>;
  /** Replies folded in an open conversation: everything under each one is hidden. */
  folded: Record<number, number[]>;
  /** Replies loaded with "Show more", beyond the ones the list brought. */
  more: Record<number, ConversationReply[]>;
  /** What the list last brought for each conversation: tells when `more` has gone out of date. */
  seen: Record<number, string>;
}>({ open: {}, folded: {}, more: {}, seen: {} });

const signature = (row: EmailSummary) =>
  `${row.replies_more ?? 0}:${(row.replies ?? []).map((reply) => reply.id).join(',')}`;

/**
 * What "Show more" loaded is a copy; the list keeps being refreshed, and a message that was
 * trashed, moved or answered since changes which replies the list brings and how many are left.
 * When that happens the copy is dropped, and "Show more" offers the rest again, so a reply the
 * data no longer holds never stays on screen (ADR-0010 INV-004).
 */
export function reconcileThreads(rows: EmailSummary[]) {
  for (const row of rows) {
    const key = conversationOf(row);
    if (!key) continue;
    const now = signature(row);
    if (threadState.seen[key] !== undefined && threadState.seen[key] !== now) delete threadState.more[key];
    threadState.seen[key] = now;
  }
}

/** A reply loaded with "Show more" is read or unread like any other: keep its row and dot true. */
export function patchExtraRead(id: number, read: boolean) {
  for (const key of Object.keys(threadState.more).map(Number)) {
    const list = threadState.more[key];
    if (list.some((reply) => reply.id === id && reply.read !== read))
      threadState.more[key] = list.map((reply) => (reply.id === id ? { ...reply, read } : reply));
  }
}

/** The conversation a row stands for, 0 outside the Conversations mode. */
export const conversationOf = (row: EmailSummary) => row.conversation_id ?? 0;

/** Every reply of a row that is known: the ones the list brought and the ones loaded since. */
export function repliesOf(row: EmailSummary): ConversationReply[] {
  const listed = row.replies ?? [];
  const extra = threadState.more[conversationOf(row)] ?? [];
  if (!extra.length) return listed;
  const known = new Set(listed.map((reply) => reply.id));
  return [...listed, ...extra.filter((reply) => !known.has(reply.id))];
}

/** How many replies the list left out and nobody has loaded yet. */
export function repliesLeft(row: EmailSummary): number {
  const loaded = repliesOf(row).length - (row.replies?.length ?? 0);
  return Math.max(0, (row.replies_more ?? 0) - loaded);
}

/**
 * Whether a reply of the conversation is unread: one on screen, or one the list left out and the
 * backend says is unread. Once "Show more" has loaded the rest, only what is loaded counts.
 */
export const hasUnreadReplies = (row: EmailSummary) =>
  repliesOf(row).some((reply) => !reply.read) || (repliesLeft(row) > 0 && !!row.replies_more_unread);

/** Whether the row's conversation is open on screen. */
export const isThreadOpen = (row: EmailSummary) =>
  !!row.replies?.length && !!threadState.open[conversationOf(row)];
