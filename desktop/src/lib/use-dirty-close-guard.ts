import { useEffect, useRef } from 'react';
import { createDirtyGuard } from './dirty-guard';
import { registerCloseGuard } from './window-close';

const dirtyGuard = createDirtyGuard(registerCloseGuard);

/** Keeps the window close from losing what an editor holds: registered only while `dirty`, removed
 *  when it turns clean or the component unmounts. `onVeto` and `saveNow` are read when the guard
 *  runs, so they may be new functions on every render. See dirty-guard.ts for what each does. */
export function useDirtyCloseGuard(
  id: string,
  dirty: boolean,
  onVeto: () => void,
  saveNow: () => Promise<boolean>,
) {
  const latest = useRef({ onVeto, saveNow });
  latest.current = { onVeto, saveNow };
  useEffect(
    () =>
      dirtyGuard(
        id,
        dirty,
        () => latest.current.onVeto(),
        () => latest.current.saveNow(),
      ),
    [id, dirty],
  );
}
