import { useEffect, useRef, useState } from 'react';
import Button from '../ds/primitives/Button';
import Switch from '../ds/primitives/Switch';
import { call, errorText } from '../lib/api';
import { openExternal } from '../lib/external-link';
import { backendText, tr } from '../lib/i18n';
import { regionalLocale } from '../lib/regional';
import {
  GOOGLE_PERMISSIONS_URL,
  canTurnOn,
  removalText,
  retryable,
  revokeText,
  statusText,
  type MailboxRevoke,
  type SyncStatus,
} from '../lib/mailbox-sync';
import type { Account } from '../lib/types';
import './ComputeSettings.css';

export type MailboxSyncProps = {
  accounts: Account[];
  changed: () => void;
  notify: (message: string, error?: boolean) => void;
};

function lastTime(seconds: number): string {
  return new Date(seconds * 1000).toLocaleString(regionalLocale(), { dateStyle: 'short', timeStyle: 'short' });
}

/** Settings → Accounts → «Sincronizzazione delle caselle» (sync spec S9): the state, the general
 *  switch, the mailboxes removed on another computer and the switch of each mailbox. */
export default function MailboxSync({ accounts, changed, notify }: MailboxSyncProps) {
  const [status, setStatus] = useState<SyncStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [removing, setRemoving] = useState<number | null>(null);
  const props = useRef({ changed, notify });
  props.current = { changed, notify };

  async function load() {
    try {
      setStatus(await call<SyncStatus>('sync_status'));
    } catch {
      setStatus(null);
    }
  }
  useEffect(() => {
    void load();
    const timer = setInterval(() => void load(), 15000);
    return () => clearInterval(timer);
  }, [accounts.length]);

  async function run(work: () => Promise<void>) {
    setBusy(true);
    try {
      await work();
      await load();
    } catch (error) {
      props.current.notify(errorText(error), true);
    } finally {
      setBusy(false);
    }
  }

  const real = accounts.filter((account) => account.kind !== 'Sample');
  const text = statusText(status, lastTime);
  const switches = new Map((status?.mailboxes ?? []).map((box) => [box.account_id, box]));
  const active = !!status && status.mode !== 'no_server' && status.mode !== 'unavailable';

  return (
    <section className="mailbox-sync" aria-label={tr('Sincronizzazione delle caselle', 'Mailbox synchronization')}>
      <h3>{tr('Sincronizzazione delle caselle', 'Mailbox synchronization')}</h3>
      <p className="muted" role="status">
        {tr(text.it, text.en)}
      </p>
      {active && (
        <div className="setting-row">
          <Switch
            checked={!!status?.enabled && status.consented}
            disabled={busy || (!status?.enabled && !canTurnOn(status))}
            onChange={(on) => void run(() => call('sync_set_enabled', { enabled: on }))}
          >
            <strong>{tr('Sincronizza le caselle tra i miei computer', 'Sync my mailboxes between my computers')}</strong>
          </Switch>
          <p>
            {tr(
              'Le caselle, con le loro credenziali, stanno sul server per ritrovarle sugli altri tuoi computer.',
              'Your mailboxes, with their credentials, are kept on the server so you find them on your other computers.',
            )}
          </p>
        </div>
      )}
      {(status?.removals ?? []).map((notice) => {
        const line = removalText(notice);
        return (
          <div key={notice.item_id} className="mailbox-sync-notice" role="alert">
            <p>{tr(line.it, line.en)}</p>
            <div className="button-row">
              <Button
                variant="secondary"
                disabled={busy}
                onClick={() =>
                  void run(async () => {
                    await call('sync_resolve_removal', { itemId: notice.item_id, removeHere: true });
                    props.current.changed();
                  })
                }
              >
                {tr('Rimuovi anche qui', 'Remove here too')}
              </Button>
              <Button
                variant="text"
                disabled={busy}
                onClick={() => void run(() => call('sync_resolve_removal', { itemId: notice.item_id, removeHere: false }))}
              >
                {tr('Tienila solo qui', 'Keep it only here')}
              </Button>
            </div>
          </div>
        );
      })}
      {active &&
        real.map((account) => {
          const box = switches.get(account.id);
          return (
            <div key={account.id} className="setting-row mailbox-sync-row">
              <Switch
                checked={box?.enabled ?? true}
                disabled={busy || !status?.enabled}
                onChange={(on) => void run(() => call('sync_set_mailbox', { accountId: account.id, enabled: on }))}
              >
                <strong>{tr('Sincronizza questa casella', 'Sync this mailbox')}</strong> · {account.email}
              </Switch>
              {box?.needs_reconnect && (
                <p className="muted">
                  {tr('Accesso revocato: ricollega la casella con Google.', 'Access revoked: connect the mailbox again with Google.')}
                </p>
              )}
              {box?.problem && <p className="muted">{backendText(box.problem)}</p>}
              {removing === account.id ? (
                <div className="button-row">
                  <span>{tr('Rimuovere la casella e la posta scaricata da questo computer?', 'Remove the mailbox and its downloaded mail from this computer?')}</span>
                  <Button
                    variant="secondary"
                    disabled={busy}
                    onClick={() =>
                      void run(async () => {
                        await call('account_remove', { accountId: account.id });
                        setRemoving(null);
                        props.current.changed();
                      })
                    }
                  >
                    {tr('Rimuovi', 'Remove')}
                  </Button>
                  <Button variant="text" onClick={() => setRemoving(null)}>
                    {tr('Annulla', 'Cancel')}
                  </Button>
                </div>
              ) : (
                <Button variant="text" disabled={busy} onClick={() => setRemoving(account.id)}>
                  {tr('Rimuovi la casella', 'Remove the mailbox')}
                </Button>
              )}
            </div>
          );
        })}
    </section>
  );
}

/** What `sync_revoke_all` did, for the «Dispositivi» section. */
export function RevokeReport({
  results,
  onRetry,
  onError,
  busy,
}: {
  results: MailboxRevoke[];
  onRetry: () => void;
  /** The link to the Google account could not be opened. */
  onError: (message: string) => void;
  busy: boolean;
}) {
  if (results.length === 0) return null;
  return (
    <div className="mailbox-sync-notice" role="status">
      {results.map((result) => {
        const line = revokeText(result);
        return <p key={result.account_id}>{tr(line.it, line.en)}</p>;
      })}
      {retryable(results).length > 0 && (
        <div className="button-row">
          <Button variant="secondary" disabled={busy} onClick={onRetry}>
            {tr('Riprova', 'Try again')}
          </Button>
          <a href={GOOGLE_PERMISSIONS_URL} target="_blank" rel="noreferrer" onClick={(event) => void openExternal(event, onError)}>
            {tr('Permessi del tuo account Google', 'Permissions of your Google account')}
          </a>
        </div>
      )}
    </div>
  );
}
