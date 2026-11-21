// The key of the credentials (phase 3, Task 7; app/src/infrastructure/secret.rs, `VaultStatus`).
// On Windows the key is wrapped for the current Windows user; a data folder copied to another user
// or computer opens its credentials only with the transfer passphrase, or after a new sign-in.
// No imports: i18n.ts reads the texts below, and tests load this file in Node.

export type VaultStatus =
  | { state: 'ready'; protected: boolean; transfer: boolean }
  | { state: 'locked'; transfer: boolean }
  | { state: 'damaged'; code: VaultCode; message: string };

/** `vault_codes` in app/src/application/error.rs. */
export type VaultCode =
  | 'vault_foreign'
  | 'vault_locked'
  | 'vault_damaged'
  | 'vault_unavailable'
  | 'vault_storage'
  | 'vault_wrong_passphrase'
  | 'vault_no_transfer'
  | 'vault_needs_unlock'
  | 'vault_passphrase'
  | 'vault_nothing_to_start_over'
  | 'vault_needs_dpapi';

/** Windows cannot open the key right now: try again; starting over is never offered for it. */
export function retryable(status: VaultStatus): boolean {
  return status.state === 'damaged' && (status.code === 'vault_unavailable' || status.code === 'vault_storage');
}

/** `MIN_TRANSFER_PASSPHRASE_CHARS` in secret.rs: characters that are not spaces. */
export const MIN_TRANSFER_PASSPHRASE = 8;

export function longEnough(passphrase: string): boolean {
  return [...passphrase].filter((character) => !/\s/.test(character)).length >= MIN_TRANSFER_PASSPHRASE;
}

// The vault's messages, exactly as secret.rs and dpapi.rs write them, in Italian. They reach the
// interface as errors of commands (with a `vault_*` code) and inside the sync errors of an account
// (`address: message`), so they are matched by text; tests/vault-key.mjs checks that every
// message of secret.rs is here.
const UNCHANGED = 'I file originali non sono stati toccati.';
const ITALIAN: [RegExp, string | ((...parts: string[]) => string)][] = [
  [/^The credentials key belongs to another Windows user or another computer: sign in to the accounts again in Settings\.$/,
    'La chiave delle credenziali appartiene a un altro utente di Windows o a un altro computer: accedi di nuovo agli account in Impostazioni.'],
  [/^The credentials key belongs to another Windows user or another computer: enter the transfer passphrase in Settings, or sign in to the accounts again\.$/,
    'La chiave delle credenziali appartiene a un altro utente di Windows o a un altro computer: inserisci la frase di trasferimento in Impostazioni, oppure accedi di nuovo agli account.'],
  [/^The protected credentials key is damaged\. The original files are unchanged\.$/, `La chiave protetta delle credenziali è danneggiata. ${UNCHANGED}`],
  [/^The protected credentials key format is unsupported\. The original files are unchanged\.$/,
    `Il formato della chiave protetta delle credenziali non è supportato. ${UNCHANGED}`],
  [/^The credentials key is missing\. Restore credentials\.key\.dpapi together with credentials\.json\.$/,
    'Manca la chiave delle credenziali. Rimetti credentials.key.dpapi insieme a credentials.json.'],
  [/^The credentials key protected by Windows did not read back\. The plain key is kept\.$/,
    'La chiave protetta da Windows non si è riletta correttamente: resta la chiave in chiaro.'],
  [/^The transfer passphrase is not correct\.$/, 'La frase di trasferimento non è corretta.'],
  [/^The transfer copy of the credentials key is damaged\. The original files are unchanged\.$/,
    `La copia di trasferimento della chiave delle credenziali è danneggiata. ${UNCHANGED}`],
  [/^The transfer passphrase opens a key that does not match the saved credentials\. The original files are unchanged\.$/,
    `La frase di trasferimento apre una chiave che non corrisponde alle credenziali salvate. ${UNCHANGED}`],
  [/^No transfer passphrase was set for these credentials: sign in to the accounts again in Settings\.$/,
    'Per queste credenziali non c’è una frase di trasferimento: accedi di nuovo agli account in Impostazioni.'],
  [/^Open the credentials on this computer before changing the transfer passphrase\.$/,
    'Apri le credenziali su questo computer prima di cambiare la frase di trasferimento.'],
  [/^A transfer passphrase is used only when Windows protects the credentials key\.$/,
    'La frase di trasferimento serve solo quando Windows protegge la chiave delle credenziali.'],
  [/^The transfer passphrase needs at least 8 characters that are not spaces\.$/,
    'La frase di trasferimento deve avere almeno 8 caratteri che non siano spazi.'],
  [/^The transfer passphrase is too long\.$/, 'La frase di trasferimento è troppo lunga.'],
  [/^The credentials open on this computer: there is nothing to start over\.$/,
    'Le credenziali si aprono su questo computer: non c’è niente da mettere da parte.'],
  [/^The note of an interrupted start-over \(credentials\.set-aside\) is damaged\. The original files are unchanged\.$/,
    `La nota di una messa da parte interrotta (credentials.set-aside) è danneggiata. ${UNCHANGED}`],
  [/^There is not enough memory to open the transfer copy of the credentials key\. Close other programs and try again\.$/,
    'Non c’è abbastanza memoria per aprire la copia di trasferimento della chiave delle credenziali. Chiudi altri programmi e riprova.'],
  [/^The transfer passphrase could not be turned into a key\. The original files are unchanged\.$/,
    `Dalla frase di trasferimento non si è potuta ricavare una chiave. ${UNCHANGED}`],
  [/^The portable credentials key is missing\. Restore credentials\.key together with credentials\.json\.$/,
    'Manca la chiave delle credenziali. Rimetti credentials.key insieme a credentials.json.'],
  [/^Portable credentials cannot be read\. The original files are unchanged\.$/, `Le credenziali non si possono leggere. ${UNCHANGED}`],
  [/^The portable credentials format is unsupported\. The original files are unchanged\.$/,
    `Il formato delle credenziali non è supportato. ${UNCHANGED}`],
  [/^The portable credentials key is damaged\. The original files are unchanged\.$/, `La chiave delle credenziali è danneggiata. ${UNCHANGED}`],
  [/^The portable credentials key format is unsupported\. The original files are unchanged\.$/,
    `Il formato della chiave delle credenziali non è supportato. ${UNCHANGED}`],
  [/^Portable credentials failed authentication\. Restore the matching credentials\.key and credentials\.json files\.$/,
    'Le credenziali non superano il controllo di autenticità. Rimetti insieme i file credentials.key e credentials.json che vanno in coppia.'],
  [/^Decrypted portable credentials are invalid\. The original files are unchanged\.$/, `Le credenziali decifrate non sono valide. ${UNCHANGED}`],
  [/^Secure randomness for portable credentials is unavailable\.$/, 'Il sistema non dà numeri casuali sicuri per le credenziali.'],
  [/^The portable credentials encryption key is invalid\.$/, 'La chiave di cifratura delle credenziali non è valida.'],
  [/^Portable credentials encryption failed\.$/, 'La cifratura delle credenziali non è riuscita.'],
  [/^Portable credentials path is invalid\.$/, 'Il percorso delle credenziali non è valido.'],
  [/^Windows key protection is not available on this system\.$/, 'La protezione delle chiavi di Windows non c’è su questo sistema.'],
  [/^Windows could not open the credentials key right now \(error (0x[0-9A-F]{8})\)\. Nothing was changed: try again later\.$/,
    (code) => `Windows non riesce ad aprire ora la chiave delle credenziali (errore ${code}). Non è cambiato niente: riprova più tardi.`],
  [/^Windows could not protect the credentials key \(error (0x[0-9A-F]{8})\)\.$/,
    (code) => `Windows non è riuscito a proteggere la chiave delle credenziali (errore ${code}).`],
  [/^Portable credentials storage failed: (.+)$/, (cause) => `I file delle credenziali non si leggono o non si scrivono ora: ${cause}`],
];

function italian(line: string): string | null {
  for (const [pattern, replacement] of ITALIAN) {
    const found = pattern.exec(line);
    if (found) return typeof replacement === 'string' ? replacement : replacement(...found.slice(1));
  }
  return null;
}

/** A line written by the vault, in Italian when `inItalian`; any other line comes back as it is. A
 *  sync error carries it after the account's address. */
export function vaultText(line: string, inItalian: boolean): string {
  if (!inItalian) return line;
  const own = italian(line);
  if (own !== null) return own;
  const account = /^([^\s:]+@[^\s:]+): (.+)$/.exec(line);
  const inner = account && italian(account[2]);
  return inner ? `${account[1]}: ${inner}` : line;
}
