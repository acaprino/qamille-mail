import { call, desktop, errorText } from './api';
import { createCloseGuards, type CloseGuard, type CloseGuardOptions } from './close-guards';

// One close listener owns destruction. Each editor can veto until its data is saved.
const guards = createCloseGuards();
/** `last`: the guard acts for good (stops a recording), so it runs after every guard that can veto.
 *  `sessionEnd`: what it saves, without asking, when Windows ends the session (no guard runs then). */
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
  const { getCurrentWindow } = await import('@tauri-apps/api/window');
  const current = getCurrentWindow();
  let closing = false;
  const stopSession = await installSessionEndHandler();
  const stopClose = await current.onCloseRequested(async (event) => {
    event.preventDefault();
    // The page is alive: without this answer Rust offers to close Qamille itself (recovery.rs).
    void call('close_seen').catch(() => {});
    if (closing) return;
    closing = true;
    try {
      if (!(await guards.run())) return;
      await current.destroy();
    } catch (error) { notify(errorText(error), true); }
    finally {
      closing = false;
      void call('close_settled').catch(() => {});
    }
  });
  return () => {
    stopSession();
    stopClose();
  };
}
