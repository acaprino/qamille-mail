// «Forgot password?» (access wave 3, spec A10), in both editions when the server offers it:
// email → a code by mail (the same answer whether the email has an account or not) → code and new
// password → back to «Sign in» with the email filled in. Every device of the account is signed out.
import { useState } from 'react';
import Button from '../../ds/primitives/Button';
import Field from '../../ds/primitives/Field';
import { call, errorText } from '../../lib/api';
import { tr } from '../../lib/i18n';

export type ResetPasswordStepProps = {
  url: string;
  onDone: (email: string) => void;
  onBack: () => void;
};

export default function ResetPasswordStep({ url, onDone, onBack }: ResetPasswordStepProps) {
  const [email, setEmail] = useState('');
  const [sent, setSent] = useState(false);
  const [code, setCode] = useState('');
  const [password, setPassword] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
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
  return (
    <section className="server-reset" aria-label={tr('Password dimenticata', 'Forgot password')}>
      <h3>{tr('Password dimenticata', 'Forgot password')}</h3>
      <Field>
        {tr('Email', 'Email')}
        <input type="email" value={email} autoComplete="email" disabled={sent} onChange={(e) => setEmail(e.currentTarget.value)} />
      </Field>
      {!sent ? (
        <div className="button-row">
          <Button
            variant="primary"
            disabled={busy || !email.includes('@')}
            onClick={() =>
              void run(async () => {
                await call('server_reset_start', { url, email });
                setSent(true);
              })
            }
          >
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
            void run(async () => {
              await call('server_reset_confirm', { url, email, code, newPassword: password });
              setPassword('');
              onDone(email);
            });
          }}
        >
          <p role="status" className="field-help">
            {tr("Se l'email è di un account, ti ho scritto un codice.", 'If the email belongs to an account, I sent you a code.')}
          </p>
          <Field>
            {tr('Codice', 'Code')}
            <input value={code} autoComplete="one-time-code" onChange={(e) => setCode(e.currentTarget.value)} />
          </Field>
          <Field>
            {tr('Nuova password', 'New password')}
            <input type="password" value={password} autoComplete="new-password" onChange={(e) => setPassword(e.currentTarget.value)} />
          </Field>
          <p className="field-help">
            {tr('Tutti i dispositivi dell’account verranno scollegati.', 'Every device of the account will be signed out.')}
          </p>
          <div className="button-row">
            <Button variant="primary" type="submit" disabled={busy || !code.trim() || password.length < 10}>
              {tr('Imposta la nuova password', 'Set the new password')}
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
