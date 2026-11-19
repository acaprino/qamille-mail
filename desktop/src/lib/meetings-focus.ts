// Keeps the keyboard where the user was when an action takes its own button away. "Termina",
// "Trascrivi", "Recupera audio", "Annulla" and "Elimina" change what the detail shows: the button
// that was pressed is gone, and the browser drops the focus to the page. When that happens the
// focus goes to the detail pane (a container that takes it by script only), so the next Tab starts
// from there and a screen reader says where it is. A focus that is somewhere else is left alone.
import { tick } from './tick';

/** Moves the focus to the detail pane if, once the screen has settled, it is on the page itself. */
export async function keepFocusInDetail(): Promise<void> {
  await tick();
  const active = document.activeElement;
  if (active && active !== document.body) return;
  document.querySelector<HTMLElement>('.meetings-page .detail-pane')?.focus({ preventScroll: true });
}
