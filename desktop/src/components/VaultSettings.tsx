// Settings → Accounts → Credentials (phase 3, Task 7): how the key of the credentials is kept, and
// the optional transfer passphrase that opens it on another Windows user or computer. Changing or
// removing the passphrase needs the key open here; Windows vouches for this user, so the old
// passphrase is not asked. The passphrase is never shown again and never saved.
import { useEffect, useState } from 'react';
import Button from '../ds/primitives/Button';
import Field from '../ds/primitives/Field';
import { call, errorText } from '../lib/api';
import { tr } from '../lib/i18n';
import { longEnough, MIN_TRANSFER_PASSPHRASE, type VaultStatus } from '../lib/vault';
import VaultUnlock from './VaultUnlock';

type Props = {
  notify: (message: string, error?: boolean) => void;
  changed: () => void;
};

export default function VaultSettings({ notify, changed }: Props) {
  const [status, setStatus] = useState<VaultStatus | null>(null);
  const [editing, setEditing] = useState(false);
  const [passphrase, setPassphrase] = useState('');
  const [repeat, setRepeat] = useState('');
  const [problem, setProblem] = useState('');
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    let live = true;
    void call<VaultStatus>('vault_status')
      .then((value) => live && setStatus(value))
      .catch((e) => live && notify(errorText(e), true));
    return () => {
      live = false;
    };
  }, []);

  function close() {
    setEditing(false);
    setPassphrase('');
    setRepeat('');
    setProblem('');
  }
  async function run(work: () => Promise<VaultStatus>, message: string) {
    setBusy(true);
    try {
      setStatus(await work());
      close();
      notify(message);
    } catch (e) {
      setProblem(errorText(e));
    } finally {
      setBusy(false);
    }
  }

  if (!status) return null;
  return (
    <>
      <h3>{tr('Credenziali', 'Credentials')}</h3>
      {status.state !== 'ready' ? (
        <VaultUnlock
          status={status}
          error={(message) => notify(message, true)}
          done={(next, message) => {
            setStatus(next);
            if (message) notify(message);
            changed();
          }}
        />
      ) : (
        <>
          <p className="muted">
            {status.protected
              ? tr(
                  'Password e accessi degli account sono cifrati con una chiave che Windows protegge per questo utente: copiata su un altro utente o computer, la cartella dei dati non li apre.',
                  'Account passwords and sign-ins are encrypted with a key that Windows protects for this user: copied to another user or computer, the data folder does not open them.',
                )
              : tr(
                  'La chiave delle credenziali è ancora in chiaro nella cartella dei dati: Qamille prova a farla proteggere da Windows a ogni avvio.',
                  'The credentials key is still in clear in the data folder: Qamille tries to have Windows protect it at every start.',
                )}
          </p>
          <div className="setting-row">
            <strong>
              {tr('Frase di trasferimento', 'Transfer passphrase')}
              {status.transfer && <span className="tag">{tr('Impostata', 'Set')}</span>}
            </strong>
            <p>
              {tr(
                'Per portare la cartella dei dati su un altro computer o utente senza accedere di nuovo agli account, scegli una frase di trasferimento: Qamille la chiederà là. Non viene salvata: se la dimentichi, sull’altro computer accedi di nuovo agli account.',
                'To take the data folder to another computer or user without signing in to the accounts again, choose a transfer passphrase: Qamille asks for it there. It is not saved: if you forget it, sign in to the accounts again on the other computer.',
              )}
            </p>
            {problem && (
              <p className="field-help" role="alert">
                {problem}
              </p>
            )}
            {editing ? (
              <form
                onSubmit={(e) => {
                  e.preventDefault();
                  if (passphrase !== repeat) return setProblem(tr('Le due frasi non coincidono.', 'The two passphrases differ.'));
                  void run(
                    () => call<VaultStatus>('vault_set_transfer_passphrase', { passphrase }),
                    tr('Frase di trasferimento salvata.', 'Transfer passphrase saved.'),
                  );
                }}
              >
                <Field>
                  {tr('Nuova frase di trasferimento', 'New transfer passphrase')}
                  <input
                    type="password"
                    value={passphrase}
                    autoComplete="new-password"
                    spellCheck={false}
                    onChange={(e) => setPassphrase(e.currentTarget.value)}
                  />
                </Field>
                <Field>
                  {tr('Ripeti la frase', 'Repeat the passphrase')}
                  <input
                    type="password"
                    value={repeat}
                    autoComplete="new-password"
                    spellCheck={false}
                    onChange={(e) => setRepeat(e.currentTarget.value)}
                  />
                </Field>
                <p className="field-help">
                  {tr(
                    `Almeno ${MIN_TRANSFER_PASSPHRASE} caratteri. Annotala in un posto sicuro, fuori da questo computer.`,
                    `At least ${MIN_TRANSFER_PASSPHRASE} characters. Write it down somewhere safe, away from this computer.`,
                  )}
                </p>
                <div className="button-row">
                  <Button variant="primary" type="submit" disabled={busy || !longEnough(passphrase) || !repeat}>
                    {busy ? tr('Attendi…', 'Working…') : tr('Salva la frase', 'Save the passphrase')}
                  </Button>
                  <Button variant="text" type="button" disabled={busy} onClick={close}>
                    {tr('Annulla', 'Cancel')}
                  </Button>
                </div>
              </form>
            ) : (
              <div className="button-row">
                <Button variant="secondary" disabled={busy || !status.protected} onClick={() => setEditing(true)}>
                  {status.transfer
                    ? tr('Cambia la frase', 'Change the passphrase')
                    : tr('Imposta una frase di trasferimento', 'Set a transfer passphrase')}
                </Button>
                {status.transfer && (
                  <Button
                    variant="text"
                    disabled={busy}
                    onClick={() =>
                      void run(
                        () => call<VaultStatus>('vault_remove_transfer_passphrase'),
                        tr('Frase di trasferimento rimossa.', 'Transfer passphrase removed.'),
                      )
                    }
                  >
                    {tr('Rimuovi la frase', 'Remove the passphrase')}
                  </Button>
                )}
              </div>
            )}
          </div>
        </>
      )}
    </>
  );
}
