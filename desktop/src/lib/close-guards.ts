// The guards a window close goes through, with no Tauri in them so `node --test` can drive them.
// Each editor can veto the close until its data is saved. A guard registered with `last: true`
// does its irreversible work (stop the recording, cancel the jobs) and so must run only when no
// other guard can still veto: a vetoed close must leave everything as it was.
export type CloseGuard = () => Promise<boolean>;
// When Windows ends the session (sign-out, shutdown, restart, an installer's Restart Manager) there
// is no one to answer a dialog and no veto (audit F2): each editor saves what it can, without
// asking. `last: false` when Windows asks whether it may end the session, which another app can
// still cancel: save, change nothing irreversible. `last: true` when it does: also stop for good.
export type SessionEndStep = (last: boolean) => Promise<unknown>;
export interface CloseGuardOptions {
  last?: boolean;
  sessionEnd?: SessionEndStep;
}
export interface CloseGuards {
  register(id: string, guard: CloseGuard, options?: CloseGuardOptions): () => void;
  /** Runs the guards, the ones registered `last` after all the others; stops at the first veto. True: the window may close. */
  run(): Promise<boolean>;
  /** Runs every session-end step in the same order, each to the end: a failing one never stops the others. False: one failed. */
  endSession(last: boolean): Promise<boolean>;
}

export function createCloseGuards(): CloseGuards {
  const guards = new Map<string, { guard: CloseGuard; last: boolean; sessionEnd?: SessionEndStep }>();
  const ordered = () => {
    const entries = [...guards.values()];
    return [...entries.filter((e) => !e.last), ...entries.filter((e) => e.last)];
  };
  return {
    register(id, guard, options = {}) {
      const entry = { guard, last: options.last === true, sessionEnd: options.sessionEnd };
      guards.set(id, entry);
      return () => {
        if (guards.get(id) === entry) guards.delete(id);
      };
    },
    async run() {
      for (const { guard } of ordered()) if (!(await guard())) return false;
      return true;
    },
    async endSession(last) {
      let saved = true;
      for (const { sessionEnd } of ordered()) {
        try {
          await sessionEnd?.(last);
        } catch {
          // The next step still saves its part; the caller learns that something was not saved.
          saved = false;
        }
      }
      return saved;
    },
  };
}
