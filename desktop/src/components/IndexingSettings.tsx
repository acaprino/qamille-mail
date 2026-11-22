import { listen } from '@tauri-apps/api/event';
import { useEffect, useRef } from 'react';
import Button from '../ui/Button';
import Switch from '../ui/Switch';
import { call, desktop, errorText } from '../lib/api';
import { parseAppEvent } from '../lib/events';
import { shortDate } from '../lib/regional';
import { backendText, tr } from '../lib/i18n';
import { useReactive } from '../lib/store';
import './IndexingSettings.css';

type Status = {
  account_id: number;
  account: string;
  indexed_present: number;
  total: number | null;
  percent: number | null;
  progress: {
    enabled: boolean;
    state: string;
    scanned: number;
    partial: number;
    notice: string | null;
    storage_limited: boolean;
    folders: { name: string; complete: boolean }[];
  };
  /** Messages of the account in the list (W3.3). */
  listed?: number;
  /** The history walk of a Gmail account; null for other providers. */
  history?: History | null;
  include_spam_trash?: boolean;
};
type History = {
  total: number;
  fetched: number;
  complete: boolean;
  last_error: string | null;
  stopped_for_space: number | null;
};
type ModelStatus = {
  installed: boolean;
  running: boolean;
  done_bytes: number;
  total_bytes: number;
  error: string | null;
};

export type IndexingSettingsProps = Record<string, never>;


export default function IndexingSettings() {
  const state = useReactive(() => ({
    rows: [] as Status[],
    error: '',
    busy: false,
    model: {
      installed: false,
      running: false,
      done_bytes: 0,
      total_bytes: 0,
      error: null,
    } as ModelStatus,
  }));
  const active = useRef(true);
  const generation = useRef(0);
  const refreshing = useRef(false);
  const again = useRef(false);
  const { model } = state;
  const labels: Record<string, string> = {
    counting: tr('Calcolo del totale', 'Counting messages'),
    needs_server: tr('Ricerca per significato: serve un server', 'Semantic search: a server is needed'),
    waiting_model: tr('In attesa del modello di ricerca', 'Waiting for the search model'),
    queued: tr('In coda', 'Queued'),
    scanning: tr('Indicizzazione in corso', 'Indexing'),
    watching: tr('In attesa di nuove email', 'Watching for new mail'),
    partial: tr('Copertura parziale', 'Partial coverage'),
    paused: tr('In pausa', 'Paused'),
    blocked: tr('Spazio account insufficiente', 'Account storage full'),
    error: tr('In attesa di riprovare', 'Waiting to retry'),
    unlinked: tr('Dispositivo non più collegato al server', 'Device no longer connected to the server'),
    model_mismatch: tr("Modello del server diverso dall'indice", 'Server model differs from the index'),
  };

  async function refresh() {
    if (!active.current) return;
    // An event that lands mid-refresh must not be lost: run once more when this pass ends.
    if (refreshing.current) {
      again.current = true;
      return;
    }
    refreshing.current = true;
    again.current = false;
    const request = ++generation.current;
    try {
      const [status, ready] = await Promise.all([
        call<Status[]>('indexing_status'),
        call<ModelStatus>('search_model_status'),
      ]);
      if (active.current && request === generation.current) {
        state.rows = status;
        state.model = ready;
        state.error = '';
      }
    } catch (e) {
      if (active.current && request === generation.current) {
        state.rows = [];
        state.error = errorText(e);
      }
    } finally {
      refreshing.current = false;
      if (again.current) void refresh();
    }
  }
  async function configure(accountId: number, enabled: boolean, retryPartial = false) {
    state.busy = true;
    ++generation.current;
    try {
      await call('configure_indexing', { accountId, enabled, retryPartial });
      await refresh();
    } catch (e) {
      state.error = errorText(e);
    } finally {
      state.busy = false;
    }
  }
  /** D4: the server serves another model; every vector is made again with it. */
  async function reindex() {
    state.busy = true;
    ++generation.current;
    try {
      await call('search_reindex');
      await refresh();
    } catch (e) {
      state.error = errorText(e);
    } finally {
      state.busy = false;
    }
  }
  async function includeSpamTrash(accountId: number, enabled: boolean) {
    state.busy = true;
    ++generation.current;
    try {
      await call('set_include_spam_trash', { accountId, enabled });
      await refresh();
    } catch (e) {
      state.error = errorText(e);
    } finally {
      state.busy = false;
    }
  }
  // Where the history of a Gmail account stands, in words: no percentages and no motion (INV-7).
  function historyState(history: History) {
    if (history.stopped_for_space !== null)
      return tr(
        `Fermo per spazio: in elenco la posta dal ${shortDate(history.stopped_for_space)}. Aumenta lo spazio dell'account per continuare.`,
        `Stopped for space: the list holds mail since ${shortDate(history.stopped_for_space)}. Increase the account's storage to continue.`,
      );
    if (history.complete) return tr('Tutta la posta è in elenco', 'All mail is in the list');
    if (history.last_error)
      return `${tr('Recupero in pausa, riprova da solo', 'Paused, it retries on its own')}: ${backendText(history.last_error)}`;
    return tr('Recupero della posta precedente in corso', 'Bringing in older mail');
  }
  useEffect(() => {
    active.current = true;
    let unlisten: (() => void) | undefined;
    let unlistenApp: (() => void) | undefined;
    // Typed progress events (ADR-0007 decision 7): the model download announces
    // `model-progress` while it runs and the indexer `index-progress`; the table keeps
    // reading its detail from the status commands, but no fast poll is needed any more.
    const onAppEvent = (payload: unknown) => {
      const event = parseAppEvent(payload);
      if (
        !state.busy &&
        (event?.kind === 'model-progress' ||
          event?.kind === 'index-progress' ||
          (event?.kind === 'history-progress' && event.complete))
      )
        void refresh();
    };
    const domAppEvent = (event: Event) => onAppEvent((event as CustomEvent<unknown>).detail);
    if (desktop) {
      // The backend announces most indexing steps and the end of the model download. Subscribe
      // first and read afterwards, so an event between the read and the subscription is not lost.
      listen('indexing-changed', () => {
        if (!state.busy) void refresh();
      })
        .then((stop) => {
          if (active.current) unlisten = stop;
          else stop();
          void refresh();
        })
        .catch(() => void refresh());
      listen('app-event', (event) => onAppEvent(event.payload))
        .then((stop) => {
          if (active.current) unlistenApp = stop;
          else stop();
        })
        .catch(() => {});
    } else {
      // The browser preview sends the same typed events as DOM events.
      window.addEventListener('app-event', domAppEvent);
      void refresh();
    }
    // Polling is the exception (INV-005, INV-009): a 30 s safety poll covers what no event
    // announces (the indexer's retry and error transitions; browser preview, where no backend
    // runs at all). The brisk model-download poll is gone: `model-progress` events, emitted
    // while `install()` runs, announce every visible movement of that download (Task 2.4).
    const timer = setInterval(() => {
      if (!state.busy) void refresh();
    }, 30_000);
    return () => {
      active.current = false;
      ++generation.current;
      unlisten?.();
      unlistenApp?.();
      window.removeEventListener('app-event', domAppEvent);
      clearInterval(timer);
    };
  }, []);

  return (
    <>
      <h2>{tr('Indice completo della posta', 'Full mail index')}</h2>
      <p className="muted">
        {tr(
          "Prima scansione di tutte le cartelle accessibili, poi nuove email aggiunte automaticamente in background. Puoi chiudere l'app: l'avanzamento viene salvato.",
          'Scans all accessible folders, then adds new mail automatically in the background. Progress is saved when you close the app.',
        )}
      </p>
      <p className="field-help">
        {tr(
          'Include oggetto, metadati, testo, documenti leggibili e immagini con EmbeddingGemma 2. Originali e allegati restano online, salvo la tua scelta di conservarli offline.',
          'Includes subjects, metadata, text, readable documents and images with EmbeddingGemma 2. Originals and attachments stay online unless you choose to keep them offline.',
        )}
      </p>
      {state.rows.some((row) => row.progress.state !== 'needs_server') && (
        <p className="field-help" role="status">
          {model.installed
            ? tr('Il server ha il modello di ricerca', 'The server has the search model')
            : tr(
                'Il server non ha il modello di ricerca: lo installa chi lo amministra',
                'The server does not have the search model: its administrator installs it',
              )}
        </p>
      )}
      {state.rows.map((row) => (
        <section
          key={row.account_id}
          className="index-account"
          aria-label={`${tr('Indice', 'Index')} ${row.account}`}
        >
          <strong>{row.account}</strong>
          {row.history && (
            <>
              <p className="field-help" data-testid="history-coverage">
                {tr('In elenco', 'Listed')}: {row.listed ?? 0}
                {row.history.total ? ` ${tr('di', 'of')} ${row.history.total}` : ''} ·{' '}
                {tr('Indicizzati per contenuto', 'Indexed by content')}: {row.indexed_present}
              </p>
              <p className="field-help">{historyState(row.history)}</p>
              <Switch
                checked={!!row.include_spam_trash}
                disabled={state.busy}
                onChange={(enabled) => void includeSpamTrash(row.account_id, enabled)}
              >
                {tr('Includi Spam e Cestino', 'Include Spam and Trash')}
              </Switch>
            </>
          )}
          <p role="status">{labels[row.progress.state] ?? row.progress.state}</p>
          {row.percent !== null && row.percent !== undefined ? (
            <>
              <div className="index-progress">
                <strong>{row.percent}%</strong>
                <span>
                  {row.indexed_present} / {row.total} {tr('email complete', 'complete messages')}
                </span>
              </div>
              <progress
                max="100"
                value={row.percent}
                aria-label={tr('Percentuale di indicizzazione', 'Indexing percentage')}
              />
            </>
          ) : (
            <>
              <p className="field-help">
                {tr('Calcolo del totale univoco delle email…', 'Counting unique messages…')}
              </p>
              <progress max="100" aria-label={tr('Censimento email', 'Counting messages')} />
            </>
          )}
          <p className="field-help">
            {row.indexed_present} {tr('email indicizzate completamente', 'fully indexed messages')}{' '}
            · {row.progress.partial}{' '}
            {tr('email con contenuti incompleti', 'messages with incomplete content')}
          </p>
          {row.progress.folders.length > 0 && (
            <p className="field-help">
              {row.progress.folders.filter((folder) => folder.complete).length} /{' '}
              {row.progress.folders.length}{' '}
              {tr('cartelle controllate nel passaggio corrente', 'folders checked in this pass')}
            </p>
          )}
          {row.progress.storage_limited && (
            <p className="index-notice">
              {tr(
                'Parte dell’indice è stata rimossa per rispettare lo spazio dell’account. Aumenta il limite e riprova per recuperare lo storico.',
                'Part of the index was removed to respect account storage. Increase the limit and retry to recover historical mail.',
              )}
            </p>
          )}
          {row.progress.notice && (
            <p className="index-notice">{backendText(row.progress.notice)}</p>
          )}
          <div className="button-row">
            <Button
              variant="secondary"
              disabled={state.busy}
              onClick={() => void configure(row.account_id, !row.progress.enabled)}
            >
              {row.progress.enabled ? tr('Pausa', 'Pause') : tr('Riprendi', 'Resume')}
            </Button>
            {(row.progress.partial > 0 ||
              row.progress.storage_limited ||
              row.progress.state === 'error' ||
              row.progress.state === 'blocked') && (
              <Button
                variant="secondary"
                disabled={state.busy}
                onClick={() => void configure(row.account_id, true, true)}
              >
                {tr('Riprova contenuti incompleti', 'Retry incomplete content')}
              </Button>
            )}
            {row.progress.state === 'model_mismatch' && (
              <Button variant="secondary" disabled={state.busy} onClick={() => void reindex()}>
                {tr('Reindicizza', 'Reindex')}
              </Button>
            )}
          </div>
        </section>
      ))}
      {state.error && (
        <p role="alert" className="index-notice">
          {state.error}
        </p>
      )}
      <p className="field-help">
        {tr(
          'File cifrati, formati non leggibili e contenuti oltre il budget vengono segnalati. Una copertura parziale non viene presentata come completa.',
          'Encrypted files, unreadable formats and content above the memory budget are reported. Partial coverage is never shown as complete.',
        )}
      </p>
    </>
  );
}
