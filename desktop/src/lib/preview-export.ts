// Sample-only browser exports. The desktop downloads original MIME from the provider.
import type { Email } from './types';

const clean = (value: string) => value.replace(/[\r\n]/g, ' ');
function base64(value: string) {
  const bytes = new TextEncoder().encode(value);
  let binary = '';
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}
const encoded = (value: string) => `=?UTF-8?B?${base64(clean(value))}?=`;
const content = (value: string) =>
  base64(value)
    .match(/.{1,76}/g)
    ?.join('\r\n') || '';

export function sampleEml(email: Email): string {
  const boundary = `qamille-${email.id}-${crypto.randomUUID()}`;
  const headers = [
    `From: ${encoded(email.from_name)} <${clean(email.from_addr)}>`,
    `To: ${email.to.map(clean).join(', ')}`,
    ...(email.cc.length ? [`Cc: ${email.cc.map(clean).join(', ')}`] : []),
    `Subject: ${encoded(email.subject)}`,
    `Date: ${new Date(email.date * 1000).toUTCString()}`,
    `Message-ID: <${clean(email.message_id || `${email.id}@sample.qamille`).replace(/[<>]/g, '')}>`,
    'MIME-Version: 1.0',
  ];
  const plain = [
    'Content-Type: text/plain; charset=utf-8',
    'Content-Transfer-Encoding: base64',
    '',
    content(email.body),
  ].join('\r\n');
  if (!email.attachments.length) return [...headers, plain, ''].join('\r\n');
  const parts = [
    plain,
    ...email.attachments.map((a) => {
      if (a.inline_text === null) throw new Error(`Attachment unavailable: ${a.name}`);
      return [
        `Content-Type: ${clean(a.mime) || 'application/octet-stream'}`,
        'Content-Transfer-Encoding: base64',
        `Content-Disposition: attachment; filename="${encoded(a.name)}"`,
        '',
        content(a.inline_text),
      ].join('\r\n');
    }),
  ];
  return [
    ...headers,
    `Content-Type: multipart/mixed; boundary="${boundary}"`,
    '',
    ...parts.flatMap((part) => [`--${boundary}`, part]),
    `--${boundary}--`,
    '',
  ].join('\r\n');
}

export function sampleThread(emails: Email[], anchor: Email): Email[] {
  const ids = new Set([anchor.id]);
  const keys = new Set<string>();
  const key = (value: string) => value.trim().replace(/^<|>$/g, '');
  let before = -1;
  while (before !== ids.size + keys.size) {
    before = ids.size + keys.size;
    for (const email of emails.filter((e) => e.account_id === anchor.account_id)) {
      const links = [key(email.thread_id), key(email.message_id)].filter(Boolean);
      if (ids.has(email.id) || links.some((value) => keys.has(value))) {
        ids.add(email.id);
        links.forEach((value) => keys.add(value));
      }
    }
  }
  return emails.filter((e) => ids.has(e.id)).sort((a, b) => a.date - b.date || a.id - b.id);
}

export function downloadSample(emails: Email[], format: 'eml' | 'mbox') {
  if (!emails.length || (format === 'eml' && emails.length !== 1))
    throw new Error('Select messages to export. Use MBOX for multiple messages.');
  const raw = emails.map(sampleEml);
  const data =
    format === 'eml'
      ? raw[0]
      : raw
          .map((message, i) => {
            const date = new Date(emails[i].date * 1000);
            const parts = date.toUTCString().split(' ');
            const stamp = `${parts[0].replace(',', '')} ${parts[2]} ${String(date.getUTCDate()).padStart(2, ' ')} ${parts[4]} ${parts[3]}`;
            const normalized = message.replace(/\r\n/g, '\n');
            const split = normalized.indexOf('\n\n');
            const escaped =
              normalized.slice(0, split + 2) +
              normalized.slice(split + 2).replace(/^(>*From )/gm, '>$1');
            return `From ${emails[i].from_addr.replace(/\s/g, '') || 'MAILER-DAEMON'} ${stamp}\n${escaped}\n`;
          })
          .join('');
  const filename =
    format === 'eml'
      ? `email-${emails[0].id}-${emails[0].subject.replace(/[\\/:*?"<>|\x00-\x1f]/g, '_').slice(0, 80) || 'message'}.eml`
      : `qamille-${emails.length}-messages.mbox`;
  const url = URL.createObjectURL(
    new Blob([data], { type: format === 'eml' ? 'message/rfc822' : 'application/mbox' }),
  );
  const link = document.createElement('a');
  link.href = url;
  link.download = filename;
  link.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
  return { count: emails.length, path: filename };
}
