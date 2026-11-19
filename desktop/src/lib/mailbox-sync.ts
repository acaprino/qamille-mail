// The mailboxes that follow the person between computers (wave 7, sync spec S6, S9). Pure
// functions without the interface language so they can be tested alone: texts are `{ it, en }`
// pairs the components pass to `tr`. Nothing here ever sees a credential: the backend keeps them
// in the vault and the status carries ids and switches only.
import type { ServerInfo } from './server-connect';

export type Text = { it: string; en: string };

export type SyncMode = 'off' | 'no_server' | 'unavailable' | 'idle' | 'paused_for_subscription' | 'error';

export interface RemovalNotice {
  item_id: string;
  name: string;
  email: string;
}
export interface MailboxSwitch {
  account_id: number;
  enabled: boolean;
  needs_reconnect: boolean;
  /** Why the mailbox is not synchronized although it is switched on, in the backend's words. */
  problem?: string | null;
}
/** `sync_status`. */
export interface SyncStatus {
  mode: SyncMode;
  enabled: boolean;
  consented: boolean;
  /** Unix seconds of the last pass that reached the server. */
  last_sync: number | null;
  error: string | null;
  removals: RemovalNotice[];
  mailboxes: MailboxSwitch[];
}
export type RevokeOutcome = 'revoked' | 'failed' | 'manual_password';
/** `sync_revoke_mailbox` and `sync_revoke_all`. */
export interface MailboxRevoke {
  account_id: number;
  email: string;
  provider_id: string;
  outcome: RevokeOutcome;
}

/** Where a person can revoke by hand what Qamille was allowed to do with a Google account. */
export const GOOGLE_PERMISSIONS_URL = 'https://myaccount.google.com/permissions';
export const GOOGLE_APP_PASSWORDS_URL = 'https://myaccount.google.com/apppasswords';

/** The sentence of «Before you connect» about the mailboxes; null when the server has no sync. */
export function consentLine(info: Pick<ServerInfo, 'sync'> | null | undefined): Text | null {
  if (!info?.sync?.available) return null;
  return {
    it: 'Le caselle che colleghi, con le loro credenziali, vengono salvate su questo server per ritrovarle sugli altri tuoi computer. Chi gestisce il server può accedervi.',
    en: 'The mailboxes you connect, with their credentials, are saved on this server so you find them on your other computers. Whoever runs the server can access them.',
  };
}

/** One line for the state of the synchronization; `time` formats Unix seconds in the person's locale. */
export function statusText(status: SyncStatus | null, time: (seconds: number) => string = (s) => String(s)): Text {
  if (!status || status.mode === 'no_server' || status.mode === 'unavailable') {
    return {
      it: 'La sincronizzazione passa da un server Qamille.',
      en: 'Synchronization goes through a Qamille server.',
    };
  }
  switch (status.mode) {
    case 'off':
      return {
        it: 'Spenta: le caselle restano solo su questo computer.',
        en: 'Off: your mailboxes stay on this computer only.',
      };
    case 'paused_for_subscription':
      return {
        it: "La sincronizzazione riprende con l'abbonamento attivo. Le caselle già presenti continuano a funzionare.",
        en: 'Synchronization resumes with an active subscription. The mailboxes already here keep working.',
      };
    case 'error':
      return {
        it: 'Non riesco ad allinearmi con il server. Riprovo da solo.',
        en: "Couldn't synchronize with the server. It will try again by itself.",
      };
    default:
      return status.last_sync === null
        ? { it: 'Attiva. Primo allineamento in corso.', en: 'On. First synchronization in progress.' }
        : { it: `Attiva. Ultimo allineamento: ${time(status.last_sync)}.`, en: `On. Last synchronized: ${time(status.last_sync)}.` };
  }
}

/** Whether the general switch can be turned on from here. */
export function canTurnOn(status: SyncStatus | null): boolean {
  return !!status && (status.mode === 'off' || status.mode === 'idle' || status.mode === 'error' || status.mode === 'paused_for_subscription') && status.consented;
}

export function removalText(notice: RemovalNotice): Text {
  return {
    it: `La casella ${notice.email} è stata rimossa o esclusa dalla sincronizzazione su un altro computer.`,
    en: `The mailbox ${notice.email} was removed or switched off on another computer.`,
  };
}

/** What happened to the access of a mailbox that was asked to be revoked. */
export function revokeText(result: MailboxRevoke): Text {
  switch (result.outcome) {
    case 'revoked':
      return {
        it: `${result.email}: accesso revocato. Ricollega la casella su questo computer: il nuovo accesso arriva agli altri.`,
        en: `${result.email}: access revoked. Connect the mailbox again on this computer: the new access reaches the others.`,
      };
    case 'failed':
      return {
        it: `${result.email}: revoca non riuscita. Google non ha risposto. Riprova, o revoca l'accesso dalla pagina dei permessi del tuo account Google.`,
        en: `${result.email}: revocation not done. Google did not answer. Try again, or revoke the access from the permissions page of your Google account.`,
      };
    default:
      return {
        it: `${result.email}: la password per app la revochi o la cambi presso il fornitore della casella${result.provider_id === 'gmail' ? ' (Account Google → Password per le app)' : ''}.`,
        en: `${result.email}: revoke or change the app password with the mailbox provider${result.provider_id === 'gmail' ? ' (Google Account → App passwords)' : ''}.`,
      };
  }
}

/** The results that need the person's attention again: a failure can be retried. */
export function retryable(results: MailboxRevoke[]): MailboxRevoke[] {
  return results.filter((result) => result.outcome === 'failed');
}

/** «Revoca gli accessi alle caselle»: ticked for another device, absent for this one. */
export function revokeChoice(device: { current: boolean }): { shown: boolean; checked: boolean } {
  return { shown: !device.current, checked: !device.current };
}

/** A pass of the synchronization changed mailboxes: the lists must be read again. */
export function mailboxesChanged(event: { kind: string }): boolean {
  return event.kind === 'accounts-changed';
}
