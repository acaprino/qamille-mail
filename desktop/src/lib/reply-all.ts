import { addressOf } from './people.ts';

/**
 * Who a reply to everyone goes to: the sender and the other people the message was sent to, then
 * the people in copy, without the reader's own addresses and without anybody twice.
 * `replyTo` is where a plain reply goes (the draft's `To`).
 */
export function replyAllRecipients(
  message: { to: string[]; cc: string[] },
  replyTo: string,
  self: Set<string>,
): { to: string; cc: string } {
  const seen = new Set<string>();
  const keep = (list: string[]) =>
    list.filter((value) => {
      const address = addressOf(value);
      if (!address || self.has(address) || seen.has(address)) return false;
      seen.add(address);
      return true;
    });
  const to = keep([...replyTo.split(','), ...message.to]);
  const cc = keep(message.cc);
  return { to: to.length ? to.join(', ') : replyTo, cc: cc.join(', ') };
}
