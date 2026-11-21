// The undo queue at the end of a close (audit Q8), with no Tauri in it so `node --test` can drive it.
// Qamille quitting with a send still undoable must send it first, or say that it did not leave and
// when it will: never a quit that silently moves it to the next start. The queue lives in Rust
// (`UndoService`); `undo_send_now` ends every undo window now and waits for the sends to leave.
import type { CloseGuard, SessionEndStep } from './close-guards';

export type Call = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;

/** How long a close waits for the queue to leave: a send takes a second or two, a dead connection
 *  answers within this, and the question follows. */
export const QUIT_WAIT_MS = 10_000;
/** At session end Windows waits about 8 s for everything Qamille saves (`window_shell.rs`). */
export const SESSION_END_WAIT_MS = 5_000;

type SendResult = { remaining: number };

/** The close guard, registered `last`: it sends, so it runs only once the editors let the window
 *  close. A send that did not leave (no connection) brings the window back, if it was in the tray,
 *  and asks: «Resta aperto» vetoes the close, «Esci comunque» leaves it for the next start. */
export function createUndoQueueGuard(call: Call): CloseGuard {
  return async () => {
    const { remaining } = await call<SendResult>('undo_send_now', { waitMs: QUIT_WAIT_MS });
    if (remaining <= 0) return true;
    await call('restore_main_window');
    return await call<boolean>('undo_quit_anyway', { remaining });
  };
}

/** Its session-end step: only when the session really ends (`last`), since while Windows only asks
 *  another app can still cancel, and a send cannot be taken back. It only makes the queue due and
 *  returns: the steps after it (the recording stop) must not wait for the network inside the ~8 s
 *  Windows gives. `settleUndoQueue` waits once every step ran. */
export function createUndoQueueSessionEnd(call: Call): SessionEndStep {
  return async (last) => {
    if (!last) return true;
    await call<SendResult>('undo_send_now', { waitMs: 0 });
    return true;
  };
}

/** At session end, after every step: what is left of Windows' wait goes to the queue. Nobody can
 *  answer then: what did not leave in time goes at the next start. */
export async function settleUndoQueue(call: Call): Promise<void> {
  await call<SendResult>('undo_send_now', { waitMs: SESSION_END_WAIT_MS });
}
