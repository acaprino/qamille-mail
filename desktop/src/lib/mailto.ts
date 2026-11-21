// mailto: links (RFC 6068) in Qamille (docs/aspetto-nativo.md, rule 32, audit Q10): the one parser, and
// the one way a link reaches the composer, whether it was clicked in a message (MessageHtml.tsx), in the
// app's own page (native_feel.rs sends it back as `compose-mailto`) or in another program (Windows starts
// Qamille with it: src-tauri/src/launch_args.rs). A link is untrusted input: it fills a new message and
// never sends one; it never attaches a file and never sets a header beyond To, Cc, Bcc, Subject and the
// text. No Tauri and no app store here, so `node --test tests/mailto.mjs` loads it as it is.
import type { Draft } from './types';

/** What a link fills in: the addresses of each field, the subject on one line, the text. */
export interface MailtoFields {
  to: string[];
  cc: string[];
  bcc: string[];
  subject: string;
  body: string;
}

/** The longest address that is read, in UTF-8 bytes (`MAIL_LINK_LIMIT` in launch_args.rs). */
export const MAILTO_LIMIT = 8 * 1024;

/** Whether `href` is a mail link, by its scheme. */
export function isMailto(href: string): boolean {
  return /^mailto:/i.test(href);
}

/** The composer's own check on an address: what RecipientInput.tsx marks as invalid fails here. */
export function validAddress(email: string): boolean {
  return /^[^\s@]+@[^\s@]+$/.test(email);
}

// A control character in a header would end it or start another one: a line break, a NUL.
const CONTROL = /[\u0000-\u001f\u007f]/;
const CONTROL_BUT_TAB = /[\u0000-\u0008\u000a-\u001f\u007f]/g;
const CONTROL_BUT_TAB_AND_LINE = /[\u0000-\u0008\u000b-\u001f\u007f]/g;

/** Percent-decoding as RFC 6068 has it: `+` is a plus, never a space. `null` for an escape that is
 *  not one (`%`, `%E0%A4%A`): nothing is guessed. */
function decode(text: string): string | null {
  try {
    return decodeURIComponent(text);
  } catch {
    return null;
  }
}

/** The addresses of one field, separated by commas or semicolons outside quotes, as the composer splits
 *  them. A line break makes the address it is in invalid (it becomes a space), so it cannot start a
 *  header or another address; angle brackets (a name with the address) are not an address of a link. */
function addresses(text: string): string[] {
  const found: string[] = [];
  let current = '';
  let quoted = false;
  for (const char of text) {
    if (char === '"') quoted = !quoted;
    if ((char === ',' || char === ';') && !quoted) {
      found.push(current);
      current = '';
    } else current += char;
  }
  found.push(current);
  return found
    .map((entry) => entry.replace(/[\r\n]/g, ' ').trim())
    .filter((entry) => entry !== '' && validAddress(entry) && !CONTROL.test(entry) && !/[<>]/.test(entry));
}

function add(list: string[], text: string) {
  for (const address of addresses(text)) {
    if (!list.some((known) => known.toLowerCase() === address.toLowerCase())) list.push(address);
  }
}

/** The fields of a `mailto:` address, or `null` when it is not one: another scheme, more than
 *  `MAILTO_LIMIT` bytes, or an escape that does not decode. The recipients come from the address
 *  (`mailto:a@b.it,c@d.it`) and from the `to`, `cc` and `bcc` headers, each checked as the composer
 *  checks it (an address that fails is left out); `subject` and `body` count once, the first time.
 *  Every other header (`attach`, `from`, `in-reply-to`, …) is ignored. A line break in the subject
 *  becomes a space, so it stays on one line; the text keeps its lines. */
export function parseMailto(href: string): MailtoFields | null {
  if (!isMailto(href) || new TextEncoder().encode(href).length > MAILTO_LIMIT) return null;
  // RFC 6068 has no fragment ("#" is written %23): what a URL parser would take for one is dropped.
  const rest = href.slice('mailto:'.length).split('#', 1)[0];
  const question = rest.indexOf('?');
  const path = decode(question < 0 ? rest : rest.slice(0, question));
  if (path === null) return null;
  const fields: MailtoFields = { to: [], cc: [], bcc: [], subject: '', body: '' };
  add(fields.to, path);
  let subject: string | null = null;
  let body: string | null = null;
  for (const pair of question < 0 ? [] : rest.slice(question + 1).split('&')) {
    if (pair === '') continue;
    const equals = pair.indexOf('=');
    const name = decode(equals < 0 ? pair : pair.slice(0, equals));
    const value = decode(equals < 0 ? '' : pair.slice(equals + 1));
    if (name === null || value === null) return null;
    switch (name.toLowerCase()) {
      case 'to':
        add(fields.to, value);
        break;
      case 'cc':
        add(fields.cc, value);
        break;
      case 'bcc':
        add(fields.bcc, value);
        break;
      case 'subject':
        subject ??= value;
        break;
      case 'body':
        body ??= value;
        break;
    }
  }
  fields.subject = (subject ?? '').replace(/[\r\n]+/g, ' ').replace(CONTROL_BUT_TAB, '').trim();
  fields.body = (body ?? '').replace(/\r\n?/g, '\n').replace(CONTROL_BUT_TAB_AND_LINE, '');
  return fields;
}

/** What `composeMailto` (App.tsx) saves into a new draft: the addresses as the composer keeps them, and
 *  the text as plain text, which the composer shows as it shows any plain-text draft. */
export function mailtoDraft(fields: MailtoFields): Pick<Draft, 'to' | 'cc' | 'bcc' | 'subject' | 'body' | 'format'> {
  return {
    to: fields.to.join(', '),
    cc: fields.cc.join(', '),
    bcc: fields.bcc.join(', '),
    subject: fields.subject,
    body: fields.body,
    format: 'Plain',
  };
}

type ComposeHandler = (fields: MailtoFields) => void;
const composers = new Set<ComposeHandler>();
let waiting: MailtoFields[] = [];

/** A mail link of the page (a message's) asks for the composer. Before anyone listens (the app has not
 *  mounted yet) the request waits for the first listener. */
export function requestCompose(fields: MailtoFields): void {
  if (composers.size === 0) {
    waiting.push(fields);
    return;
  }
  for (const compose of composers) compose(fields);
}

/** Listens to `requestCompose`, and gets what waited. Returns the way to stop. */
export function onComposeRequest(handler: ComposeHandler): () => void {
  composers.add(handler);
  const queued = waiting;
  waiting = [];
  for (const fields of queued) handler(fields);
  return () => {
    composers.delete(handler);
  };
}

type Listen = (event: string, handler: (event: { payload: string }) => void) => Promise<() => void>;
type Call = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;

export interface MailtoRouting {
  /** Opens a new message with the link's fields (`composeMailto` in App.tsx). */
  compose: (fields: MailtoFields) => void;
  /** A link that is not one: over the limit, an escape that does not decode, another scheme. */
  refused: () => void;
  failed: (error: unknown) => void;
  /** The desktop app's links from outside the page: `compose-mailto` from Rust, and the link Windows
   *  started Qamille with (`take_launch_mailto`). The browser preview has neither. */
  native?: { listen: Listen; call: Call };
}

/** Every mail link to the composer. The link of the start is asked for once, and only after the listener
 *  of `compose-mailto` is in: Rust keeps a link until the page has asked (`launch_args.rs`), then sends
 *  each one as an event. Returns the way to stop. */
export function installMailtoRouting({ compose, refused, failed, native }: MailtoRouting): () => void {
  const open = (href: string | null) => {
    if (href === null) return;
    const fields = parseMailto(href);
    if (fields) compose(fields);
    else refused();
  };
  const stops = [onComposeRequest(compose)];
  let stopped = false;
  if (native) {
    void native
      .listen('compose-mailto', (event) => open(event.payload))
      .then(async (unlisten) => {
        // A page that went before the listener was in (a reload) leaves the link to the next one.
        if (stopped) return unlisten();
        stops.push(unlisten);
        open(await native.call<string | null>('take_launch_mailto'));
      })
      .catch(failed);
  }
  return () => {
    stopped = true;
    for (const stop of stops) stop();
  };
}
