// The key of the credentials in the browser preview (phase 3, Task 7). Tests set
// `window.__previewVault` before the page loads to open it as a data folder from another Windows
// user or computer. The messages are the Rust ones (secret.rs).
import type { VaultStatus } from './vault';

declare global {
  interface Window {
    /**
     * Test hook: `locked` is a data folder whose key Windows wraps for another user or computer;
     * `passphrase` is the transfer passphrase that opens it; `startedOver` counts the start-overs.
     */
    __previewVault?: {
      state?: 'ready' | 'locked' | 'damaged' | 'unavailable' | 'storage';
      transfer?: boolean;
      passphrase?: string;
      startedOver?: number;
      /** The folder a sign-in set the old credentials aside in (`credentials_set_aside`). */
      notice?: string;
    };
  }
}

const FOLDER = 'credentials-set-aside-20261121-101500';

function status(): VaultStatus {
  const vault = window.__previewVault ?? {};
  switch (vault.state ?? 'ready') {
    case 'locked':
      return { state: 'locked', transfer: !!vault.transfer };
    case 'damaged':
      return { state: 'damaged', code: 'vault_damaged', message: 'The protected credentials key is damaged. The original files are unchanged.' };
    case 'storage':
      return { state: 'damaged', code: 'vault_storage', message: 'Portable credentials storage failed: Access is denied. (os error 5)' };
    case 'unavailable':
      return {
        state: 'damaged',
        code: 'vault_unavailable',
        message: 'Windows could not open the credentials key right now (error 0x8007000D). Nothing was changed: try again later.',
      };
    default:
      return { state: 'ready', protected: true, transfer: !!vault.transfer };
  }
}

export function previewVault(command: string, args: Record<string, unknown>): VaultStatus | string | void {
  const vault = (window.__previewVault ??= {});
  const ready = status().state === 'ready';
  switch (command) {
    case 'vault_set_transfer_passphrase': {
      if (!ready) throw new Error('Open the credentials on this computer before changing the transfer passphrase.');
      const passphrase = String(args.passphrase ?? '');
      if ([...passphrase].filter((c) => !/\s/.test(c)).length < 8)
        throw new Error('The transfer passphrase needs at least 8 characters that are not spaces.');
      vault.transfer = true;
      vault.passphrase = passphrase;
      return status();
    }
    case 'vault_remove_transfer_passphrase':
      if (!ready) throw new Error('Open the credentials on this computer before changing the transfer passphrase.');
      vault.transfer = false;
      vault.passphrase = undefined;
      return status();
    case 'vault_unlock':
      if (ready) return status();
      if (vault.state !== 'locked' || !vault.transfer)
        throw new Error('No transfer passphrase was set for these credentials: sign in to the accounts again in Settings.');
      if (args.passphrase !== vault.passphrase) throw new Error('The transfer passphrase is not correct.');
      vault.state = 'ready';
      return status();
    case 'vault_dismiss_set_aside':
      vault.notice = undefined;
      return;
    case 'vault_start_over':
      if (ready) throw new Error('The credentials open on this computer: there is nothing to start over.');
      if (vault.state === 'unavailable' && !args.unopenable)
        throw new Error('Windows could not open the credentials key right now (error 0x8007000D). Nothing was changed: try again later.');
      vault.state = 'ready';
      vault.transfer = false;
      vault.passphrase = undefined;
      vault.startedOver = (vault.startedOver ?? 0) + 1;
      return FOLDER;
    default:
      return status();
  }
}
