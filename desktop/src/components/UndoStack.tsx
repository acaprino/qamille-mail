import { AnimatePresence, motion, useIsPresent, useReducedMotion } from 'motion/react';
import { useEffect, useState } from 'react';
import Button from '../ds/primitives/Button';
import Count from '../ds/primitives/Count';
import Icon from '../ds/primitives/Icon';
import { cn } from '../lib/utils';
import { undoLabel } from '../lib/undo';
import {
  canGiveUp,
  canUndo,
  hasOpenUndoWindow,
  isRetryNotice,
  retrySeconds,
  visibleUndoActions,
} from '../lib/undo-notices';
import { backendText, tr } from '../lib/i18n';
import type { UndoAction } from '../lib/types';

export type UndoStackProps = {
  actions: UndoAction[];
  undo: (action: UndoAction) => Promise<void>;
  dismiss: (id: number) => Promise<void>;
};

const spring = { type: 'spring', stiffness: 400, damping: 32 } as const;

export default function UndoStack({ actions, undo, dismiss }: UndoStackProps) {
  const [now, setNow] = useState(() => Date.now());
  const [working, setWorking] = useState<number[]>([]);
  const visible = visibleUndoActions(actions, now);
  // The tick exists only while a countdown is on screen (INV-005): it starts with the first open
  // undo window or retry row and stops when the last countdown ends, even if actions are still
  // being confirmed. Closed actions being confirmed are counted by the single activity
  // indicator (ADR-0007 decision 7), no longer by a row of this stack.
  const counting = hasOpenUndoWindow(actions, now);
  useEffect(() => {
    if (!counting) return;
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 50);
    return () => clearInterval(timer);
  }, [counting]);

  async function busyWhile(id: number, work: () => Promise<void>) {
    setWorking((list) => [...list, id]);
    try {
      await work();
    } finally {
      setWorking((list) => list.filter((other) => other !== id));
    }
  }
  const cancel = (action: UndoAction) => busyWhile(action.id, () => undo(action));
  /** "Rinuncia": the only case where a queued confirmation returns the local state. */
  const giveUp = (action: UndoAction) => busyWhile(action.id, () => dismiss(action.id));

  if (!visible.length) return null;
  return (
    <section
      className="undo-stack"
      aria-label={tr('Azioni recenti sulle email', 'Recent email actions')}
    >
      <AnimatePresence initial={false}>
        {[...visible].reverse().map((action) => (
          <Notice
            key={action.id}
            action={action}
            now={now}
            working={working.includes(action.id)}
            onUndo={() => void cancel(action)}
            onGiveUp={() => void giveUp(action)}
            onDismiss={() => void dismiss(action.id)}
          />
        ))}
      </AnimatePresence>
    </section>
  );
}

function Notice({
  action,
  now,
  working,
  onUndo,
  onGiveUp,
  onDismiss,
}: {
  action: UndoAction;
  now: number;
  working: boolean;
  onUndo: () => void;
  onGiveUp: () => void;
  onDismiss: () => void;
}) {
  const still = useReducedMotion();
  // A notice on its way out is still drawn for a moment: it must not take a click or the focus.
  const present = useIsPresent();
  const remaining = Math.max(0, action.expires_at_ms - now);
  const available = canUndo(action, now);
  const retrying = isRetryNotice(action, now);
  const seconds = retrySeconds(action, now);
  const subject = action.subject || tr('Messaggio senza oggetto', 'Untitled message');
  return (
    <motion.div
      className={cn('undo-notice', action.status === 'failed' && 'failed')}
      data-action-id={action.id}
      inert={!present}
      layout={still ? false : 'position'}
      initial={still ? false : { opacity: 0, y: 16, scale: 0.97 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      exit={still ? { opacity: 0, transition: { duration: 0 } } : { opacity: 0, scale: 0.96 }}
      transition={spring}
    >
      <span className="undo-icon">
        <Icon
          name={
            action.status === 'failed'
              ? 'info'
              : retrying
                ? 'refresh'
                : action.operation.kind === 'send_draft'
                  ? 'send'
                  : 'check'
          }
          size={18}
        />
      </span>
      <div className="undo-copy">
        <strong role="status">
          {action.status === 'failed'
            ? tr(
                `Impossibile confermare: ${undoLabel(action)}`,
                `Could not confirm: ${undoLabel(action)}`,
              )
            : undoLabel(action)}
        </strong>
        <span className="undo-subject" title={action.subject}>
          {subject}
        </span>
        {retrying ? (
          // One line: why the confirmation is waiting, and when it tries again.
          <p role="status">
            {action.error ? `${backendText(action.error)} — ` : null}
            {seconds === null || seconds === 0
              ? tr('nuovo tentativo in corso…', 'retrying now…')
              : tr(`nuovo tentativo tra ${seconds}s`, `next attempt in ${seconds}s`)}
          </p>
        ) : action.error ? (
          <p role="alert">{backendText(action.error)}</p>
        ) : null}
      </div>
      {action.status === 'failed' ? (
        <Button
          variant="icon"
          type="button"
          aria-label={tr('Ignora l’azione non riuscita', 'Dismiss failed action')}
          tip={tr('Ignora', 'Dismiss')}
          onClick={onDismiss}
        >
          <Icon name="close" size={16} />
        </Button>
      ) : canGiveUp(action, now) ? (
        // A retry to abandon: the exported predicate, not a local re-derivation.
        <Button
          variant="secondary"
          className="undo-button"
          type="button"
          disabled={working || action.status === 'committing'}
          title={
            action.status === 'committing'
              ? tr(
                  'Tentativo in corso: non si può rinunciare ora',
                  'Attempt in progress: cannot give up right now',
                )
              : undefined
          }
          onClick={onGiveUp}
          aria-label={`${tr('Rinuncia', 'Give up')}: ${undoLabel(action)} — ${subject}`}
        >
          {tr('Rinuncia', 'Give up')}
        </Button>
      ) : (
        <>
          <span className="undo-seconds" aria-hidden="true">
            {available ? (
              <>
                <Count value={Math.ceil(remaining / 1000)} />s
              </>
            ) : (
              '…'
            )}
          </span>
          <Button
            variant="secondary"
            className="undo-button"
            type="button"
            disabled={!available || working}
            onClick={onUndo}
            aria-label={`${tr('Annulla', 'Undo')}: ${undoLabel(action)} — ${subject}`}
          >
            {available ? tr('Annulla', 'Undo') : tr('In corso…', 'Applying…')}
          </Button>
          <div
            className="undo-track"
            role="progressbar"
            aria-label={tr('Tempo rimasto per annullare', 'Time left to undo')}
            aria-valuemin={0}
            aria-valuemax={(action.expires_at_ms - action.created_at_ms) / 1000}
            aria-valuenow={Math.ceil(remaining / 1000)}
          >
            <span
              style={{
                transform: `scaleX(${Math.min(1, remaining / (action.expires_at_ms - action.created_at_ms))})`,
              }}
            />
          </div>
        </>
      )}
    </motion.div>
  );
}
