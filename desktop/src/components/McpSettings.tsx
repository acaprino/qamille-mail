import { useEffect, useRef } from 'react';
import Button from '../ds/primitives/Button';
import Field from '../ds/primitives/Field';
import Select from '../ds/primitives/Select';
import Switch from '../ds/primitives/Switch';
import { call, desktop, errorText } from '../lib/api';
import { tr } from '../lib/i18n';
import { useReactive } from '../lib/store';
import type { Workspace } from '../lib/types';
import AssistantInstallers from './AssistantInstallers';
import { Input, Textarea } from '../ui/Input';

type Profile = {
  access: 'ReadOnly' | 'Triage' | 'Send';
  account_ids: number[] | null;
  calendar_ids: string[] | null;
  ai_execute: boolean;
  permanent_delete: boolean;
};
type Configuration = { profiles: Record<string, Profile>; worker_autostart: boolean };

export type McpSettingsProps = {
  workspace: Workspace;
  notify: (message: string, error?: boolean) => void;
};

/** A plain deep copy, also of the reactive proxies (structuredClone cannot copy a Proxy). */
const copy = <T,>(value: T): T => JSON.parse(JSON.stringify(value)) as T;

export default function McpSettings({ workspace, notify }: McpSettingsProps) {
  // Read after an await and from handlers: always the current props.
  const props = useRef({ workspace, notify });
  props.current = { workspace, notify };
  const state = useReactive(() => ({
    config: { profiles: {}, worker_autostart: false } as Configuration,
    clientConfig: '',
    running: false,
    busy: false,
    ready: false,
    loadError: '',
    loadedProfileName: null as string | null,
    profileName: 'default',
    calendars: '',
    profile: {
      access: workspace.settings.mcp_access,
      account_ids: null,
      calendar_ids: null,
      ai_execute: false,
      permanent_delete: false,
    } as Profile,
  }));

  async function load() {
    state.ready = false;
    state.loadedProfileName = null;
    state.loadError = '';
    if (!desktop) {
      selectProfile();
      state.ready = true;
      return;
    }
    try {
      const value = await call<{
        config: Configuration;
        worker_running: boolean;
        client_config: unknown;
      }>('mcp_configuration');
      state.config = value.config;
      state.running = value.worker_running;
      state.clientConfig = JSON.stringify(value.client_config, null, 2);
      selectProfile();
      state.ready = true;
    } catch (e) {
      state.loadError = errorText(e);
      props.current.notify(state.loadError, true);
    }
  }
  function selectProfile() {
    state.loadedProfileName = null;
    try {
      state.profile = copy(
        state.config.profiles[state.profileName] || {
          access: props.current.workspace.settings.mcp_access,
          account_ids: null,
          calendar_ids: null,
          ai_execute: false,
          permanent_delete: false,
        },
      );
      state.calendars = state.profile.calendar_ids?.join('\n') || '';
      state.loadedProfileName = state.profileName;
      state.loadError = '';
    } catch (e) {
      state.loadError = errorText(e);
      props.current.notify(state.loadError, true);
    }
  }
  function toggleAccount(id: number, checked: boolean) {
    const ids = new Set(state.profile.account_ids || []);
    if (checked) ids.add(id);
    else ids.delete(id);
    state.profile.account_ids = [...ids];
  }
  async function save() {
    if (
      !desktop ||
      !state.ready ||
      state.busy ||
      state.loadedProfileName !== state.profileName ||
      !state.profileName.trim()
    )
      return;
    state.busy = true;
    try {
      const next = copy(state.config);
      const current = copy(state.profile);
      current.calendar_ids =
        current.calendar_ids === null
          ? null
          : state.calendars
              .split(/[\n,]+/)
              .map((v) => v.trim())
              .filter(Boolean);
      next.profiles[state.profileName] = current;
      await call('save_mcp_configuration', { config: next });
      state.config = next;
      props.current.notify(tr('Permessi assistenti salvati', 'Assistant permissions saved'));
    } catch (e) {
      props.current.notify(errorText(e), true);
    } finally {
      state.busy = false;
    }
  }
  async function start() {
    state.busy = true;
    try {
      await call('start_mcp_worker');
      await load();
      props.current.notify(tr('Processo in background avviato', 'Background worker started'));
    } catch (e) {
      props.current.notify(errorText(e), true);
    } finally {
      state.busy = false;
    }
  }
  useEffect(() => {
    void load();
  }, []);

  const { profile } = state;
  return (
    <>
      <div className="mcp-settings">
        <h3>{tr('Connessione MCP', 'MCP connection')}</h3>
        <p className="field-help">
          {tr(
            'Collega un assistente alla posta, alle bozze e al calendario. Il livello di accesso generale qui sopra resta il limite massimo.',
            'Connect an assistant to mail, drafts and calendars. The general access level above remains the maximum permission.',
          )}
        </p>
        {state.ready ? (
          <>
            <Field>
              {tr('Profilo assistente', 'Assistant profile')}
              <Input
                value={state.profileName}
                onChange={(e) => {
                  state.profileName = e.currentTarget.value;
                  selectProfile();
                }}
                placeholder="default"
              />
            </Field>
            <Select
              label={tr('Permesso del profilo', 'Profile permission')}
              value={profile.access}
              onChange={(value) => (state.profile.access = value as Profile['access'])}
              items={[
                { value: 'ReadOnly', label: tr('Solo lettura', 'Read only') },
                { value: 'Triage', label: tr('Organizzazione e bozze', 'Organize and draft') },
                {
                  value: 'Send',
                  label: tr('Invio e calendario remoto', 'Send and remote calendar'),
                },
              ]}
            />
            <div className="switch-list">
              <Switch
                checked={profile.account_ids === null}
                onChange={(on) => (state.profile.account_ids = on ? null : [])}
              >
                {tr('Tutti gli account', 'All accounts')}
              </Switch>
              {profile.account_ids !== null &&
                workspace.accounts.map((account) => (
                  <Switch
                    key={account.id}
                    checked={profile.account_ids?.includes(account.id) ?? false}
                    onChange={(on) => toggleAccount(account.id, on)}
                  >
                    {account.email}
                  </Switch>
                ))}
              <Switch
                checked={profile.calendar_ids === null}
                onChange={(on) => (state.profile.calendar_ids = on ? null : [])}
              >
                {tr('Tutti i calendari', 'All calendars')}
              </Switch>
              {profile.calendar_ids !== null && (
                <Field>
                  {tr(
                    'Calendari consentiti (uno per riga; vuoto = nessuno)',
                    'Allowed calendars (one per line; empty = none)',
                  )}
                  <Textarea
                    rows={2}
                    value={state.calendars}
                    onChange={(e) => (state.calendars = e.currentTarget.value)}
                    placeholder={'local\nprimary'}
                  />
                </Field>
              )}
              <Switch
                checked={profile.ai_execute}
                onChange={(on) => (state.profile.ai_execute = on)}
              >
                {tr(
                  'Consenti elaborazione AI di messaggi e meeting',
                  'Allow AI processing of messages and meetings',
                )}
              </Switch>
              <Switch
                checked={profile.permanent_delete}
                onChange={(on) => (state.profile.permanent_delete = on)}
              >
                {tr(
                  'Consenti eliminazione definitiva delle bozze',
                  'Allow permanent deletion of drafts',
                )}
              </Switch>
            </div>
            {/* `worker_autostart` stays in the configuration, which saves it as loaded: it starts nothing. */}
            <p className="field-help">
              {tr(
                'Il worker non serve più all’accesso: con «Avvia Qamille all’accesso» (Impostazioni → Generali) Qamille parte nell’area di notifica e invia da sé.',
                'The worker is no longer needed at sign-in: with “Start Qamille at Windows sign-in” (Settings → General) Qamille starts in the notification area and sends by itself.',
              )}
            </p>
            <p className="field-help" role="status">
              {state.running
                ? tr('Processo in background attivo', 'Background worker running')
                : tr('Processo in background fermo', 'Background worker stopped')}
            </p>
            <div className="settings-actions">
              <Button
                variant="secondary"
                disabled={state.busy || !desktop}
                onClick={() => void start()}
              >
                {tr('Avvia background', 'Start background worker')}
              </Button>
              <Button
                variant="primary"
                disabled={
                  state.busy ||
                  !desktop ||
                  state.loadedProfileName !== state.profileName ||
                  !state.profileName.trim()
                }
                onClick={() => void save()}
              >
                {tr('Salva permessi assistenti', 'Save assistant permissions')}
              </Button>
            </div>
            {state.clientConfig && (
              <details>
                <summary>
                  {tr(
                    'Configurazione da copiare nel client MCP',
                    'Configuration to copy into the MCP client',
                  )}
                </summary>
                <pre>{state.clientConfig}</pre>
                <p className="field-help">
                  {tr(
                    'Per un profilo diverso da default aggiungi --mcp-profile e il nome del profilo agli argomenti.',
                    'For a profile other than default, add --mcp-profile and the profile name to the arguments.',
                  )}
                </p>
              </details>
            )}
          </>
        ) : (
          !state.loadError && (
            <p role="status">{tr('Caricamento permessi…', 'Loading permissions…')}</p>
          )
        )}
        {state.loadError && (
          <>
            <p role="alert">{state.loadError}</p>
            <Button
              variant="secondary"
              disabled={state.busy || !desktop}
              onClick={() => void load()}
            >
              {tr('Ricarica permessi', 'Reload permissions')}
            </Button>
          </>
        )}
      </div>
      <AssistantInstallers profile={state.profileName} notify={notify} />
    </>
  );
}
