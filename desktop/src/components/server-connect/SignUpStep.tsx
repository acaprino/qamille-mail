// «Create account» in the Professional build (access wave 3, spec A9): email → a code by mail →
// code, password twice, optional username, device name. The server answers the same way whether
// the email is free or already has an account; «Send the code again» after 60 s.
import { useEffect, useState, type ReactNode } from 'react';
import Button from '../../ui/Button';
import Field from '../../ui/Field';
import DeviceName from './DeviceName';
import { call, errorText } from '../../lib/api';
import { tr } from '../../lib/i18n';
import type { LegalInfo } from '../../lib/server-connect';
import { acceptedTerms, needsTermsBox, termsBlock } from '../../lib/subscription-usage';
import TermsBox from './TermsBox';
import { Input } from '../../ui/Input';

export type SignUpStepProps = {
  url: string;
  defaultDeviceName: string;
  /** The server's documents (`GET /v1/info`), when it asks for them. */
  legal?: LegalInfo | null;
  /** The host titles the step already (the sign-in card). */
  titled?: boolean;
  /** «Continue with Google», offered until the code is sent. */
  google?: ReactNode;
  /** The Terms box shared with «Continue with Google»: one box for both ways in, above them. */
  terms?: boolean;
  onTermsChange?: (checked: boolean) => void;
  onDone: () => void;
  onBack: () => void;
};

export default function SignUpStep({ url, defaultDeviceName, legal, titled = false, google, terms: sharedTerms, onTermsChange, onDone, onBack }: SignUpStepProps) {
  const [email, setEmail] = useState('');
  const [sent, setSent] = useState(false);
  const [wait, setWait] = useState(0);
  const [code, setCode] = useState('');
  const [password, setPassword] = useState('');
  const [repeat, setRepeat] = useState('');
  const [username, setUsername] = useState('');
  const [deviceName, setDeviceName] = useState(defaultDeviceName);
  const [ownTerms, setOwnTerms] = useState(false);
  const terms = sharedTerms ?? ownTerms;
  const setTerms = onTermsChange ?? setOwnTerms;
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  useEffect(() => {
    if (wait <= 0) return;
    const timer = setTimeout(() => setWait(wait - 1), 1000);
    return () => clearTimeout(timer);
  }, [wait]);
  async function run(work: () => Promise<void>) {
    setBusy(true);
    setError('');
    try {
      await work();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }
  const send = () =>
    run(async () => {
      await call('server_signup_start', { url, email });
      setSent(true);
      setWait(60);
    });

  return (
    <section className="server-signup" aria-label={tr('Crea account', 'Create account')}>
      {!titled && <h3>{tr('Crea account', 'Create account')}</h3>}
      {google && !sent && needsTermsBox(legal) && <TermsBox legal={legal!} checked={terms} onChange={setTerms} onError={setError} />}
      {!sent && google}
      <Field>
        {tr('Email', 'Email')}
        <Input type="email" value={email} autoComplete="email" disabled={sent} onChange={(e) => setEmail(e.currentTarget.value)} />
      </Field>
      {!google && needsTermsBox(legal) && <TermsBox legal={legal!} checked={terms} onChange={setTerms} onError={setError} />}
      {!sent ? (
        <div className="button-row step-actions">
          <Button variant="primary" disabled={busy || !email.includes('@') || termsBlock(legal, terms)} onClick={() => void send()}>
            {tr('Mandami il codice', 'Send me the code')}
          </Button>
          <Button variant="text" onClick={onBack}>
            {tr('Indietro', 'Back')}
          </Button>
        </div>
      ) : (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            if (password !== repeat) return setError(tr('Le due password non coincidono.', 'The two passwords differ.'));
            void run(async () => {
              await call('server_signup_confirm', { url, email, code, password, username: username.trim() || null, deviceName, acceptedTerms: acceptedTerms(legal, terms) });
              setPassword('');
              setRepeat('');
              onDone();
            });
          }}
        >
          <p role="status" className="field-help">
            {tr('Ti ho scritto un codice: controlla la posta.', 'I sent you a code: check your mail.')}
          </p>
          <Field>
            {tr('Codice', 'Code')}
            <Input value={code} autoComplete="one-time-code" onChange={(e) => setCode(e.currentTarget.value)} />
          </Field>
          <Field>
            {tr('Password', 'Password')}
            <Input type="password" value={password} autoComplete="new-password" onChange={(e) => setPassword(e.currentTarget.value)} />
          </Field>
          <Field>
            {tr('Ripeti la password', 'Repeat the password')}
            <Input type="password" value={repeat} autoComplete="new-password" onChange={(e) => setRepeat(e.currentTarget.value)} />
          </Field>
          <Field>
            {tr('Username (facoltativo)', 'Username (optional)')}
            <Input value={username} autoComplete="username" onChange={(e) => setUsername(e.currentTarget.value)} />
          </Field>
          <DeviceName value={deviceName} onChange={setDeviceName} />
          <div className="button-row step-actions">
            <Button variant="primary" type="submit" disabled={busy || !code.trim() || password.length < 10 || termsBlock(legal, terms)}>
              {tr('Crea account ed entra', 'Create account and sign in')}
            </Button>
            <Button variant="secondary" type="button" disabled={busy || wait > 0} onClick={() => void send()}>
              {wait > 0 ? tr(`Invia di nuovo il codice (${wait} s)`, `Send the code again (${wait} s)`) : tr('Invia di nuovo il codice', 'Send the code again')}
            </Button>
            <Button variant="text" type="button" onClick={onBack}>
              {tr('Indietro', 'Back')}
            </Button>
          </div>
        </form>
      )}
      {error && (
        <p role="alert" className="index-notice">
          {error}
        </p>
      )}
    </section>
  );
}
