// Phase 3, Task 7: the key of the credentials, in English and in Italian.
//  - Every message of the vault (app/src/infrastructure/secret.rs) has an Italian text (lib/vault.ts),
//    also inside the sync error of an account.
//  - A data folder from another Windows user or computer opens with a dialog that asks for the
//    transfer passphrase or offers a new sign-in, which always asks first and sets the old
//    credentials aside; damaged credentials get the dialog too; when Windows cannot open the key
//    right now only «Try again» is offered.
//  - After a sign-in set the old credentials aside, the folder is named once.
//  - Settings → Accounts → Credentials sets, changes and removes the transfer passphrase.
// Synthetic data only; the preview plays the vault. Run against the browser preview:
// QAMILLE_PREVIEW_URL (default http://127.0.0.1:1420).
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import { chromium, expect } from '@playwright/test';

const desktop = path.resolve(import.meta.dirname, '..');
const { vaultText } = await import(pathToFileURL(path.join(desktop, 'src/lib/vault.ts')).href);
{
  const source = fs.readFileSync(path.join(desktop, '../app/src/infrastructure/secret.rs'), 'utf8');
  // Every message constant: a sentence (a space, a final full stop).
  const messages = [...source.matchAll(/^const [A-Z_]+: &str = "([^"]+ [^"]+\.)";$/gm)].map((match) => match[1]);
  assert.ok(messages.length >= 30, `found ${messages.length} messages`);
  const dynamic = [
    'Windows could not open the credentials key right now (error 0x8007000D). Nothing was changed: try again later.',
    'Windows could not protect the credentials key (error 0x80090016).',
    'Portable credentials storage failed: Access is denied. (os error 5)',
  ];
  for (const message of [...messages, ...dynamic]) {
    const italian = vaultText(message, true);
    assert.notEqual(italian, message, `no Italian text for: ${message}`);
    assert.equal(vaultText(message, false), message);
  }
  const foreign = messages.find((m) => m.includes('sign in to the accounts again in Settings'));
  assert.ok(vaultText(`mario@example.test: ${foreign}`, true).startsWith('mario@example.test: La chiave delle credenziali'));
  assert.equal(vaultText('Another message', true), 'Another message');
  console.log(`PASS every vault message (${messages.length} + ${dynamic.length}) has its Italian text`);
}

const TEXT = {
  'en-US': {
    sidebar: 'Sidebar', workspace: /^Settings/, accounts: 'Accounts', credentials: 'Credentials',
    fromAnother: 'Credentials from another computer', doNotOpen: 'The credentials do not open',
    chosenThere: 'the account credentials open with the transfer passphrase chosen there',
    passphrase: 'Transfer passphrase', open: 'Open the credentials', wrong: 'The transfer passphrase is not correct.',
    opened: 'Credentials opened: Windows now protects them for this user.',
    foreign: 'The credentials key belongs to another Windows user or another computer: sign in to the accounts again in Settings. Mail and settings stay.',
    signInAgain: 'Sign in to the accounts again', notDeleted: 'without being deleted', cancel: 'Cancel', later: 'Later',
    previous: 'The previous credentials are in data\\credentials-set-aside-20261121-101500', forgotten: 'Forgotten: sign in again',
    change: 'Change the passphrase', protects: 'encrypted with a key that Windows protects for this user',
    set: 'Set a transfer passphrase', newPassphrase: 'New transfer passphrase', repeat: 'Repeat the passphrase',
    save: 'Save the passphrase', differ: 'The two passphrases differ.', saved: 'Transfer passphrase saved.', tag: 'Set',
    remove: 'Remove the passphrase', removed: 'Transfer passphrase removed.',
    short: 'The transfer passphrase needs at least 8 characters that are not spaces.',
    damaged: 'The protected credentials key is damaged. The original files are unchanged.',
    retry: 'Try again', rightNow: 'Windows cannot open the credentials key right now',
    unavailable: 'Windows could not open the credentials key right now (error 0x8007000D). Nothing was changed: try again later.',
    notice: 'The credentials of the other computer are in data\\credentials-set-aside-20261121-101500',
    putBack: 'If Windows opens them again later, they can be put back',
    filesNow: 'The credential files cannot be read right now',
    storage: 'Portable credentials storage failed: Access is denied. (os error 5)',
  },
  'it-IT': {
    sidebar: 'Barra laterale', workspace: /^Impostazioni/, accounts: 'Account', credentials: 'Credenziali',
    fromAnother: 'Credenziali di un altro computer', doNotOpen: 'Le credenziali non si aprono',
    chosenThere: 'le credenziali degli account si aprono con la frase di trasferimento scelta là',
    passphrase: 'Frase di trasferimento', open: 'Apri le credenziali', wrong: 'La frase di trasferimento non è corretta.',
    opened: 'Credenziali aperte: ora le protegge Windows per questo utente.',
    foreign: 'La chiave delle credenziali appartiene a un altro utente di Windows o a un altro computer: accedi di nuovo agli account in Impostazioni. La posta e le impostazioni restano.',
    signInAgain: 'Accedi di nuovo agli account', notDeleted: 'senza essere cancellate', cancel: 'Annulla', later: 'Più tardi',
    previous: 'Le credenziali di prima sono in data\\credentials-set-aside-20261121-101500', forgotten: 'Non la ricordo: accedi di nuovo',
    change: 'Cambia la frase', protects: 'cifrati con una chiave che Windows protegge per questo utente',
    set: 'Imposta una frase di trasferimento', newPassphrase: 'Nuova frase di trasferimento', repeat: 'Ripeti la frase',
    save: 'Salva la frase', differ: 'Le due frasi non coincidono.', saved: 'Frase di trasferimento salvata.', tag: 'Impostata',
    remove: 'Rimuovi la frase', removed: 'Frase di trasferimento rimossa.',
    short: 'La frase di trasferimento deve avere almeno 8 caratteri che non siano spazi.',
    damaged: 'La chiave protetta delle credenziali è danneggiata. I file originali non sono stati toccati.',
    retry: 'Riprova', rightNow: 'Windows non riesce ad aprire ora la chiave delle credenziali',
    unavailable: 'Windows non riesce ad aprire ora la chiave delle credenziali (errore 0x8007000D). Non è cambiato niente: riprova più tardi.',
    notice: 'Le credenziali dell’altro computer sono in data\\credentials-set-aside-20261121-101500',
    putBack: 'Se poi Windows torna ad aprirle, si possono rimettere a posto',
    filesNow: 'I file delle credenziali non si leggono ora',
    storage: 'I file delle credenziali non si leggono o non si scrivono ora: Access is denied. (os error 5)',
  },
};

const browser = await chromium.launch({ channel: 'msedge', headless: true, args: ['--force-prefers-reduced-motion'] });
const errors = [];
async function open(vault, locale, server) {
  const context = await browser.newContext({ viewport: { width: 1440, height: 1000 }, locale });
  const page = await context.newPage();
  page.on('pageerror', (error) => errors.push(error.message));
  page.setDefaultTimeout(15000);
  await page.addInitScript(({ vault, server }) => {
    window.__previewVault = vault;
    if (server) window.__previewServer = server;
  }, { vault, server });
  await page.goto(process.env.QAMILLE_PREVIEW_URL || 'http://127.0.0.1:1420');
  // The Professional opens on its server sign-in, not on the mail.
  if (!server) await page.getByRole('button', { name: /^(New message|Nuovo messaggio)/ }).waitFor();
  return { page, context };
}
const vault = (page) => page.evaluate(() => window.__previewVault);
const toast = (page, text) => page.locator('[data-scope="toast"][data-part="root"]').filter({ hasText: text });
async function credentials(page, t) {
  await page.getByRole('navigation', { name: t.sidebar }).getByRole('button', { name: t.workspace }).click();
  await page.locator('.settings-nav').getByRole('button', { name: t.accounts, exact: true }).click();
  await expect(page.getByRole('heading', { name: t.credentials, exact: true })).toBeVisible();
}

try {
  for (const [locale, t] of Object.entries(TEXT)) {
    {
      const { page, context } = await open({ state: 'locked', transfer: true, passphrase: 'correct horse battery' }, locale);
      const dialog = page.getByRole('dialog', { name: t.fromAnother });
      await expect(dialog).toBeVisible();
      await expect(dialog).toContainText(t.chosenThere);
      const field = dialog.getByLabel(t.passphrase);
      assert.equal(await field.getAttribute('autocomplete'), 'new-password', 'nothing saved is offered');
      await expect(dialog.getByRole('button', { name: t.open })).toBeDisabled();
      await field.fill('wrong horse battery');
      await field.press('Enter');
      await expect(toast(page, t.wrong)).toBeVisible();
      await expect(dialog).toBeVisible();
      assert.equal((await vault(page)).state, 'locked', 'a wrong passphrase changes nothing');
      await field.fill('correct horse battery');
      await dialog.getByRole('button', { name: t.open }).click();
      await expect(dialog).toHaveCount(0);
      await expect(toast(page, t.opened)).toBeVisible();
      assert.equal((await vault(page)).state, 'ready');
      assert.equal((await vault(page)).startedOver, undefined, 'opening with the passphrase sets nothing aside');
      console.log(`PASS [${locale}] a folder from another computer asks for the transfer passphrase; a wrong one changes nothing`);
      await context.close();
    }
    {
      const { page, context } = await open({ state: 'locked', transfer: false }, locale);
      const dialog = page.getByRole('dialog', { name: t.fromAnother });
      await expect(dialog).toContainText(t.foreign);
      await expect(dialog.getByLabel(t.passphrase)).toHaveCount(0);
      await dialog.getByRole('button', { name: t.signInAgain }).click();
      await expect(dialog).toContainText(t.notDeleted);
      await dialog.getByRole('button', { name: t.cancel, exact: true }).click();
      assert.equal((await vault(page)).startedOver, undefined, 'cancel sets nothing aside');
      await dialog.getByRole('button', { name: t.signInAgain }).click();
      await dialog.getByRole('button', { name: t.signInAgain }).click();
      await expect(dialog).toHaveCount(0);
      await expect(toast(page, t.previous)).toBeVisible();
      assert.equal((await vault(page)).startedOver, 1);
      console.log(`PASS [${locale}] without a passphrase the dialog says to sign in again, and starting over asks first`);
      await context.close();
    }
    {
      const { page, context } = await open({ state: 'damaged' }, locale);
      const dialog = page.getByRole('dialog', { name: t.doNotOpen });
      await expect(dialog).toContainText(t.damaged);
      await expect(dialog.getByRole('button', { name: t.signInAgain })).toBeVisible();
      await dialog.getByRole('button', { name: t.later, exact: true }).click();
      await expect(dialog).toHaveCount(0);
      console.log(`PASS [${locale}] damaged credentials get the dialog too`);
      await context.close();
    }
    {
      const { page, context } = await open({ state: 'unavailable' }, locale);
      const dialog = page.getByRole('dialog', { name: t.doNotOpen });
      await expect(dialog).toContainText(t.rightNow);
      await expect(dialog).toContainText(t.unavailable);
      await expect(dialog.getByRole('button', { name: t.signInAgain })).toHaveCount(0);
      await dialog.getByRole('button', { name: t.retry, exact: true }).click();
      await expect(toast(page, t.unavailable)).toBeVisible();
      await expect(dialog).toBeVisible();
      assert.equal((await vault(page)).startedOver, undefined);
      await page.evaluate(() => (window.__previewVault.state = 'ready'));
      await dialog.getByRole('button', { name: t.retry, exact: true }).click();
      await expect(dialog).toHaveCount(0);
      console.log(`PASS [${locale}] when Windows cannot open the key right now, «Try again» comes first`);
      await context.close();
    }
    {
      // «Try again» fails: the way out is the confirmed new sign-in, which only moves files.
      const { page, context } = await open({ state: 'unavailable' }, locale);
      const dialog = page.getByRole('dialog', { name: t.doNotOpen });
      await dialog.getByRole('button', { name: t.retry, exact: true }).click();
      await expect(toast(page, t.unavailable)).toBeVisible();
      await dialog.getByRole('button', { name: t.signInAgain }).click();
      await expect(dialog).toContainText(t.putBack);
      await dialog.getByRole('button', { name: t.cancel, exact: true }).click();
      assert.equal((await vault(page)).startedOver, undefined, 'cancel sets nothing aside');
      await dialog.getByRole('button', { name: t.signInAgain }).click();
      await dialog.getByRole('button', { name: t.signInAgain }).click();
      await expect(dialog).toHaveCount(0);
      await expect(toast(page, t.previous)).toBeVisible();
      assert.equal((await vault(page)).startedOver, 1);
      console.log(`PASS [${locale}] when «Try again» fails, a confirmed new sign-in sets the credentials aside`);
      await context.close();
    }
    {
      // Files that cannot be read right now: only «Try again», with its own words, never a start-over.
      const { page, context } = await open({ state: 'storage' }, locale);
      const dialog = page.getByRole('dialog', { name: t.doNotOpen });
      await expect(dialog).toContainText(t.filesNow);
      await expect(dialog).not.toContainText(t.rightNow);
      await dialog.getByRole('button', { name: t.retry, exact: true }).click();
      await expect(toast(page, t.storage)).toBeVisible();
      await dialog.getByRole('button', { name: t.retry, exact: true }).click();
      await expect(dialog.getByRole('button', { name: t.retry, exact: true })).toBeEnabled();
      await expect(dialog.getByRole('button', { name: t.signInAgain })).toHaveCount(0);
      assert.equal((await vault(page)).startedOver, undefined);
      console.log(`PASS [${locale}] files that cannot be read now get only «Try again», never a start-over`);
      await context.close();
    }
    {
      const { page, context } = await open({ notice: 'credentials-set-aside-20261121-101500' }, locale);
      await expect(toast(page, t.notice)).toBeVisible();
      await expect.poll(async () => (await vault(page)).notice ?? null).toBe(null);
      await expect(page.getByRole('dialog')).toHaveCount(0);
      console.log(`PASS [${locale}] after a sign-in set the old credentials aside, the folder is named once`);
      await context.close();
    }
    {
      const { page, context } = await open({ state: 'locked', transfer: true, passphrase: 'correct horse battery' }, locale);
      const dialog = page.getByRole('dialog', { name: t.fromAnother });
      await dialog.getByRole('button', { name: t.later, exact: true }).click();
      await expect(dialog).toHaveCount(0);
      // Settings shows the same controls; the mail is there meanwhile.
      await credentials(page, t);
      await expect(page.getByLabel(t.passphrase)).toBeVisible();
      await page.getByRole('button', { name: t.forgotten }).click();
      await expect(page.getByText(t.notDeleted)).toBeVisible();
      await page.getByRole('button', { name: t.cancel, exact: true }).click();
      await page.getByLabel(t.passphrase).fill('correct horse battery');
      await page.getByRole('button', { name: t.open }).click();
      await expect(page.getByRole('button', { name: t.change })).toBeVisible();
      assert.equal((await vault(page)).state, 'ready');
      console.log(`PASS [${locale}] «Later» closes the dialog; Settings → Accounts opens the credentials too`);
      await context.close();
    }
    {
      const { page, context } = await open({}, locale);
      await expect(page.getByRole('dialog')).toHaveCount(0);
      await credentials(page, t);
      await expect(page.getByText(t.protects)).toBeVisible();
      await page.getByRole('button', { name: t.set }).click();
      const save = page.getByRole('button', { name: t.save });
      await page.getByLabel(t.newPassphrase).fill('a b c d e f g');
      await page.getByLabel(t.repeat).fill('a b c d e f g');
      await expect(save).toBeDisabled();
      await page.getByLabel(t.newPassphrase).fill('a passphrase for the move');
      await page.getByLabel(t.repeat).fill('a passphrase for the mvoe');
      await save.click();
      await expect(page.locator('[role="alert"], [data-scope="toast"][data-part="root"]').filter({ hasText: t.differ })).toBeVisible();
      assert.equal((await vault(page)).transfer, undefined, 'two different passphrases save nothing');
      await page.getByLabel(t.repeat).fill('a passphrase for the move');
      await page.getByLabel(t.repeat).press('Enter');
      await expect(toast(page, t.saved)).toBeVisible();
      assert.equal((await vault(page)).passphrase, 'a passphrase for the move');
      await expect(page.getByLabel(t.newPassphrase)).toHaveCount(0);
      await expect(page.locator('.tag').filter({ hasText: new RegExp(`^${t.tag}$`) })).toBeVisible();
      await page.getByRole('button', { name: t.remove }).click();
      await expect(toast(page, t.removed)).toBeVisible();
      await expect(page.getByRole('button', { name: t.set })).toBeVisible();
      assert.equal((await vault(page)).transfer, false);
      console.log(`PASS [${locale}] Settings sets the transfer passphrase (twice, 8 characters that are not spaces) and removes it`);
      await context.close();
    }
  }
  {
    // The Professional's server sign-in after «Later»: a sign-in refused because the passphrase
    // must come first brings the dialog back, since Settings cannot be reached from there.
    const { page, context } = await open(
      { state: 'locked', transfer: true, passphrase: 'correct horse battery' },
      'en-US',
      { edition: 'professional' },
    );
    const dialog = page.getByRole('dialog', { name: TEXT['en-US'].fromAnother });
    await dialog.getByRole('button', { name: 'Later', exact: true }).click();
    await expect(dialog).toHaveCount(0);
    await expect(page.getByRole('heading', { name: 'Sign in to Qamille' })).toBeVisible();
    await page.getByRole('button', { name: 'Sign in', exact: true }).click();
    await page.getByRole('button', { name: 'I agree, continue', exact: true }).click();
    await page.getByLabel('Username or email').fill('mario');
    await page.getByLabel('Password', { exact: true }).fill('una password lunga');
    await page.locator('.server-connect').getByRole('button', { name: 'Sign in', exact: true }).click();
    await expect(dialog).toBeVisible();
    await dialog.getByLabel('Transfer passphrase').fill('correct horse battery');
    await dialog.getByRole('button', { name: 'Open the credentials' }).click();
    await expect(dialog).toHaveCount(0);
    console.log('PASS a sign-in refused for the transfer passphrase brings the dialog back, over the server sign-in');
    await context.close();
  }
  {
    // A vault error from the backend, shown by the page in the interface language.
    const { page, context } = await open({}, 'it-IT');
    const shown = await page.evaluate(async (message) => {
      const { errorText, CommandError } = await import('/src/lib/api.ts');
      return errorText(new CommandError(message, 'vault_passphrase'));
    }, TEXT['en-US'].short);
    assert.equal(shown, TEXT['it-IT'].short);
    console.log('PASS a vault error from a command is worded in Italian');
    await context.close();
  }
  assert.deepEqual(errors, []);
} finally {
  await browser.close();
}
