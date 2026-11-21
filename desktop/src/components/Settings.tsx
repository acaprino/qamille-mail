import { useEffect, useImperativeHandle, useRef, useState, type Ref } from 'react';
import Button from '../ds/primitives/Button';
import Checkbox from '../ds/primitives/Checkbox';
import Field from '../ds/primitives/Field';
import Icon from '../ds/primitives/Icon';
import GoogleMark from '../ds/primitives/GoogleMark';
import Select from '../ds/primitives/Select';
import Switch from '../ds/primitives/Switch';
import { call, desktop, errorText } from '../lib/api';
import { needsReconsent, providerItems as serverProviderItems, reconsentText, type LlmStatus } from '../lib/assistant-server';
import type { Appearance } from '../lib/appearance';
import { cancelGoogle, connectGoogle } from '../lib/google-sign-in';
import { languageChoice, setLanguageChoice, tr, type LanguageChoice } from '../lib/i18n';
import { timeZoneChoices } from '../lib/regional';
import { useReactive } from '../lib/store';
import { tick } from '../lib/tick';
import type { Settings as WorkspaceSettings, Signature, Workspace } from '../lib/types';
import AccountStorage from './AccountStorage';
import MailboxSync from './MailboxSync';
import GoogleSetupGuide from './GoogleSetupGuide';
import GoogleSignInDialog from './GoogleSignInDialog';
import IndexingSettings from './IndexingSettings';
import McpSettings from './McpSettings';
import MeetingSettings from './MeetingSettings';
import ContactsSettings from './contacts/ContactsSettings';
import ComputeSettings from './ComputeSettings';
import DesktopSettings from './DesktopSettings';
import VaultSettings from './VaultSettings';
import './Settings.css';

export type SettingsHandle = {
  /** Opens a section by its key ('General', 'Accounts', 'Local storage', 'Search', 'Meetings'…); unknown keys are ignored. */
  showSection: (name: string) => void;
  /** Drops the unsaved edits of the form. */
  discard: () => void;
  /** Saves the form; resolves to whether it was saved. */
  save: () => Promise<boolean>;
};

export type SettingsProps = {
  workspace: Workspace;
  changed: () => void;
  notify: (message: string, error?: boolean) => void;
  appearance: Appearance;
  appearanceChanged: (value: Appearance) => void;
  /** Whether the form holds unsaved edits: the component reports it through `onDirtyChange`. */
  dirty?: boolean;
  onDirtyChange?: (value: boolean) => void;
  ref?: Ref<SettingsHandle>;
};

/** A plain deep copy, also of the reactive proxies (structuredClone cannot copy a Proxy). */
const copy = <T,>(value: T): T => JSON.parse(JSON.stringify(value)) as T;
const emptySignature = (): Signature => ({ id: 0, name: '', body: '', html: false, modified: 0 });

const SECTION_NAMES: Record<string, [string, string]> = {
  General: ['Generali', 'General'],
  Accounts: ['Account', 'Accounts'],
  'Local storage': ['Archivio locale', 'Local storage'],
  Assistant: ['Assistente', 'Assistant'],
  Signatures: ['Firme', 'Signatures'],
  Search: ['Ricerca', 'Search'],
  Compute: ['Calcolo', 'Compute'],
  Contacts: ['Rubrica', 'Contacts'],
  Meetings: ['Riunioni', 'Meetings'],
  'Agent access': ['Accesso agenti', 'Agent access'],
};
const SECTIONS = Object.keys(SECTION_NAMES);
const sectionName = (key: string) => tr(SECTION_NAMES[key][0], SECTION_NAMES[key][1]);
// Select reads '' as "nothing chosen", so the automatic choice of the time zone and the regional format has a value of its own.
const AUTO = 'auto';

function zoneOffset(zone: string): string {
  try {
    return (
      new Intl.DateTimeFormat('en', { timeZone: zone, timeZoneName: 'shortOffset' })
        .formatToParts(new Date())
        .find((part) => part.type === 'timeZoneName')?.value || ''
    );
  } catch {
    return '';
  }
}

export default function Settings({
  workspace,
  changed,
  notify,
  appearance,
  appearanceChanged,
  onDirtyChange,
  ref,
}: SettingsProps) {
  // Read after an await and from the handle: always the current props.
  const props = useRef({ workspace, changed, notify, onDirtyChange });
  props.current = { workspace, changed, notify, onDirtyChange };
  const baseline = useRef<WorkspaceSettings>(copy(workspace.settings));
  const form = useReactive(() => ({
    settings: copy(workspace.settings),
    section: 'General',
    busy: false,
    aiKey: '',
    googleSecret: '',
    googlePending: false,
    googleDialog: false,
    googleError: '',
    googleGuide: false,
    developerSettings: false,
    email: '',
    name: '',
    password: '',
    signature: emptySignature(),
    zoneSearch: '',
  }));
  const { settings, signature } = form;
  const googleCancelled = useRef(false);
  const disposed = useRef(false);
  const googleClientInput = useRef<HTMLInputElement>(null);
  useEffect(() => {
    disposed.current = false;
    return () => {
      disposed.current = true;
    };
  }, []);
  // The Professional build carries its own Google client: the guide to make one and the fields to enter it
  // are for the self-hosted build only.
  const [professional, setProfessional] = useState(false);
  useEffect(() => {
    void call<{ kind: string }>('app_edition')
      .then((edition) => !disposed.current && setProfessional(edition.kind === 'professional'))
      .catch(() => {});
  }, []);

  // A change made elsewhere, such as the top bar's appearance toggle, reaches this form
  // unless the same field already has an unsaved edit here.
  const incomingKey = JSON.stringify(workspace.settings);
  useEffect(() => {
    const incoming = copy(props.current.workspace.settings);
    const target = form.settings as unknown as Record<string, unknown>;
    const before = baseline.current as unknown as Record<string, unknown>;
    for (const [key, value] of Object.entries(incoming)) {
      if (JSON.stringify(target[key]) === JSON.stringify(before[key])) target[key] = value;
    }
    baseline.current = incoming;
  }, [incomingKey, form]);

  const unsaved = JSON.stringify(settings) !== incomingKey || !!form.aiKey || !!form.googleSecret;
  useEffect(() => {
    props.current.onDirtyChange?.(unsaved);
  }, [unsaved]);

  const [zones] = useState(() => timeZoneChoices());
  // The assistant of the connected server: `unavailable` when not connected or without one.
  const [llmStatus, setLlmStatus] = useState<LlmStatus | null>(null);
  useEffect(() => {
    let live = true;
    call<LlmStatus>('server_llm_status')
      .then((status) => live && setLlmStatus(status))
      .catch(() => live && setLlmStatus(null));
    return () => {
      live = false;
    };
  }, []);
  const shownZones = zones.filter(
    (zone) =>
      zone.value === settings.time_zone ||
      zone.label.toLowerCase().includes(form.zoneSearch.toLowerCase()),
  );
  const zoneItems = [
    { value: AUTO, label: tr('Automatico · fuso del sistema', 'Automatic · system time zone') },
    ...shownZones.map((zone) => ({
      value: zone.value,
      label: `${zone.label} · ${zoneOffset(zone.value)}`,
    })),
  ];
  const languageItems = [
    {
      value: 'auto',
      label: tr(
        'Automatica · segue il formato regionale',
        'Automatic · follows the regional format',
      ),
    },
    { value: 'it', label: 'Italiano' },
    { value: 'en', label: 'English' },
  ];
  const localeItems = [
    {
      value: AUTO,
      label: tr('Automatico · formato del sistema', 'Automatic · system regional format'),
    },
    ...['it-IT', 'en-US', 'en-GB', 'fr-FR', 'de-DE', 'es-ES', 'pt-BR', 'ja-JP'].map((locale) => ({
      value: locale,
      label: locale,
    })),
  ];
  const undoItems = [5, 10, 15, 20, 30].map((seconds) => ({
    value: String(seconds),
    label: tr(`${seconds} secondi`, `${seconds} seconds`),
  }));
  const appearanceItems = [
    { value: 'system', label: tr('Automatico (come Windows)', 'Automatic (system)') },
    { value: 'light', label: tr('Chiaro', 'Light') },
    { value: 'dark', label: tr('Scuro', 'Dark') },
  ];
  // «Server Qamille» first when the connected server has an assistant; the providers with a key
  // of the person come after, under «Usa una tua chiave».
  const providerItems = serverProviderItems(llmStatus).map((item) => ({
    value: item.value,
    label:
      item.ownKey && llmStatus && llmStatus.kind !== 'unavailable'
        ? `${tr('Usa una tua chiave', 'Use your own key')} · ${tr(item.label.it, item.label.en)}`
        : tr(item.label.it, item.label.en),
  }));
  const reconsent = reconsentText(llmStatus);
  const accessItems = [
    { value: 'ReadOnly', label: tr('Solo lettura', 'Read only') },
    { value: 'Triage', label: tr('Lettura e organizzazione', 'Read and organize') },
    { value: 'Send', label: tr('Lettura, organizzazione e invio', 'Read, organize and send') },
  ];

  async function run(action: () => Promise<unknown>, message: string): Promise<boolean> {
    form.busy = true;
    try {
      await action();
      props.current.notify(message);
      props.current.changed();
      return true;
    } catch (e) {
      props.current.notify(errorText(e), true);
      return false;
    } finally {
      form.busy = false;
    }
  }
  async function signInWithGoogle() {
    form.googleDialog = true;
    if (form.googlePending) return;
    form.googlePending = true;
    form.googleError = '';
    googleCancelled.current = false;
    try {
      await connectGoogle(form.email);
      if (disposed.current) return;
      form.googleDialog = false;
      props.current.notify(tr('Account Google collegato', 'Google account connected'));
      props.current.changed();
    } catch (error) {
      if (disposed.current) return;
      if (googleCancelled.current) {
        form.googleDialog = false;
        return;
      }
      form.googleError = errorText(error);
      if (!form.googleDialog) props.current.notify(form.googleError, true);
    } finally {
      if (!disposed.current) form.googlePending = false;
    }
  }
  function cancelGoogleSignIn() {
    googleCancelled.current = true;
    cancelGoogle().catch(() => {});
  }
  function showGoogleGuide() {
    form.googleDialog = false;
    form.googleGuide = true;
  }
  async function configureGoogle() {
    form.googleGuide = false;
    form.section = 'Accounts';
    form.developerSettings = true;
    await tick();
    googleClientInput.current?.focus();
  }
  function showSection(name: string) {
    if (name in SECTION_NAMES) form.section = name;
  }
  function discard() {
    form.settings = copy(props.current.workspace.settings);
    baseline.current = copy(props.current.workspace.settings);
    form.aiKey = '';
    form.googleSecret = '';
  }
  async function save(): Promise<boolean> {
    const saved = await run(
      () =>
        call('save_settings', {
          update: {
            settings: copy(form.settings),
            ai_key: form.aiKey || null,
            google_client_secret: form.googleSecret || null,
          },
        }),
      tr('Impostazioni salvate', 'Settings saved'),
    );
    if (saved) {
      form.aiKey = '';
      form.googleSecret = '';
      baseline.current = copy(form.settings);
    }
    return saved;
  }
  useImperativeHandle(ref, () => ({ showSection, discard, save }));

  const section = form.section;
  const hasGmail = workspace.accounts.some((account) => account.kind === 'Gmail');
  return (
    <>
      <section className="settings-page">
        <div className="page-heading">
          <div>
            <span className="page-kicker">{tr('Il tuo spazio di lavoro', 'Your workspace')}</span>
            <h1>{tr('Impostazioni', 'Settings')}</h1>
            <p>{tr('Rendi Qamille davvero tuo.', 'Make Qamille feel like yours.')}</p>
          </div>
          {section !== 'Local storage' && section !== 'Contacts' && (
            <div className="settings-actions">
              {unsaved && (
                <>
                  <span className="unsaved-note" role="status">
                    {tr('Modifiche non salvate', 'Unsaved changes')}
                  </span>
                  <Button variant="secondary" disabled={form.busy} onClick={discard}>
                    {tr('Annulla modifiche', 'Discard')}
                  </Button>
                </>
              )}
              <Button
                variant="primary"
                disabled={form.busy || !unsaved}
                onClick={() => void save()}
              >
                {form.busy ? tr('Attendi…', 'Working…') : tr('Salva modifiche', 'Save changes')}
              </Button>
            </div>
          )}
        </div>
        <div className="settings-layout">
          <nav
            className="settings-nav"
            aria-label={tr('Sezioni delle impostazioni', 'Settings sections')}
          >
            {SECTIONS.map((label) => (
              <button
                key={label}
                className={section === label ? 'active' : undefined}
                aria-current={section === label ? 'page' : undefined}
                onClick={() => (form.section = label)}
              >
                {sectionName(label)}
              </button>
            ))}
          </nav>
          <div className="settings-content">
            {section === 'General' ? (
              <>
                <h2>{tr('Uno spazio tutto tuo', 'A workspace of your own')}</h2>
                <p className="muted">
                  {tr(
                    'Il tuo nome compare nei messaggi in uscita.',
                    'Your name appears on outgoing messages.',
                  )}
                </p>
                <Field>
                  {tr('Il tuo nome', 'Your name')}
                  <input
                    value={settings.user_name}
                    onChange={(e) => (form.settings.user_name = e.currentTarget.value)}
                  />
                </Field>
                <Field>
                  {tr('Cerca una località per il fuso orario', 'Find a city for the time zone')}
                  <input
                    value={form.zoneSearch}
                    onChange={(e) => (form.zoneSearch = e.currentTarget.value)}
                    placeholder={tr('Roma, Londra, Havana…', 'Rome, London, Havana…')}
                    type="search"
                  />
                </Field>
                <Select
                  className="settings-select"
                  label={tr('Fuso orario', 'Time zone')}
                  value={settings.time_zone || AUTO}
                  items={zoneItems}
                  onChange={(value) => (form.settings.time_zone = value === AUTO ? '' : value)}
                />
                <p className="field-help">
                  {tr(
                    'Date, ricerca e calendario usano questo fuso. La località viene cercata sul computer.',
                    'Dates, search and calendar use this time zone. City lookup stays on this computer.',
                  )}
                </p>
                <Select
                  className="settings-select"
                  label={tr('Lingua', 'Language')}
                  value={languageChoice()}
                  items={languageItems}
                  onChange={(value) => setLanguageChoice(value as LanguageChoice)}
                />
                <p className="field-help">
                  {tr(
                    'Si applica subito su questo computer.',
                    'Applies right away on this computer.',
                  )}
                </p>
                <Select
                  className="settings-select"
                  label={tr('Formato regionale', 'Regional format')}
                  value={settings.locale || AUTO}
                  items={localeItems}
                  onChange={(value) => (form.settings.locale = value === AUTO ? '' : value)}
                />
                <Select
                  className="settings-select"
                  label={tr('Tempo per annullare', 'Undo time')}
                  value={String(settings.undo_seconds)}
                  items={undoItems}
                  onChange={(value) => (form.settings.undo_seconds = Number(value))}
                />
                <p className="field-help">
                  {tr(
                    "Tempo per annullare le azioni sulla posta, compresi il completamento e l'invio. I messaggi partono al termine del conto alla rovescia. Tieni aperto Qamille per completare le azioni in sospeso.",
                    'Time to undo email actions, including Done and Send. Messages are sent after this countdown. Keep Qamille open to finish pending actions.',
                  )}
                </p>
                <Select
                  className="settings-select"
                  label={tr('Aspetto', 'Appearance')}
                  value={appearance}
                  items={appearanceItems}
                  onChange={(value) => appearanceChanged(value as Appearance)}
                />
                <p className="field-help">
                  {tr(
                    'Automatico segue il tema di sistema e aggiorna subito anche il contenuto delle email HTML.',
                    'Automatic follows the system theme and immediately updates HTML email content too.',
                  )}
                </p>
                <div className="setting-row">
                  <Switch
                    checked={settings.auto_classify}
                    onChange={(on) => (form.settings.auto_classify = on)}
                  >
                    <strong>{tr('Organizza la posta in arrivo', 'Organize incoming mail')}</strong>
                  </Switch>
                  <p>
                    {tr(
                      'Smista i messaggi nelle categorie della Smart Inbox.',
                      'Sort messages into Smart Inbox categories.',
                    )}
                  </p>
                </div>
                <div className="setting-row">
                  <Switch
                    checked={settings.auto_tag}
                    onChange={(on) => (form.settings.auto_tag = on)}
                  >
                    <strong>{tr('Suggerisci etichette', 'Suggest labels')}</strong>
                  </Switch>
                  <p>
                    {tr(
                      'Etichetta i messaggi in base al contenuto.',
                      'Label messages based on their content.',
                    )}
                  </p>
                </div>
                {desktop && <DesktopSettings />}
              </>
            ) : section === 'Accounts' ? (
              <>
                <h2>{tr('I tuoi account email', 'Your email accounts')}</h2>
                <p className="muted">
                  {tr(
                    'Sono supportati gli account Gmail e Google Workspace.',
                    'Gmail and Google Workspace accounts are supported.',
                  )}
                </p>
                {workspace.accounts.map((account) => (
                  <div key={account.id} className="connected-account">
                    <span
                      className="account-color"
                      style={{ background: `rgb(${account.color.join(',')})` }}
                    />
                    <div>
                      <strong>{account.name}</strong>
                      <p>{account.email}</p>
                    </div>
                    <span className="tag">
                      {account.kind === 'Sample'
                        ? tr('Esempio', 'Sample')
                        : tr('Collegato', 'Connected')}
                    </span>
                  </div>
                ))}
                <MailboxSync accounts={workspace.accounts} changed={changed} notify={notify} />
                {/* With a Gmail mailbox already here, Google is for adding another one, not for signing in again. */}
                <h3>{hasGmail ? tr('Aggiungi un account Google', 'Add a Google account') : tr('Accedi con Google', 'Sign in with Google')}</h3>
                <p className="muted">
                  {hasGmail
                    ? tr(
                        'Collega un altro indirizzo Gmail o Google Workspace: la pagina di Google si apre nel browser.',
                        'Connect another Gmail or Google Workspace address: Google’s page opens in your browser.',
                      )
                    : tr(
                        'Scegli il tuo account o inserisci email e password nella pagina di Google che si apre nel browser.',
                        'Choose your account or enter your email and password on Google’s page in your browser.',
                      )}
                </p>
                {/* Google's own look for its sign-in button: neutral, with its «G». */}
                <Button
                  variant="secondary"
                  disabled={form.busy}
                  onClick={() => void signInWithGoogle()}
                >
                  <GoogleMark />
                  {form.googlePending
                    ? tr('Mostra avanzamento accesso', 'Show sign-in progress')
                    : hasGmail
                      ? tr('Aggiungi un account Google', 'Add a Google account')
                      : tr('Accedi con Google', 'Sign in with Google')}
                </Button>
                <div className="google-setup-help" hidden={hasGmail || professional}>
                  <p>
                    {tr(
                      'Prima configurazione di Qamille?',
                      'Setting up Qamille for the first time?',
                    )}
                  </p>
                  <Button variant="text" onClick={showGoogleGuide}>
                    {tr("Come attivare l'accesso Google", 'How to set up Google sign-in')}
                  </Button>
                </div>
                <details
                  className="settings-details"
                  hidden={professional}
                  open={form.developerSettings}
                  onToggle={(e) => (form.developerSettings = e.currentTarget.open)}
                >
                  <summary>{tr('Opzioni sviluppatore', 'Developer settings')}</summary>
                  <p className="field-help">
                    {tr(
                      'Per usare un tuo progetto Google, inserisci ID e secret seguendo la guida. Puoi lasciarli vuoti solo se questa versione include già una configurazione Google.',
                      'To use your own Google project, enter its ID and secret following the guide. Leave them blank only if this version already includes a Google configuration.',
                    )}
                  </p>
                  <Field>
                    {tr('ID client desktop di Google', 'Google desktop client ID')}
                    <input
                      ref={googleClientInput}
                      value={settings.google_client_id}
                      onChange={(e) => (form.settings.google_client_id = e.currentTarget.value)}
                      placeholder={tr('Il tuo ID client OAuth', 'Your OAuth client ID')}
                      disabled={form.googlePending}
                    />
                  </Field>
                  <Field>
                    {tr('Client secret di Google', 'Google client secret')}
                    <input
                      type="password"
                      value={form.googleSecret}
                      onChange={(e) => (form.googleSecret = e.currentTarget.value)}
                      placeholder={
                        workspace.has_google_secret
                          ? tr(
                              'Salvato in modo sicuro: lascia vuoto per mantenerlo',
                              'Saved securely — leave blank to keep',
                            )
                          : 'Client secret'
                      }
                      autoComplete="off"
                      disabled={form.googlePending}
                    />
                  </Field>
                  <Button
                    variant="secondary"
                    disabled={form.busy || form.googlePending}
                    onClick={() => void save()}
                  >
                    {tr('Salva configurazione Google', 'Save Google configuration')}
                  </Button>
                  <p className="google-config-help">
                    {tr(
                      'Dopo il salvataggio, premi Accedi con Google.',
                      'After saving, select Sign in with Google.',
                    )}
                  </p>
                </details>
                <details className="settings-details">
                  <summary>
                    {tr('Usa una password per le app di Google', 'Use a Google app password')}
                  </summary>
                  <Field>
                    {tr("Nome dell'account", 'Account name')}
                    <input
                      value={form.name}
                      onChange={(e) => (form.name = e.currentTarget.value)}
                    />
                  </Field>
                  <Field>
                    Email
                    <input
                      type="email"
                      value={form.email}
                      onChange={(e) => (form.email = e.currentTarget.value)}
                    />
                  </Field>
                  <Field>
                    {tr('Password per le app', 'App password')}
                    <input
                      type="password"
                      value={form.password}
                      onChange={(e) => (form.password = e.currentTarget.value)}
                      autoComplete="off"
                    />
                  </Field>
                  <Button
                    variant="secondary"
                    disabled={form.busy || form.googlePending || !form.email || !form.password}
                    onClick={() =>
                      void run(
                        async () => {
                          await call('connect_password', {
                            name: form.name,
                            email: form.email,
                            password: form.password,
                          });
                          form.password = '';
                        },
                        tr('Account collegato', 'Account connected'),
                      )
                    }
                  >
                    {tr('Collega account', 'Connect account')}
                  </Button>
                </details>
                <VaultSettings notify={notify} changed={changed} />
              </>
            ) : section === 'Local storage' ? (
              <AccountStorage accounts={workspace.accounts} changed={changed} notify={notify} />
            ) : section === 'Assistant' ? (
              <>
                <h2>{tr('Scrittura e risposte', 'Writing and answers')}</h2>
                <p className="muted">
                  {tr(
                    'Scegli dove elaborare riassunti e suggerimenti di scrittura.',
                    'Choose where summaries and writing suggestions are processed.',
                  )}
                </p>
                <Select
                  className="settings-select"
                  label="Provider"
                  value={settings.ai_provider}
                  onChange={(value) =>
                    (form.settings.ai_provider = value as WorkspaceSettings['ai_provider'])
                  }
                  items={providerItems}
                />
                {reconsent && needsReconsent(llmStatus) && (
                  <div className="field-help" role="alert">
                    <p>{tr(reconsent.it, reconsent.en)}</p>
                    <Button
                      variant="secondary"
                      onClick={() =>
                        void run(async () => {
                          const status = await call<{ url: string | null }>('server_status');
                          await call('server_record_llm_consent', { url: status.url });
                          setLlmStatus(await call<LlmStatus>('server_llm_status'));
                        }, tr('Provider confermato.', 'Provider confirmed.'))
                      }
                    >
                      {tr('Conferma', 'Confirm')}
                    </Button>
                  </div>
                )}
                {settings.ai_provider === 'QamilleServer' && (
                  <p className="field-help">
                    {tr(
                      'Usa il server collegato: non servono chiavi. Il testo da elaborare va a quel server.',
                      'Uses the connected server: no keys needed. The text to process goes to that server.',
                    )}
                  </p>
                )}
                {settings.ai_provider !== 'Offline' && settings.ai_provider !== 'QamilleServer' && (
                  <>
                    <Field>
                      {tr('Modello', 'Model')}
                      <input
                        value={settings.ai_model}
                        onChange={(e) => (form.settings.ai_model = e.currentTarget.value)}
                        placeholder={tr('Nome del modello', 'Model name')}
                      />
                    </Field>
                    <Field>
                      Endpoint
                      <input
                        value={settings.ai_base_url}
                        onChange={(e) => (form.settings.ai_base_url = e.currentTarget.value)}
                        placeholder="https://…"
                      />
                    </Field>
                    <Field>
                      {tr('Chiave API', 'API key')}
                      <input
                        type="password"
                        value={form.aiKey}
                        onChange={(e) => (form.aiKey = e.currentTarget.value)}
                        placeholder={
                          workspace.has_ai_key
                            ? tr(
                                'Salvata in modo sicuro: lascia vuoto per mantenerla',
                                'Saved securely — leave blank to keep',
                              )
                            : tr('Chiave API', 'API key')
                        }
                        autoComplete="off"
                      />
                    </Field>
                    <p className="field-help">
                      {tr(
                        'Un provider online riceve il testo che gli chiedi di elaborare.',
                        'An online provider receives the text you ask it to process.',
                      )}
                    </p>
                  </>
                )}
              </>
            ) : section === 'Signatures' ? (
              <>
                <h2>{tr('Firma a modo tuo', 'Sign off your way')}</h2>
                <p className="muted">
                  {tr(
                    'Scegli una firma quando scrivi un messaggio.',
                    'Choose a signature when composing a message.',
                  )}
                </p>
                <div className="signature-list">
                  {workspace.signatures.map((s) => (
                    <button
                      key={s.id}
                      className={signature.id === s.id ? 'chosen' : undefined}
                      aria-pressed={signature.id === s.id}
                      onClick={() => (form.signature = copy(s))}
                    >
                      {s.name}
                      <Icon name="chevron" size={16} />
                    </button>
                  ))}
                  {/* A new signature being written is the chosen one, as a saved one is when open. */}
                  <button
                    className={workspace.signatures.some((x) => x.id === signature.id) ? undefined : 'chosen'}
                    aria-pressed={!workspace.signatures.some((x) => x.id === signature.id)}
                    onClick={() => (form.signature = emptySignature())}
                  >
                    <Icon name="plus" size={16} />
                    {tr('Nuova firma', 'New signature')}
                  </button>
                </div>
                <Field>
                  {tr('Nome', 'Name')}
                  <input
                    value={signature.name}
                    onChange={(e) => (form.signature.name = e.currentTarget.value)}
                  />
                </Field>
                <Field>
                  {tr('Testo della firma', 'Signature text')}
                  <textarea
                    rows={7}
                    value={signature.body}
                    onChange={(e) => (form.signature.body = e.currentTarget.value)}
                  />
                </Field>
                <div className="settings-check">
                  <Checkbox
                    checked={signature.html}
                    onChange={(checked) => (form.signature.html = checked)}
                  >
                    {tr('Questa firma contiene HTML', 'This signature contains HTML')}
                  </Checkbox>
                </div>
                <Button
                  variant="secondary"
                  disabled={form.busy || !signature.name.trim()}
                  onClick={() =>
                    void run(
                      async () => {
                        form.signature = await call<Signature>('save_signature', {
                          signature: copy(form.signature),
                        });
                      },
                      tr('Firma salvata', 'Signature saved'),
                    )
                  }
                >
                  {tr('Salva firma', 'Save signature')}
                </Button>
              </>
            ) : section === 'Compute' ? (
              <ComputeSettings />
            ) : section === 'Search' ? (
              <IndexingSettings />
            ) : section === 'Contacts' ? (
              <ContactsSettings notify={notify} />
            ) : section === 'Meetings' ? (
              // MeetingSettings edits the form's own fields (Svelte's bind:settings): the form is
              // a reactive store, so a write in place and a whole new value both land in it.
              <MeetingSettings
                settings={form.settings}
                onSettingsChange={(next: WorkspaceSettings) => Object.assign(form.settings, next)}
                notify={notify}
              />
            ) : (
              <>
                <h2>{tr('Accesso agenti', 'Agent access')}</h2>
                <p className="muted">
                  {tr(
                    'Decidi cosa possono fare gli assistenti esterni tramite la connessione MCP.',
                    'Control what external assistants can do through the MCP connection.',
                  )}
                </p>
                <Select
                  className="settings-select"
                  label={tr('Livello di accesso', 'Access level')}
                  value={settings.mcp_access}
                  onChange={(value) =>
                    (form.settings.mcp_access = value as WorkspaceSettings['mcp_access'])
                  }
                  items={accessItems}
                />
                <p className="field-help">
                  {tr("Avvia l'eseguibile desktop con", 'Start the desktop executable with')}{' '}
                  <code>--mcp</code>{' '}
                  {tr('per collegare un assistente.', 'to connect an assistant.')}
                </p>
                <McpSettings workspace={workspace} notify={notify} />
              </>
            )}
          </div>
        </div>
      </section>

      {form.googleDialog && (
        <GoogleSignInDialog
          pending={form.googlePending}
          error={form.googleError}
          hide={() => (form.googleDialog = false)}
          cancel={cancelGoogleSignIn}
          retry={() => void signInWithGoogle()}
          setup={showGoogleGuide}
        />
      )}
      {form.googleGuide && (
        <GoogleSetupGuide
          close={() => (form.googleGuide = false)}
          configure={() => void configureGoogle()}
        />
      )}
    </>
  );
}
