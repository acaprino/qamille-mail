// The undo queue at the end of a close (audit Q8), with no Tauri in it so `node --test` can drive it.
// Qamille quitting with a send still undoable must send it first, or say what quitting does to what
// did not leave: never a quit that silently moves it to the next start. A send or an action still
// queued goes at the next start; one already on its way dies with the process, and the next start
// marks it failed without retrying it (check Sent). The queue lives in Rust (`UndoService`);
// `undo_send_now` ends every undo window now and waits for the queue to leave.
import type { CloseGuard, SessionEndStep } from './close-guards';

export type Call = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;

/** How long a close waits for the queue to leave: a send takes a second or two, a dead connection
 *  answers within this, and the question follows. */
export const QUIT_WAIT_MS = 10_000;
/** At session end Windows waits about 8 s for everything Qamille saves (`window_shell.rs`). */
export const SESSION_END_WAIT_MS = 5_000;

/** `UndoSendResult` in `commands/undo.rs`: what did not leave, by what quitting does to it. */
type SendResult = {
  remaining: number;
  /** Sends still queued: the next start sends them. */
  sends: number;
  /** Sends on their way: quitting may stop them, and nothing sends them again. */
  sending: number;
  /** Other actions (archive, trash, move…) still queued: the next start carries them out. */
  actions: number;
  /** Other actions being confirmed: quitting may interrupt them. */
  confirming: number;
  /** Sends that failed while the close waited (no connection): they did not leave, and are in Drafts. */
  failed?: number;
};

/** The close guard, registered `last`: it sends, so it runs only once the editors let the window
 *  close. What did not leave brings the window back, if it was in the tray, and asks, saying what
 *  quitting does to each kind (`unsent_text`): «Resta aperto» vetoes the close, «Esci comunque»
 *  leaves the queued ones for the next start and may lose the ones on their way. A send that failed
 *  during the wait (no connection) asks too: it did not leave, and it is in Drafts. */
export function createUndoQueueGuard(call: Call): CloseGuard {
  return async () => {
    const { remaining, sends, sending, actions, confirming, failed = 0 } = await call<SendResult>('undo_send_now', { waitMs: QUIT_WAIT_MS });
    if (remaining <= 0) return true;
    // In view before the question, and still in view after «Resta aperto»: the person can open Drafts.
    await call('restore_main_window');
    return await call<boolean>('undo_quit_anyway', { left: { sends, sending, actions, confirming, failed } });
  };
}

/** Its session-end step: only when the session really ends (`last`), since while Windows only asks
 *  another app can still cancel, and a send cannot be taken back. It only makes the queue due and
 *  returns: the steps after it (the recording stop) must not wait for the network inside the ~8 s
 *  Windows gives. `settleUndoQueue` waits once every step ran. */
export function createUndoQueueSessionEnd(call: Call): SessionEndStep {
  return async (last) => {
    if (!last) return true;
    await call<SendResult>('undo_send_now', { waitMs: 0, sessionEnd: true });
    return true;
  };
}

/** At session end, after every step: what is left of Windows' wait goes to the queue. Nobody can
 *  answer then: what is still queued goes at the next start, and what is still on its way when
 *  Windows ends the process is not sent again (the next start marks it failed: check Sent). */
export async function settleUndoQueue(call: Call): Promise<void> {
  // `sessionEnd`: what did not leave becomes a line in native.log; nothing asks.
  await call<SendResult>('undo_send_now', { waitMs: SESSION_END_WAIT_MS, sessionEnd: true });
}
