import type { ConversationReply, Email, EmailSummary } from './types';

/** Replies a list row carries; the rest are read on demand (`read_replies`). */
export const REPLY_CAP = 30;

// Match provider IDs and reply links, scoped to the owning mailbox.
export function conversationIds(emails: Email[]): Map<number, number> {
  const parents = emails.map((_, index) => index);
  const links = new Map<string, number>();
  function root(index: number): number {
    while (parents[index] !== index) {
      parents[index] = parents[parents[index]];
      index = parents[index];
    }
    return index;
  }
  emails.forEach((email, index) => {
    for (const value of [email.message_id, email.thread_id]) {
      const link = value.trim().replace(/^<|>$/g, '');
      if (!link) continue;
      const key = `${email.account_id}:${link}`;
      const other = links.get(key);
      if (other === undefined) links.set(key, index);
      else {
        const a = root(index);
        const b = root(other);
        parents[Math.max(a, b)] = Math.min(a, b);
      }
    }
  });
  return new Map(emails.map((email, index) => [email.id, emails[root(index)].id]));
}

export function groupConversations(items: EmailSummary[], emails: Email[]): EmailSummary[] {
  const ids = conversationIds(emails);
  const groups = new Map<number, EmailSummary>();
  const order = ['Pins', 'Priority', 'New', 'Notifications', 'Newsletters', 'Seen'];
  for (const item of items) {
    const id = ids.get(item.id) ?? item.id;
    const existing = groups.get(id);
    if (!existing) {
      groups.set(id, { ...item, message_ids: [...item.message_ids], labels: [...item.labels] });
      continue;
    }
    existing.message_ids.push(item.id);
    existing.read &&= item.read;
    existing.pinned ||= item.pinned;
    existing.done &&= item.done;
    existing.attachment_count += item.attachment_count;
    existing.labels = [...new Set([...existing.labels, ...item.labels])];
    if (order.indexOf(item.section) < order.indexOf(existing.section))
      existing.section = item.section;
  }
  return [...groups.values()];
}

// ---- The other messages of a conversation, as a tree flattened for the list (ADR-0010) ----
// Mirrors app/src/domain/conversation.rs (`reply_tree`): the same rules in the same order, so the
// preview and the backend agree.

const messageKey = (value: string) => value.trim().replace(/^<+|>+$/g, '');

// The parent a message names. Mail saved before the parent was stored (empty, or absent in the
// preview's data) gets it the way the backend reads it: received mail takes it from `thread_id`,
// a sent reply has none.
function namedParent(email: Email): string {
  const stored = messageKey(email.in_reply_to ?? '');
  if (stored) return stored;
  const thread = messageKey(email.thread_id);
  return email.folder !== 'Sent' && thread && thread !== messageKey(email.message_id) ? thread : '';
}

const byDate = (a: Email, b: Email) => a.date - b.date || a.id - b.id;

function toReply(email: Email, parent: number | null): ConversationReply {
  return {
    id: email.id,
    account_id: email.account_id,
    parent_id: parent,
    from_name: email.from_name,
    from_addr: email.from_addr,
    to: email.to,
    snippet: email.body.replace(/\s+/g, ' ').slice(0, 140),
    date: email.date,
    read: email.read,
    folder: email.folder,
  };
}

/**
 * The other messages of the conversation that holds `mainId`, in tree order: every message of the
 * conversation except the main one, copies of it, Trash and Spam (unless that folder is the open
 * view, which holds its own members: ADR-0010 INV-004 hides them from the live views only). A
 * message hangs under the one it answers; when that is not there it hangs under the oldest message
 * of the conversation, if that one is listed, and is a root otherwise. A reply cycle opens at its
 * oldest message.
 */
export function replyRows(emails: Email[], mainId: number, viewFolder = ''): ConversationReply[] {
  const main = emails.find((email) => email.id === mainId);
  if (!main) return [];
  const hidden = (folder: string) =>
    (folder === 'Trash' || folder === 'Spam') && folder.toLowerCase() !== viewFolder.toLowerCase();
  const groups = conversationIds(emails);
  const group = emails.filter(
    (email) => email.account_id === main.account_id && groups.get(email.id) === groups.get(mainId),
  );
  const mainKey = messageKey(main.message_id);
  const oldest = [...group].sort(byDate)[0]?.id;

  const seen = new Set<string>();
  const candidates = group
    .filter((email) => email.id !== main.id && !hidden(email.folder))
    .filter((email) => {
      const key = messageKey(email.message_id);
      return !key || key !== mainKey;
    })
    .sort((a, b) => a.id - b.id)
    .filter((email) => {
      const key = messageKey(email.message_id);
      return !key || (!seen.has(key) && !!seen.add(key));
    })
    .sort(byDate);
  const byId = new Map(candidates.map((email) => [email.id, email]));
  const byKey = new Map(
    candidates.filter((email) => messageKey(email.message_id)).map((email) => [messageKey(email.message_id), email.id]),
  );

  const parent = new Map<number, number | null>();
  for (const email of candidates) {
    const named = namedParent(email);
    let direct: number | null | undefined;
    if (named && named === mainKey) direct = null;
    else if (named) {
      const id = byKey.get(named);
      direct = id !== undefined && id !== email.id ? id : undefined;
    }
    parent.set(
      email.id,
      direct !== undefined ? direct : oldest !== undefined && oldest !== email.id && byId.has(oldest) ? oldest : null,
    );
  }

  for (const email of candidates) {
    const path = [email.id];
    let current = email.id;
    for (;;) {
      const next = parent.get(current);
      if (next == null) break;
      const at = path.indexOf(next);
      if (at >= 0) {
        const lowest = path.slice(at).reduce((a, b) => (byDate(byId.get(a)!, byId.get(b)!) <= 0 ? a : b));
        parent.set(lowest, null);
        break;
      }
      path.push(next);
      current = next;
    }
  }

  const children = new Map<number | null, number[]>();
  for (const email of candidates) {
    const key = parent.get(email.id) ?? null;
    children.set(key, [...(children.get(key) ?? []), email.id]);
  }
  const out: ConversationReply[] = [];
  const visited = new Set<number>();
  const stack = [...(children.get(null) ?? [])].reverse();
  while (stack.length) {
    const id = stack.pop()!;
    if (visited.has(id)) continue;
    visited.add(id);
    out.push(toReply(byId.get(id)!, parent.get(id) ?? null));
    stack.push(...[...(children.get(id) ?? [])].reverse());
  }
  // Opening each cycle makes every message reachable; should a case slip through, it still shows.
  for (const email of candidates) if (!visited.has(email.id)) out.push(toReply(email, parent.get(email.id) ?? null));
  return out;
}
