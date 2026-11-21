// A click on a Windows notification of Qamille (docs/aspetto-nativo.md, rule 28). Rust shows the
// notifications (src-tauri/src/notifications.rs); on a click it brings the window back
// (`background::restore`) and sends `notification-open` with what the notification was about, and the
// page opens it. Nothing from Tauri is imported at the top, so `node --test` drives the routing
// (tests/notification-open.mjs).

/** The payload of `notification-open`. */
export type NotificationOpen = {
  kind: 'mail' | 'scheduled' | 'sign-in';
  /** The message a new-mail notification opens (the newest, for a group). */
  email_id?: number;
  /** The mail account; absent on the Qamille server's sign-in. */
  account_id?: number;
};

export type NotificationOpenActions = {
  /** New mail: the message, in the Inbox. */
  openMail(id: number): void;
  /** A scheduled send that failed: the Drafts. */
  openDrafts(): void;
  /** An expired sign-in of a mail account: Settings → Accounts. */
  openAccounts(): void;
  /** The Qamille server's sign-in: Settings → Compute. Without it, `openAccounts`. */
  openServer?(): void;
};

const isId = (value: unknown): value is number => typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;

/** The payload if it is one Rust sends, with only the fields the page uses; otherwise `null`. */
export function parseNotificationOpen(payload: unknown): NotificationOpen | null {
  if (!payload || typeof payload !== 'object') return null;
  const { kind, email_id, account_id } = payload as Record<string, unknown>;
  if (account_id !== undefined && account_id !== null && !isId(account_id)) return null;
  const account = isId(account_id) ? { account_id } : {};
  if (kind === 'mail') return isId(email_id) ? { kind: 'mail', email_id, ...account } : null;
  if (kind === 'scheduled') return { kind: 'scheduled' };
  if (kind === 'sign-in') return { kind: 'sign-in', ...account };
  return null;
}

/** Opens what the notification was about; `false` for a payload it does not know. */
export function routeNotificationOpen(payload: unknown, actions: NotificationOpenActions): boolean {
  const open = parseNotificationOpen(payload);
  if (!open) return false;
  if (open.kind === 'mail' && open.email_id !== undefined) actions.openMail(open.email_id);
  else if (open.kind === 'scheduled') actions.openDrafts();
  else if (open.account_id === undefined && actions.openServer) actions.openServer();
  else actions.openAccounts();
  return true;
}

/** Listens to `notification-open` in the desktop app (nothing in the browser preview). */
export function installNotificationOpen(actions: NotificationOpenActions): () => void {
  let disposed = false;
  let unlisten: (() => void) | undefined;
  void (async () => {
    const { desktop } = await import('./api');
    if (!desktop || disposed) return;
    const { listen } = await import('@tauri-apps/api/event');
    const stop = await listen<unknown>('notification-open', (event) => void routeNotificationOpen(event.payload, actions));
    if (disposed) stop();
    else unlisten = stop;
  })().catch(() => {
    // Without the listener a click still brings the window back.
  });
  return () => {
    disposed = true;
    unlisten?.();
  };
}
