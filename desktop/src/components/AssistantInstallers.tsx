import { useEffect, useRef } from 'react';
import Button from '../ui/Button';
import Switch from '../ui/Switch';
import { call, desktop, errorText } from '../lib/api';
import { backendText, tr } from '../lib/i18n';
import { useReactive } from '../lib/store';
import './AssistantInstallers.css';

type TargetId = 'codex' | 'claude_code' | 'claude_desktop' | 'chatgpt' | 'claude_cowork';
type Target = {
  id: TargetId;
  name: string;
  mode: 'direct' | 'package';
  detected: boolean;
  installed: boolean;
  prepared?: boolean;
  requires_import?: boolean;
  notes?: string[];
  supports_mcp: boolean;
  supports_skill: boolean;
};
type Options = { target: TargetId; profile: string; installMcp: boolean; installSkill: boolean };
type Plan = {
  target: TargetId;
  token: string;
  files: { path: string; action: string }[];
  notes: string[];
  requires_import: boolean;
};
type Result = {
  target: TargetId;
  changed_paths: string[];
  backup_paths: string[];
  notes: string[];
  package_path?: string;
  open_url?: string;
};
type SelectedPlan = { plan: Plan; options: Options; selection: string };

export type AssistantInstallersProps = {
  profile: string;
  notify: (message: string, error?: boolean) => void;
};

export default function AssistantInstallers({ profile, notify }: AssistantInstallersProps) {
  // Read after an await: always the current props.
  const props = useRef({ profile, notify });
  props.current = { profile, notify };
  const state = useReactive(() => ({
    targets: [
      {
        id: 'codex',
        name: 'Codex',
        mode: 'direct',
        detected: false,
        installed: false,
        supports_mcp: true,
        supports_skill: true,
      },
      {
        id: 'claude_code',
        name: 'Claude Code',
        mode: 'direct',
        detected: false,
        installed: false,
        supports_mcp: true,
        supports_skill: true,
      },
      {
        id: 'claude_desktop',
        name: 'Claude Desktop',
        mode: 'direct',
        detected: false,
        installed: false,
        supports_mcp: true,
        supports_skill: true,
      },
      {
        id: 'chatgpt',
        name: 'ChatGPT Work',
        mode: 'package',
        detected: false,
        installed: false,
        supports_mcp: true,
        supports_skill: true,
      },
      {
        id: 'claude_cowork',
        name: 'Claude Cowork',
        mode: 'package',
        detected: false,
        installed: false,
        supports_mcp: true,
        supports_skill: true,
      },
    ] as Target[],
    loading: desktop,
    installMcp: true,
    installSkill: true,
    pendingTarget: null as TargetId | null,
    pendingAction: null as 'preview' | 'apply' | 'open' | null,
    selected: null as SelectedPlan | null,
    result: null as (Result & { requiresImport: boolean }) | null,
    error: '',
  }));
  const active = useRef(true);
  const generation = useRef(0);
  const previousSelection = useRef('');
  const busy = state.pendingTarget !== null;
  const selectionNow = () =>
    JSON.stringify([props.current.profile, state.installMcp, state.installSkill]);
  const selection = selectionNow();

  useEffect(() => {
    if (previousSelection.current !== selection) {
      previousSelection.current = selection;
      ++generation.current;
      state.selected = null;
      state.result = null;
      state.error = '';
    }
  }, [selection, state]);

  function options(target: Target): Options {
    return {
      target: target.id,
      profile: props.current.profile.trim(),
      installMcp: state.installMcp && target.supports_mcp,
      installSkill: state.installSkill && target.supports_skill,
    };
  }
  function hasSelection(target: Target): boolean {
    return (
      (state.installMcp && target.supports_mcp) || (state.installSkill && target.supports_skill)
    );
  }
  function description(target: Target): string {
    switch (target.id) {
      case 'codex':
        return tr(
          'Collega Qamille e aggiunge la skill personale per Codex.',
          'Connect Qamille and add the personal Codex skill.',
        );
      case 'claude_code':
        return tr(
          'Collega Qamille e aggiunge la skill personale per Claude Code.',
          'Connect Qamille and add the personal Claude Code skill.',
        );
      case 'claude_desktop':
        return tr(
          'Configura il server MCP e prepara la skill ZIP da importare in Claude.',
          'Configure the MCP server and prepare the skill ZIP to import into Claude.',
        );
      case 'chatgpt':
        return tr(
          'Prepara il plugin locale per ChatGPT Work sul computer.',
          'Prepare the local plugin for ChatGPT Work on this computer.',
        );
      case 'claude_cowork':
        return tr(
          'Prepara l’estensione MCP e la skill ZIP da importare in Claude Cowork.',
          'Prepare the MCP extension and skill ZIP to import into Claude Cowork.',
        );
    }
  }
  function status(target: Target): string {
    if (!desktop) return tr('Anteprima web', 'Web preview');
    if (target.installed) return tr('Configurato', 'Configured');
    if (target.prepared) return tr('Pronto da importare', 'Ready to import');
    return target.detected
      ? tr('Rilevato sul PC', 'Detected on this PC')
      : tr('Disponibile', 'Available');
  }
  function prepareLabel(target: Target): string {
    if (target.id === 'chatgpt') return tr('Prepara plugin', 'Prepare plugin');
    if (target.id === 'claude_cowork') {
      if (state.installMcp && state.installSkill)
        return tr('Prepara estensione e skill', 'Prepare extension and skill');
      return state.installMcp
        ? tr('Prepara estensione MCP', 'Prepare MCP extension')
        : tr('Prepara skill', 'Prepare skill');
    }
    return tr('Prepara integrazione', 'Prepare integration');
  }
  function actionLabel(action: string): string {
    const labels: Record<string, string> = {
      create: tr('Crea', 'Create'),
      created: tr('Crea', 'Create'),
      add: tr('Aggiunge', 'Add'),
      update: tr('Aggiorna', 'Update'),
      replace: tr('Aggiorna', 'Update'),
      copy: tr('Copia', 'Copy'),
      unchanged: tr('Già aggiornato', 'Already up to date'),
      no_change: tr('Già aggiornato', 'Already up to date'),
      backup: tr('Copia di sicurezza', 'Backup'),
      write: tr('Salva', 'Save'),
    };
    return labels[action] ?? backendText(action);
  }
  function report(cause: unknown): void {
    state.error = errorText(cause);
    props.current.notify(state.error, true);
  }
  async function refresh(): Promise<void> {
    if (!desktop || !active.current) return;
    state.loading = true;
    try {
      const value = await call<{ targets: Target[]; executable_path: string }>(
        'assistant_installation_status',
      );
      if (active.current) {
        state.targets = value.targets;
        state.error = '';
      }
    } catch (cause) {
      if (active.current) report(cause);
    } finally {
      if (active.current) state.loading = false;
    }
  }
  async function preview(target: Target): Promise<void> {
    if (
      !desktop ||
      state.pendingTarget !== null ||
      !hasSelection(target) ||
      !props.current.profile.trim()
    )
      return;
    const request = ++generation.current;
    const selectedOptions = options(target);
    const selectedKey = selectionNow();
    state.pendingTarget = target.id;
    state.pendingAction = 'preview';
    state.selected = null;
    state.result = null;
    state.error = '';
    try {
      const plan = await call<Plan>('assistant_installation_preview', selectedOptions);
      if (active.current && request === generation.current && selectedKey === selectionNow()) {
        state.selected = { plan, options: selectedOptions, selection: selectedKey };
      }
    } catch (cause) {
      if (active.current && request === generation.current) report(cause);
    } finally {
      if (active.current) {
        state.pendingTarget = null;
        state.pendingAction = null;
      }
    }
  }
  async function apply(): Promise<void> {
    const current = state.selected
      ? (JSON.parse(JSON.stringify(state.selected)) as SelectedPlan)
      : null;
    if (
      !desktop ||
      state.pendingTarget !== null ||
      !current ||
      current.selection !== selectionNow()
    )
      return;
    const request = ++generation.current;
    state.pendingTarget = current.plan.target;
    state.pendingAction = 'apply';
    state.error = '';
    try {
      const value = await call<Result>('assistant_installation_apply', {
        ...current.options,
        token: current.plan.token,
      });
      if (
        active.current &&
        request === generation.current &&
        current.selection === selectionNow()
      ) {
        state.result = { ...value, requiresImport: current.plan.requires_import };
        state.selected = null;
        props.current.notify(
          current.plan.requires_import
            ? tr(
                'Pacchetto pronto: completa l’importazione nell’assistente.',
                'Package ready: finish importing it in the assistant.',
              )
            : tr('Integrazione Qamille configurata.', 'Qamille integration configured.'),
        );
      }
      if (active.current) await refresh();
    } catch (cause) {
      if (active.current && request === generation.current) {
        state.selected = null;
        report(cause);
      }
    } finally {
      if (active.current) {
        state.pendingTarget = null;
        state.pendingAction = null;
      }
    }
  }
  async function open(target: TargetId): Promise<void> {
    if (!desktop || state.pendingTarget !== null) return;
    state.pendingTarget = target;
    state.pendingAction = 'open';
    try {
      await call('assistant_installation_open', { target });
    } catch (cause) {
      if (active.current) report(cause);
    } finally {
      if (active.current) {
        state.pendingTarget = null;
        state.pendingAction = null;
      }
    }
  }
  useEffect(() => {
    active.current = true;
    void refresh();
    return () => {
      active.current = false;
      ++generation.current;
    };
  }, []);

  const { selected, result } = state;
  return (
    <section className="assistant-installers" aria-labelledby="assistant-installers-heading">
      <h3 id="assistant-installers-heading">
        {tr('Installa negli assistenti', 'Install in your assistants')}
      </h3>
      <p className="installer-help">
        {tr(
          'Scegli cosa aggiungere e controlla i file prima di installare.',
          'Choose what to add and review the files before installing.',
        )}
      </p>
      <p className="installer-help">
        {tr('Profilo MCP', 'MCP profile')}:{' '}
        <strong>{profile || tr('da scegliere', 'choose a profile')}</strong>
      </p>
      <fieldset disabled={busy || state.loading} className="installer-options">
        <legend>{tr('Cosa installare', 'What to install')}</legend>
        <div className="switch-list">
          <Switch
            checked={state.installMcp}
            onChange={(on) => (state.installMcp = on)}
            disabled={busy || state.loading}
          >
            {tr('Collegamento MCP', 'MCP connection')}
          </Switch>
          <Switch
            checked={state.installSkill}
            onChange={(on) => (state.installSkill = on)}
            disabled={busy || state.loading}
          >
            {tr('Skill Qamille', 'Qamille skill')}
          </Switch>
        </div>
      </fieldset>
      {!desktop && (
        <p className="installer-help">
          {tr(
            'L’installazione sul PC è disponibile nell’app desktop.',
            'Installation on this PC is available in the desktop app.',
          )}
        </p>
      )}
      {state.loading && (
        <p className="installer-help" role="status">
          {tr('Controllo degli assistenti sul PC…', 'Checking assistants on this PC…')}
        </p>
      )}
      {state.targets.map((target) => (
        <section key={target.id} className="installer-target" aria-label={target.name}>
          <div className="installer-target-heading">
            <strong>{target.name}</strong>
            <span className="installer-status">{status(target)}</span>
          </div>
          <p className="installer-help">{description(target)}</p>
          {(target.notes ?? []).map((note, i) => (
            <p key={i} className="installer-help">
              {backendText(note)}
            </p>
          ))}
          <Button
            variant="secondary"
            disabled={!desktop || busy || state.loading || !profile.trim() || !hasSelection(target)}
            onClick={() => void preview(target)}
            aria-label={`${tr('Anteprima installazione', 'Preview installation')} ${target.name}`}
          >
            {state.pendingTarget === target.id && state.pendingAction === 'preview'
              ? tr('Preparazione…', 'Preparing…')
              : tr('Controlla installazione', 'Review installation')}
          </Button>
          {selected?.plan.target === target.id && selected.selection === selection && (
            <div
              className="installer-review"
              role="region"
              aria-label={`${tr('Anteprima', 'Preview')} ${target.name}`}
            >
              <h4>{tr('File previsti', 'Planned files')}</h4>
              {selected.plan.files.length ? (
                <ul className="installer-files">
                  {selected.plan.files.map((file, i) => (
                    <li key={i}>
                      <span>{actionLabel(file.action)}</span>
                      <code>{file.path}</code>
                    </li>
                  ))}
                </ul>
              ) : (
                <p className="installer-help">
                  {tr('Nessun file da modificare.', 'No files to change.')}
                </p>
              )}
              {selected.plan.notes.map((note, i) => (
                <p key={i} className="installer-help">
                  {backendText(note)}
                </p>
              ))}
              {selected.plan.requires_import && (
                <p className="installer-help">
                  {tr(
                    'Dopo la creazione, completa l’importazione nell’assistente.',
                    'After creating the package, finish importing it in the assistant.',
                  )}
                </p>
              )}
              <div className="settings-actions">
                <Button variant="primary" disabled={busy} onClick={() => void apply()}>
                  {state.pendingAction === 'apply'
                    ? tr('Installazione…', 'Installing…')
                    : selected.plan.requires_import
                      ? prepareLabel(target)
                      : tr('Installa', 'Install')}
                </Button>
                <Button variant="secondary" disabled={busy} onClick={() => (state.selected = null)}>
                  {tr('Annulla', 'Cancel')}
                </Button>
              </div>
            </div>
          )}
          {result?.target === target.id && (
            <div className="installer-result" role="status">
              <strong>
                {result.requiresImport
                  ? tr('Pronto da importare', 'Ready to import')
                  : tr('Configurazione completata', 'Configuration complete')}
              </strong>
              {result.notes.map((note, i) => (
                <p key={i} className="installer-help">
                  {backendText(note)}
                </p>
              ))}
              {result.package_path && <p className="installer-path">{result.package_path}</p>}
              {result.changed_paths.length > 0 && (
                <details>
                  <summary>{tr('File salvati', 'Saved files')}</summary>
                  <ul className="installer-files">
                    {result.changed_paths.map((path, i) => (
                      <li key={i}>
                        <code>{path}</code>
                      </li>
                    ))}
                  </ul>
                </details>
              )}
              {result.backup_paths.length > 0 && (
                <details>
                  <summary>{tr('Copie di sicurezza', 'Backups')}</summary>
                  <ul className="installer-files">
                    {result.backup_paths.map((path, i) => (
                      <li key={i}>
                        <code>{path}</code>
                      </li>
                    ))}
                  </ul>
                </details>
              )}
              {(result.package_path || result.open_url) && (
                <Button variant="secondary" disabled={busy} onClick={() => void open(target.id)}>
                  {tr('Apri per completare', 'Open to finish')}
                </Button>
              )}
            </div>
          )}
        </section>
      ))}
      {!state.installMcp && !state.installSkill && (
        <p className="installer-help">
          {tr('Seleziona almeno una voce da installare.', 'Select at least one item to install.')}
        </p>
      )}
      {state.error && (
        <>
          <p className="installer-error" role="alert">
            {state.error}
          </p>
          <Button
            variant="secondary"
            disabled={busy || state.loading || !desktop}
            onClick={() => void refresh()}
          >
            {tr('Riprova il controllo', 'Check again')}
          </Button>
        </>
      )}
    </section>
  );
}
