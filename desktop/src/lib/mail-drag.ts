/** Dragging messages from the list onto a composer, which attaches each one as `.eml`. */
export const MAIL_DRAG_TYPE = 'application/x-qamille-mail';

export function startMailDrag(event: DragEvent, ids: number[], label: string) {
  const transfer = event.dataTransfer;
  if (!transfer || !ids.length) return;
  transfer.effectAllowed = 'copy';
  // Only the private type: editors and other drop targets ignore it.
  transfer.setData(MAIL_DRAG_TYPE, JSON.stringify(ids));
  if (ids.length > 1 && typeof document !== 'undefined') {
    const badge = document.createElement('div');
    badge.className = 'mail-drag-badge';
    badge.textContent = label;
    document.body.append(badge);
    transfer.setDragImage(badge, 12, 12);
    setTimeout(() => badge.remove(), 0);
  }
}

export function carriesMail(event: DragEvent): boolean {
  return !!event.dataTransfer && [...event.dataTransfer.types].includes(MAIL_DRAG_TYPE);
}

export function droppedMailIds(event: DragEvent): number[] {
  const raw = event.dataTransfer?.getData(MAIL_DRAG_TYPE);
  if (!raw) return [];
  try {
    const parsed: unknown = JSON.parse(raw);
    return Array.isArray(parsed)
      ? [...new Set(parsed.filter((id): id is number => Number.isSafeInteger(id) && id >= 0))]
      : [];
  } catch {
    return [];
  }
}
