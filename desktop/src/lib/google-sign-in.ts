import { call } from './api';

// Navigation can unmount Settings while the browser is still signing in.
// Reuse that request when the user returns instead of opening another browser tab.
let pending: Promise<void> | null = null;

// `loginHint` is the address typed so far, so Google's page opens on that account.
export function connectGoogle(loginHint = ''): Promise<void> {
  if (!pending) {
    pending = call<void>('connect_google', { loginHint: loginHint.trim() }).finally(() => {
      pending = null;
    });
  }
  return pending;
}

// Ends a sign-in that is still waiting for the browser; the pending request then fails with "Sign-in stopped."
export function cancelGoogle(): Promise<void> {
  return call<void>('cancel_google_sign_in');
}
