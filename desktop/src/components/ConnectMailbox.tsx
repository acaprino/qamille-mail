// The first screen of an app with no mailbox yet: one button that connects the Gmail of the Qamille account
// (its address comes from the server, so Google's page opens on it), instead of an empty inbox that says nothing.
// Another address or provider stays in Settings > Accounts.
import { useEffect, useState } from 'react';
import Button from '../ds/primitives/Button';
import { call, errorText } from '../lib/api';
import { connectGoogle } from '../lib/google-sign-in';
import { tr } from '../lib/i18n';
import type { AccountInfo } from '../lib/server-connect';

export type ConnectMailboxProps = {
  /** The mailbox is in: the workspace is read again. */
  connected: () => void;
  /** Opens Settings > Accounts. */
  otherWays: () => void;
};

export default function ConnectMailbox({ connected, otherWays }: ConnectMailboxProps) {
  const [email, setEmail] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  useEffect(() => {
    let live = true;
    void call<AccountInfo>('server_account_info')
      .then((info) => live && setEmail(info.email?.trim() ?? ''))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, []);
  async function connect() {
    setBusy(true);
    setError('');
    try {
      await connectGoogle(email);
      connected();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <div className="connect-mailbox">
      <h2>{tr('Collega la tua posta', 'Connect your mail')}</h2>
      <p>
        {email
          ? tr(
              `Qamille legge la posta di ${email} dopo che lo consenti nella pagina di Google.`,
              `Qamille reads the mail of ${email} once you allow it on Google's page.`,
            )
          : tr(
              'Qamille legge la tua posta Gmail dopo che lo consenti nella pagina di Google.',
              "Qamille reads your Gmail once you allow it on Google's page.",
            )}
      </p>
      <Button variant="secondary" className="google-button" disabled={busy} onClick={() => void connect()}>
        <img src="/brand/google.svg" alt="" width={18} height={18} />
        {busy
          ? tr('Conferma nel browser…', 'Confirm in your browser…')
          : email
            ? tr(`Collega ${email}`, `Connect ${email}`)
            : tr('Collega un account Google', 'Connect a Google account')}
      </Button>
      <Button variant="text" onClick={otherWays}>
        {tr('Un altro indirizzo o provider', 'Another address or provider')}
      </Button>
      {error && (
        <p role="alert" className="connect-mailbox-error">
          {error}
        </p>
      )}
    </div>
  );
}
