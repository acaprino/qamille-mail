// The key of the credentials (phase 3, Task 7; app/src/infrastructure/secret.rs, `VaultStatus`).
// On Windows the key is wrapped for the current Windows user; a data folder copied to another user
// or computer opens its credentials only with the transfer passphrase, or after a new sign-in.
export type VaultStatus =
  | { state: 'ready'; protected: boolean; transfer: boolean }
  | { state: 'locked'; transfer: boolean }
  | { state: 'damaged'; message: string };

/** `MIN_TRANSFER_PASSPHRASE_CHARS` in secret.rs, spaces at the ends not counted. */
export const MIN_TRANSFER_PASSPHRASE = 8;

export function longEnough(passphrase: string): boolean {
  return [...passphrase.trim()].length >= MIN_TRANSFER_PASSPHRASE;
}
