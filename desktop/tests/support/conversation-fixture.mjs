// A conversation for the browser tests, added to a clone of the sample fixture. It is not in
// sample.json: other tests count that file's messages.
//
//   R  Maria   Inbox   the first message                                     read
//   A  Luca    Inbox   answers R                                             read
//   S  Alex    Sent    answers A (the user's own reply)                      read
//   B  Maria   Inbox   answers A, to twelve people                           UNREAD
//   C  Luca    Inbox   answers S, the newest: the main row of the list        read
//
// The tree below the main row (C) is R, then A under it, then S and B under A.
export const SUBJECT = 'Launch plan: who owns what?';
export const IDS = { root: 7001, a: 7002, sent: 7003, many: 7004, main: 7005, extra: 7100 };
export const ME = 'alex@studio-example.com';
export const MARIA = ['Maria Rossi', 'maria@client-example.com'];
export const LUCA = ['Luca Bianchi', 'luca@studio-example.com'];
export const TWELVE = [ME, MARIA[1], LUCA[1], ...Array.from({ length: 9 }, (_, i) => `guest${i + 1}@example.test`)];

const BASE = 1_793_626_000; // newer than every message of the sample

/**
 * `extraReplies` more messages answering A, so that the conversation passes the cap of 30.
 * `read` sets the read state of chosen messages by id, e.g. `{ [IDS.many]: true }`.
 */
export function withConversation(sample, { extraReplies = 0, read = {} } = {}) {
  const fixture = structuredClone(sample);
  const template = fixture.emails.find((e) => e.folder === 'Inbox');
  const account = fixture.accounts.find((a) => a.email === ME).id;
  // Keys are written without the domain; the ids carry it, as the provider writes them.
  const mail = (id, folder, [name, address], to, key, parent, offset, read, body, thread = parent || key) => ({
    ...structuredClone(template), id, account_id: account, folder, from_name: name, from_addr: address,
    to, cc: [], subject: SUBJECT, body, date: BASE + offset, read,
    message_id: `<${key}@test>`, in_reply_to: parent ? `${parent}@test` : '', thread_id: `${thread}@test`,
    attachments: [], labels: [], pinned: false, done: false, due: null, snoozed_until: null,
    set_aside: false, reminder_at: null, replied: false, category: 'Personal', returned_at: null,
    ...(folder === 'Sent' ? { category: 'Personal' } : {}),
  });
  const alex = ['Alex Example', ME];
  fixture.emails.push(
    mail(IDS.root, 'Inbox', MARIA, [ME], 'launch-r', '', 0, true, 'We need to decide who owns the launch plan.'),
    mail(IDS.a, 'Inbox', LUCA, [ME, MARIA[1]], 'launch-a', 'launch-r', 60, true, 'I can take the schedule. Maria, can you own the budget?', 'launch-r'),
    // A reply Qamille sent copies its parent's thread, one level too high: here the root's.
    mail(IDS.sent, 'Sent', alex, [LUCA[1], MARIA[1]], 'launch-s', 'launch-a', 120, true, 'Budget is with Maria, schedule with Luca. Agreed?', 'launch-r'),
    mail(IDS.many, 'Inbox', MARIA, TWELVE, 'launch-b', 'launch-a', 180, false, 'Yes. Adding the rest of the team so everyone sees it.', 'launch-a'),
    mail(IDS.main, 'Inbox', LUCA, [ME], 'launch-c', 'launch-s', 240, true, 'Agreed. I will send the schedule on Monday.', 'launch-s'),
  );
  for (let i = 0; i < extraReplies; i++) {
    fixture.emails.push(
      mail(IDS.extra + i, 'Inbox', LUCA, [ME], `launch-x${i}`, 'launch-a', 300 + i, true, `Extra reply number ${i + 1}.`, 'launch-a'),
    );
  }
  for (const email of fixture.emails) if (email.id in read) email.read = read[email.id];
  fixture.next_id = Math.max(fixture.next_id ?? 0, 100_000);
  return fixture;
}
