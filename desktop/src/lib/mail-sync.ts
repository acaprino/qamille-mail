import type { Account } from './types';

/** Mirror of `MailSyncStatus` in app/src/application/mail_sync_status.rs: the retained last result per account. */
export interface MailSyncStatus {
  revision: number;
  accounts: { account_id: number; phase: 'syncing' | 'complete' | 'error'; added: number; errors: string[] }[];
}

/**
 * The sync errors worth an alert: the retained errors of the connected accounts plus the
 * failure of the last manual request, each once. The sync phase itself is not read from the
 * status: it reaches the user only through the activity indicator (ADR-0007 decision 7,
 * INV-009), fed by the typed `sync-progress` events.
 */
export function syncErrors(accounts: Account[], status: MailSyncStatus, requestError = ''): string[] {
  const connected = accounts.filter((account) => account.kind === 'Gmail');
  const states = status.accounts.filter((state) => connected.some((account) => account.id === state.account_id));
  return [...new Set([...states.flatMap((state) => state.errors), ...(requestError ? [requestError] : [])])];
}
