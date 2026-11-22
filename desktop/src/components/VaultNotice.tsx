// At start-up, when the credentials of this data folder do not open (phase 3, Task 7): a folder
// from another Windows user or computer (the dialog asks for the transfer passphrase, or says the
// accounts need a new sign-in), damaged files, or Windows unable to open the key right now. It draws
// over everything, the Professional's server sign-in included. «Later» closes it until the next
// start, or until a sign-in is refused for the passphrase; Settings → Accounts has the same
// controls. When a sign-in has set the old credentials of
// another computer aside (`setAside`, from the workspace snapshot), it says where, once.
import { useEffect, useRef, useState } from 'react';
import Button from '../ds/primitives/Button';
import Modal from '../ds/primitives/Modal';
import { call } from '../lib/api';
import { tr } from '../lib/i18n';
import { VAULT_LOCKED_EVENT, type VaultStatus } from '../lib/vault';
import VaultUnlock from './VaultUnlock';

type Props = {
  notify: (message: string, error?: boolean) => void;
  /** The credentials changed: the workspace reads them again. */
  changed: () => void;
  setAside?: string | null;
};

export default function VaultNotice({ notify, changed, setAside }: Props) {
  const [status, setStatus] = useState<VaultStatus | null>(null);
  const told = useRef('');

  useEffect(() => {
    let live = true;
    const read = () =>
      void call<VaultStatus>('vault_status')
        .then((value) => live && setStatus(value))
        .catch(() => {});
    read();
    // A sign-in refused for the transfer passphrase brings the dialog back after «Later».
    window.addEventListener(VAULT_LOCKED_EVENT, read);
    return () => {
      live = false;
      window.removeEventListener(VAULT_LOCKED_EVENT, read);
    };
  }, []);

  useEffect(() => {
    if (!setAside || told.current === setAside) return;
    told.current = setAside;
    notify(
      tr(
        `Le credenziali dell’altro computer sono in data\\${setAside}: non sono cancellate, ma Qamille non le usa più.`,
        `The credentials of the other computer are in data\\${setAside}: they are not deleted, but Qamille no longer uses them.`,
      ),
    );
    void call('vault_dismiss_set_aside').catch(() => {});
  }, [setAside]);

  if (!status || status.state === 'ready') return null;
  return (
    <Modal size="sm" aria-labelledby="vault-notice-title" onClose={() => setStatus(null)}>
      <h2 id="vault-notice-title">
        {status.state === 'locked'
          ? tr('Credenziali di un altro computer', 'Credentials from another computer')
          : tr('Le credenziali non si aprono', 'The credentials do not open')}
      </h2>
      <VaultUnlock
        status={status}
        error={(message) => notify(message, true)}
        done={(next, message) => {
          setStatus(next);
          if (message) notify(message);
          changed();
        }}
      >
        <Button variant="text" type="button" onClick={() => setStatus(null)}>
          {tr('Più tardi', 'Later')}
        </Button>
      </VaultUnlock>
    </Modal>
  );
}
