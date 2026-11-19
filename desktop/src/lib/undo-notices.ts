import type { UndoAction } from './types';

export function canUndo(action: UndoAction, now: number): boolean {
  return action.status === 'pending' && action.expires_at_ms > now;
}

/**
 * A confirmation that never reached the provider: its undo window is closed, it
 * was attempted at least once, and the queue is waiting for its next try. It is
 * not a failure, so the local state stays applied and the row shows the reason,
 * the countdown and a way to give up (ADR-0007 v3, decision 8 / INV-010).
 */
export function isRetryNotice(action: UndoAction, now: number): boolean {
  return action.status !== 'failed' && (action.attempts ?? 0) > 0 && !canUndo(action, now);
}

/** Seconds left before the next attempt, or null when none is scheduled. */
export function retrySeconds(action: UndoAction, now: number): number | null {
  const retryAt = action.retry_at_ms ?? null;
  if (retryAt === null) return null;
  return Math.max(0, Math.ceil((retryAt - now) / 1000));
}

/** Whether the notice offers "Rinuncia": a real failure, or a retry to abandon. */
export function canGiveUp(action: UndoAction, now: number): boolean {
  return action.status === 'failed' || isRetryNotice(action, now);
}

/**
 * Whether anything on screen still changes with time: an action with an open undo
 * window has a countdown, and so has a retry row waiting for its next attempt.
 * Both end by moving the action off the stack or into a static state. Failed
 * notices, sends waiting for delivery and a retry whose deadline has passed (its
 * attempt is in flight) are visible but static, and confirming actions are only a
 * counter.
 */
export function hasOpenUndoWindow(actions: UndoAction[], now: number): boolean {
  return actions.some(
    (action) =>
      canUndo(action, now) ||
      (isRetryNotice(action, now) && (retrySeconds(action, now) ?? 0) > 0),
  );
}

/**
 * A notice lives as long as its undo window. Once the window closes, the
 * action is confirmed in the background, which can take seconds while a sync
 * holds the mailbox; those actions are only counted (see below). A failure
 * comes back as its own notice, and so does a confirmation that is being
 * retried, because the user must see why and be able to give up. A send keeps
 * its notice until delivery, because nothing else tells the user it went out.
 */
export function visibleUndoActions(actions: UndoAction[], now: number): UndoAction[] {
  return actions.filter(
    (action) =>
      action.status === 'failed' ||
      action.operation.kind === 'send_draft' ||
      canUndo(action, now) ||
      isRetryNotice(action, now),
  );
}

/** Closed actions still being confirmed: closing the app now would interrupt them. */
export function confirmingUndoActions(actions: UndoAction[], now: number): UndoAction[] {
  const visible = new Set(visibleUndoActions(actions, now));
  return actions.filter((action) => action.status !== 'failed' && !visible.has(action));
}
