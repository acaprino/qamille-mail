// At start-up, when Windows does not open the credentials of this data folder (phase 3, Task 7: a
// folder copied from another Windows user or computer), a dialog asks for the transfer passphrase,
// or says that the accounts need a new sign-in. It draws over everything, the Professional's
// server sign-in included. «Later» closes it until the next start; Settings → Accounts has the same
// controls.
import { useEffect, useState } from 'react';
import Button from '../ds/primitives/Button';
import Modal from '../ds/primitives/Modal';
import { call } from '../lib/api';
import { tr } from '../lib/i18n';
import type { VaultStatus } from '../lib/vault';
import VaultUnlock from './VaultUnlock';

type Props = {
  notify: (message: string, error?: boolean) => void;
  /** The credentials changed: the workspace reads them again. */
  changed: () => void;
};

export default function VaultNotice({ notify, changed }: Props) {
  const [status, setStatus] = useState<VaultStatus | null>(null);

  useEffect(() => {
    let live = true;
    void call<VaultStatus>('vault_status')
      .then((value) => live && setStatus(value))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, []);

  if (status?.state !== 'locked') return null;
  return (
    <Modal size="sm" aria-labelledby="vault-notice-title" onClose={() => setStatus(null)}>
      <h2 id="vault-notice-title">{tr('Credenziali di un altro computer', 'Credentials from another computer')}</h2>
      <VaultUnlock
        status={status}
        error={(message) => notify(message, true)}
        done={(next, message) => {
          setStatus(next);
          notify(message);
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
