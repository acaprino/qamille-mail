export interface ComposerState {
  subject: string;
  recipient: string;
  status: string;
  dirty: boolean;
  busy: boolean;
}

export interface ComposerHandle {
  flush: () => Promise<boolean>;
  close: () => Promise<void>;
  isBusy: () => boolean;
}
import type { Draft } from './types';

// Copy message content into a fresh identity without copying provider/sync metadata.
export function copiedDraft(source: Draft, created: Draft): Draft {
  return {
    ...created,
    account_id: source.account_id,
    to: source.to,
    cc: source.cc,
    bcc: source.bcc,
    subject: source.subject,
    body: source.body,
    reply_to: source.reply_to,
    attachments: [...source.attachments],
    format: source.format,
    signature_id: source.signature_id,
    quote: source.quote,
    send_at: null,
  };
}

