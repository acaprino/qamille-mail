import { call, errorText } from './api';

/** A link that leaves the app: it opens in the system browser (the `open_link` command), never in a
 *  window of the app, which a click on a plain `<a target="_blank">` would ask for and the desktop
 *  webview would deny, leaving a link that does nothing. The click's own navigation is always
 *  cancelled; a failure goes to `onError` as text. The preview opens a tab (`previewCommand`). */
export async function openExternal(
  event: { preventDefault(): void; currentTarget: { href: string } },
  onError?: (message: string) => void,
): Promise<void> {
  event.preventDefault();
  // Read before the first await: React clears `currentTarget` once the handler returns.
  const url = event.currentTarget.href;
  try {
    await call('open_link', { url });
  } catch (error) {
    onError?.(errorText(error));
  }
}
