import { call, desktop, errorText } from './api';
import { createCloseGuards, type CloseGuard, type CloseGuardOptions, type SessionEndStep } from './close-guards';
import { createUndoQueueGuard, createUndoQueueSessionEnd } from './undo-close';

// One close listener owns destruction. Each editor can veto until its data is saved.
const guards = createCloseGuards();
// The undo queue (lib/undo-close.ts) is registered when this module loads, before any editor, and
// installed by installWindowCloseHandler. Of the guards that run last it comes first: a «Resta aperto»
// there must come before the meetings guard stops a recording, while a send that leaves early, before
// a later veto, only loses the last seconds of its undo window.
let undoQueue: { guard: CloseGuard; sessionEnd: SessionEndStep } | null = null;
guards.register('undo-queue', async () => (undoQueue ? undoQueue.guard() : true), {
  last: true,
  sessionEnd: async (last) => (undoQueue ? undoQueue.sessionEnd(last) : true),
});
/** `last`: the guard acts for good (stops a recording), so it runs after every guard that can veto.
 *  `sessionEnd`: what it saves, without asking, when Windows ends the session (no guard runs then).
 *  A `sessionEnd` step that answers `false`, or throws, means "not saved": the reload then asks. */
export function registerCloseGuard(id: string, guard: CloseGuard, options?: CloseGuardOptions) {
  return guards.register(id, guard, options);
}
/** Windows ends the session: `session-ending` from src-tauri/src/window_shell.rs. The steps run one
 *  phase after the other; after the last one the backend lets Windows go on (at most ~8 s). */
async function installSessionEndHandler(): Promise<() => void> {
  const { listen } = await import('@tauri-apps/api/event');
  let saving: Promise<unknown> = Promise.resolve();
  const stopSession = await listen<{ last: boolean }>('session-ending', (event) => {
    const last = event.payload.last === true;
    saving = saving.then(() => guards.endSession(last));
    if (last) void saving.finally(() => call('session_end_ready').catch(() => {}));
  });
  // "Ricarica la finestra" in the system menu (src-tauri/src/recovery.rs): save what the sign-out
  // would save, without asking and without stopping a recording, then tell Rust whether everything
  // was saved: only then it reloads by itself, otherwise it asks.
  const stopReload = await listen<number>('reload-requested', (event) => {
    const saved = saving.then(() => guards.endSession(false));
    saving = saved;
    void saved.then((ok) => call('reload_ready', { token: event.payload, saved: ok })).catch(() => {});
  });
  return () => {
    stopSession();
    stopReload();
  };
}
export async function installWindowCloseHandler(notify: (message: string, error?: boolean) => void) {
  if (!desktop) return () => {};
  undoQueue = { guard: createUndoQueueGuard(call), sessionEnd: createUndoQueueSessionEnd(call) };
  const { getCurrentWindow } = await import('@tauri-apps/api/window');
  const current = getCurrentWindow();
  let closing = false;
  const stopSession = await installSessionEndHandler();
  const stopClose = await current.onCloseRequested(async (event) => {
    event.preventDefault();
    // The page is alive: without this answer Rust offers to close Qamille itself (recovery.rs).
    // `false`: the close button only put the window in the tray (background.rs); nothing closes.
    const closes = await call<boolean>('close_seen').catch(() => true);
    if (!closes || closing) return;
    closing = true;
    let closed = false;
    try {
      if (!(await guards.run())) return;
      await current.destroy();
      closed = true;
    } catch (error) { notify(errorText(error), true); }
    finally {
      closing = false;
      // A guard kept the window, or failed, during «Esci» from the tray: the window comes back, so
      // its question or the error is seen. On a window already in view this only focuses it.
      if (!closed) void call('restore_main_window').catch(() => {});
      void call('close_settled').catch(() => {});
    }
  });
  return () => {
    stopSession();
    stopClose();
    undoQueue = null;
  };
}
