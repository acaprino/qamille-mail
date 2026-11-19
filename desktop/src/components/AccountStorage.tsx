import { useEffect, useRef } from 'react';
import Button from '../ds/primitives/Button';
import Field from '../ds/primitives/Field';
import Icon from '../ds/primitives/Icon';
import Select from '../ds/primitives/Select';
import Switch from '../ds/primitives/Switch';
import { call, CommandError, errorText } from '../lib/api';
import { tr } from '../lib/i18n';
import { regionalLocale } from '../lib/regional';
import { useReactive } from '../lib/store';
import type { Account, AccountStoragePolicy, AccountStorageStatus } from '../lib/types';
import './AccountStorage.css';

export type AccountStorageProps = {
  accounts: Account[];
  changed: () => void;
  notify: (message: string, error?: boolean) => void;
};

/** A plain deep copy, also of the reactive proxies (structuredClone cannot copy a Proxy). */
const copy = <T,>(value: T): T => JSON.parse(JSON.stringify(value)) as T;

function bytes(value: number): string {
  const fixed = (amount: number, digits: number) =>
    amount.toLocaleString(regionalLocale(), {
      minimumFractionDigits: digits,
      maximumFractionDigits: digits,
    });
  if (value >= 1_000_000_000) return `${fixed(value / 1_000_000_000, 2)} GB`;
  if (value >= 1_000_000) return `${fixed(value / 1_000_000, 1)} MB`;
  if (value >= 1_000) return `${fixed(value / 1_000, 1)} KB`;
  return tr(`${value} byte`, value === 1 ? '1 byte' : `${value} bytes`);
}
function remoteOnlyText(count: number): string {
  return count === 1
    ? tr(
        '1 messaggio noto richiede una connessione per essere letto.',
        '1 known message needs a connection to read.',
      )
    : tr(
        `${count} messaggi noti richiedono una connessione per essere letti.`,
        `${count} known messages need a connection to read.`,
      );
}

export default function AccountStorage({ accounts, changed, notify }: AccountStorageProps) {
  // Read after an await: always the current props.
  const props = useRef({ changed, notify });
  props.current = { changed, notify };
  const state = useReactive(() => ({
    accountId: accounts[0]?.id ?? 0,
    status: null as AccountStorageStatus | null,
    loading: false,
    saving: false,
    error: '',
    confirmation: '',
    pendingPolicy: null as AccountStoragePolicy | null,
    mode: 'limited' as 'limited' | 'unlimited',
    limitGb: 2 as number | undefined,
    keepAllOffline: false,
  }));
  const request = useRef(0);
  const accountItems = accounts.map((account) => ({
    value: String(account.id),
    label: `${account.name} · ${account.email}`,
  }));
  const modeItems = [
    { value: 'limited', label: tr('Limitato', 'Limited') },
    { value: 'unlimited', label: tr('Illimitato', 'Unlimited') },
  ];
  const accountIds = accounts.map((account) => account.id).join(',');
  const { status, limitGb } = state;
  const invalidLimit =
    state.mode === 'limited' &&
    (limitGb === undefined ||
      !Number.isFinite(limitGb) ||
      limitGb <= 0 ||
      !Number.isSafeInteger(Math.round(limitGb * 1_000_000_000)));

  const accountId = state.accountId;
  useEffect(() => {
    const ids = accountIds.split(',').filter(Boolean).map(Number);
    if (!ids.includes(accountId)) {
      state.accountId = ids[0] ?? 0;
      return;
    }
    void load(accountId);
  }, [accountIds, accountId]);

  function applyStatus(result: AccountStorageStatus) {
    state.status = result;
    state.mode = result.policy.limit.mode;
    state.limitGb =
      result.policy.limit.mode === 'limited' ? result.policy.limit.max_bytes / 1_000_000_000 : 2;
    state.keepAllOffline = result.policy.keep_all_offline;
  }
  async function load(selected: number) {
    const current = ++request.current;
    state.loading = true;
    state.status = null;
    state.confirmation = '';
    state.pendingPolicy = null;
    state.error = '';
    try {
      const result = await call<AccountStorageStatus>('get_account_storage', {
        accountId: selected,
      });
      if (request.current === current && selected === state.accountId) applyStatus(result);
    } catch (e) {
      if (request.current === current && selected === state.accountId) state.error = errorText(e);
    } finally {
      if (request.current === current) state.loading = false;
    }
  }
  async function refreshUsage() {
    const selected = state.accountId;
    const current = ++request.current;
    state.error = '';
    try {
      const result = await call<AccountStorageStatus>('get_account_storage', {
        accountId: selected,
      });
      // Refresh coverage without replacing unsaved form values.
      if (current === request.current && selected === state.accountId) state.status = result;
    } catch (e) {
      if (current === request.current && selected === state.accountId) state.error = errorText(e);
    }
  }
  function policy(): AccountStoragePolicy {
    return {
      limit:
        state.mode === 'unlimited'
          ? { mode: 'unlimited' }
          : { mode: 'limited', max_bytes: Math.round((state.limitGb ?? 0) * 1_000_000_000) },
      keep_all_offline: state.keepAllOffline,
      // Set from Settings → Search; this page keeps it as it is.
      include_spam_trash: state.status?.policy.include_spam_trash ?? false,
    };
  }
  async function save(confirmCleanup = false) {
    if (state.saving || state.loading || !state.status || invalidLimit) return;
    const selected = state.accountId;
    const current = ++request.current;
    const next = confirmCleanup && state.pendingPolicy ? copy(state.pendingPolicy) : policy();
    state.saving = true;
    state.error = '';
    try {
      const result = await call<AccountStorageStatus>('update_account_storage', {
        accountId: selected,
        policy: next,
        confirmCleanup,
      });
      if (current === request.current && selected === state.accountId) {
        applyStatus(result);
        state.confirmation = '';
        state.pendingPolicy = null;
      }
      props.current.changed();
      props.current.notify(
        tr("Impostazioni dell'archivio salvate", 'Account storage settings saved'),
      );
    } catch (e) {
      if (current !== request.current || selected !== state.accountId) return;
      if (e instanceof CommandError && e.code === 'cleanup_confirmation_required') {
        state.confirmation = errorText(e);
        state.pendingPolicy = next;
      } else {
        state.error = errorText(e);
        state.confirmation = '';
        state.pendingPolicy = null;
      }
    } finally {
      state.saving = false;
    }
  }

  return (
    <>
      <h2>{tr('Archivio per ogni account', 'Storage for each account')}</h2>
      <p className="muted">
        {tr(
          'Scegli un limite o lascia illimitati i dati locali. Ogni account ha il suo spazio.',
          'Choose a limit or keep unlimited local data. Each account has its own space.',
        )}
      </p>

      {accounts.length === 0 ? (
        <p className="field-help">
          {tr(
            "Collega un account email per configurarne l'archivio.",
            'Connect an email account to configure its storage.',
          )}
        </p>
      ) : (
        <>
          <Select
            className="settings-select"
            label={tr("Account dell'archivio", 'Storage account')}
            value={String(state.accountId)}
            items={accountItems}
            disabled={state.saving}
            onChange={(value) => (state.accountId = Number(value))}
          />

          {state.loading ? (
            <p role="status" className="muted">
              {tr("Controllo dell'archivio dell'account…", 'Checking account storage…')}
            </p>
          ) : status ? (
            <>
              <fieldset disabled={state.saving || !!state.confirmation} className="storage-fields">
                <Select
                  className="settings-select"
                  label={tr('Archivio locale', 'Local storage')}
                  value={state.mode}
                  onChange={(value) => (state.mode = value as 'limited' | 'unlimited')}
                  items={modeItems}
                />
                {state.mode === 'limited' ? (
                  <>
                    <Field>
                      {tr('Limite in GB', 'Limit in GB')}
                      <input
                        type="number"
                        aria-label={tr(
                          "Limite dell'archivio dell'account in GB",
                          'Account storage limit in GB',
                        )}
                        min="0.001"
                        step="any"
                        inputMode="decimal"
                        value={limitGb ?? ''}
                        onChange={(e) => {
                          const text = e.currentTarget.value;
                          state.limitGb = text === '' ? undefined : Number(text);
                        }}
                        aria-invalid={invalidLimit}
                      />
                    </Field>
                    {invalidLimit && (
                      <p role="alert" className="inline-error">
                        {tr(
                          'Inserisci un limite maggiore di zero.',
                          'Enter a positive storage limit.',
                        )}
                      </p>
                    )}
                    <p className="field-help">
                      {tr(
                        'Quando serve spazio, viene cancellata la posta locale rimovibile più vecchia di questo account. I messaggi restano sul server; gli altri account conservano i propri dati.',
                        'When space is needed, the oldest removable local mail in this account is cleared. Messages remain on the server; other accounts keep their data.',
                      )}
                    </p>
                  </>
                ) : (
                  <p className="field-help">
                    {tr(
                      'Nessun limite per questo account. I download si fermano se il computer esaurisce lo spazio.',
                      'No account storage limit. Downloads pause if the computer runs out of space.',
                    )}
                  </p>
                )}
                <div className="setting-row">
                  <Switch
                    checked={state.keepAllOffline}
                    onChange={(on) => (state.keepAllOffline = on)}
                  >
                    <strong>{tr('Tieni tutta la posta offline', 'Keep all mail offline')}</strong>
                  </Switch>
                  <p>
                    {tr(
                      'Protegge i testi e gli allegati scaricati dalla pulizia automatica.',
                      'Keep downloaded bodies and attachments protected from automatic cleanup.',
                    )}
                  </p>
                </div>
                {state.keepAllOffline && (
                  <p className="field-help">
                    {tr(
                      "Salva, poi sincronizza la posta per scaricare i contenuti mancanti. Se la posta protetta raggiunge il limite, i download si fermano. L'archivio illimitato da solo non attiva i download offline.",
                      'Save, then use Sync mail to download missing content. If protected mail reaches the limit, downloads pause. Unlimited storage alone does not enable offline downloads.',
                    )}
                  </p>
                )}
              </fieldset>

              <div
                className="settings-note"
                aria-label={tr("Utilizzo dell'archivio dell'account", 'Account storage usage')}
              >
                <Icon name="info" />
                <div>
                  <p>
                    <strong>
                      {tr(`${bytes(status.used_bytes)} in uso`, `${bytes(status.used_bytes)} used`)}
                    </strong>{' '}
                    ·{' '}
                    {status.policy.limit.mode === 'limited'
                      ? tr(
                          `Limite di ${bytes(status.policy.limit.max_bytes)}`,
                          `${bytes(status.policy.limit.max_bytes)} limit`,
                        )
                      : tr('Nessun limite', 'No limit')}
                  </p>
                  <p>
                    {tr(
                      `Protetti: ${bytes(status.protected_bytes)} · rimovibili in locale: ${bytes(status.reclaimable_bytes)}`,
                      `${bytes(status.protected_bytes)} protected · ${bytes(status.reclaimable_bytes)} removable locally`,
                    )}
                  </p>
                  {status.cleanup_pending && (
                    <p role="status">
                      {tr(
                        'Impostazioni salvate. La pulizia locale è in sospeso, quindi parte dello spazio su disco può essere ancora occupata. Aggiorna lo stato per riprovare.',
                        'Settings are saved. Local cleanup is pending, so some disk space may still be occupied. Refresh the status to retry.',
                      )}
                    </p>
                  )}
                  {status.index_rebuild_required && (
                    <p role="status">
                      {tr(
                        "Ricostruisci l'indice di ricerca in Impostazioni → Ricerca. Un indice illeggibile è stato conservato a parte; la posta e le bozze non sono cambiate.",
                        'Rebuild the search index in Settings → Search. An unreadable index was preserved separately; your mail and drafts are unchanged.',
                      )}
                    </p>
                  )}
                  {status.over_limit && (
                    <p role="status">
                      {tr(
                        'Limite di archiviazione raggiunto. I dati protetti non verranno rimossi.',
                        'Storage limit reached. Protected data will not be removed.',
                      )}
                    </p>
                  )}
                  {status.policy.keep_all_offline && (
                    <p role="status">
                      {status.offline_complete
                        ? tr(
                            'I messaggi archiviati sono disponibili offline.',
                            'Stored messages are available offline.',
                          )
                        : tr(
                            'La copia offline è incompleta. Sincronizza la posta per cercare i contenuti mancanti.',
                            'Offline copy is incomplete. Sync mail to check for missing content.',
                          )}
                    </p>
                  )}
                  {status.remote_only_messages > 0 && (
                    <p>{remoteOnlyText(status.remote_only_messages)}</p>
                  )}
                </div>
              </div>
              <p className="field-help">
                {tr(
                  "L'utilizzo comprende i dati della posta nota e le bozze. I componenti condivisi, come il modello di ricerca, sono a parte. La copertura completa della casella viene verificata durante la sincronizzazione.",
                  'Usage covers known mail data and drafts. Shared components such as the search model are separate. Full mailbox coverage is checked during sync.',
                )}
              </p>
              {/* The two actions in one row, apart; the confirmation that a save can ask replaces Save. */}
              <div className="button-row">
                <Button
                  variant="secondary"
                  disabled={state.saving || !!state.confirmation}
                  onClick={() => void refreshUsage()}
                >
                  {tr("Aggiorna lo stato dell'archivio", 'Refresh storage status')}
                </Button>
                {!state.confirmation && (
                  <Button
                    variant="primary"
                    disabled={state.saving || invalidLimit}
                    onClick={() => void save()}
                  >
                    {state.saving
                      ? tr('Salvataggio…', 'Saving…')
                      : tr("Salva l'archivio dell'account", 'Save account storage')}
                  </Button>
                )}
              </div>

              {state.confirmation && (
                <div className="storage-confirmation" role="alert">
                  <p>{state.confirmation}</p>
                  <p>
                    {tr(
                      'Verranno cancellate solo le copie locali rimovibili di questo account. I messaggi sul server e le bozze non sincronizzate restano al sicuro.',
                      'Only removable local copies in this account will be cleared. Server messages and unsynced drafts stay safe.',
                    )}
                  </p>
                  <div className="button-row">
                    <Button
                      variant="primary"
                      disabled={state.saving}
                      onClick={() => void save(true)}
                    >
                      {state.saving
                        ? tr('Applicazione…', 'Applying…')
                        : tr(
                            'Applica il limite e cancella le copie locali',
                            'Apply limit and clear local copies',
                          )}
                    </Button>
                    <Button
                      variant="secondary"
                      disabled={state.saving}
                      onClick={() => {
                        state.confirmation = '';
                        state.pendingPolicy = null;
                      }}
                    >
                      {tr('Annulla', 'Cancel')}
                    </Button>
                  </div>
                </div>
              )}
            </>
          ) : null}
          {state.error && (
            <>
              <p role="alert" className="inline-error">
                {state.error}
              </p>
              {!status && (
                <Button
                  variant="secondary"
                  disabled={state.loading}
                  onClick={() => void load(state.accountId)}
                >
                  {tr('Riprova', 'Retry')}
                </Button>
              )}
            </>
          )}
        </>
      )}
    </>
  );
}
