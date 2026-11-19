import { tr } from './i18n';
import type { UndoAction } from './types';
import { mailActionLabels } from './undo-labels';

export function undoLabel(action: UndoAction): string {
  const op = action.operation;
  if (op.kind === 'send_draft') return tr('Invio del messaggio', 'Sending message');
  if (op.kind === 'discard_draft') return tr('Bozza eliminata', 'Draft discarded');
  if (op.kind === 'schedule_draft') return tr('Messaggio programmato', 'Message scheduled');
  const label = mailActionLabels[op.action];
  return label ? tr(label[0], label[1]) : tr('Messaggio aggiornato', 'Message updated');
}
