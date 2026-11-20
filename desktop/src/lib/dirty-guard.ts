// The close guard of an editor with unsaved typing (the calendar event sheet, the contact form),
// with no Tauri and no React in it so `node --test` can drive it. The hook that ties it to a
// component's life is use-dirty-close-guard.ts.
import type { CloseGuard, CloseGuardOptions } from './close-guards';

/** `registerCloseGuard` (window-close.ts), or any registry with the same shape. */
type Register = (id: string, guard: CloseGuard, options: CloseGuardOptions) => () => void;

/** Returns what an editor calls with its state: while `dirty` the guard is registered, and the
 *  returned function removes it (a no-op when the editor is clean).
 *  - Closing the window: `onVeto` tells the person what is open, and the close is vetoed. Nothing
 *    is saved behind their back.
 *  - Windows ends the session or the window reloads: nobody can answer, so `saveNow` runs and its
 *    answer is the step's. `false` (an invalid form) or a throw is "not saved". */
export function createDirtyGuard(register: Register) {
  return (
    id: string,
    dirty: boolean,
    onVeto: () => void,
    saveNow: () => Promise<boolean>,
  ): (() => void) => {
    if (!dirty) return () => {};
    return register(
      id,
      async () => {
        onVeto();
        return false;
      },
      { sessionEnd: () => saveNow() },
    );
  };
}
