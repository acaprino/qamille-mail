// Asked once per account when the history brought many old unread messages into the Inbox (EX-9):
// mark the ones older than 30 days read, here and on Gmail, or leave the Inbox as it is.
import { useRef, useState } from 'react';
import Button from '../ui/Button';
import Modal from '../ui/Modal';
import { call, errorText } from '../lib/api';
import { tr } from '../lib/i18n';

export type HistoryUnreadDialogProps = {
  accountId: number;
  account: string;
  count: number;
  onClose: (message?: string, error?: boolean) => void;
};

export default function HistoryUnreadDialog({ accountId, account, count, onClose }: HistoryUnreadDialogProps) {
  const [busy, setBusy] = useState(false);
  // The dialog opens by itself, possibly while someone is typing: the focus starts on Leave as is,
  // not on the first button, so a Space or Enter right away does not mark old mail read on Gmail.
  const leave = useRef<HTMLButtonElement>(null);
  async function markRead() {
    setBusy(true);
    try {
      const changed = await call<number>('mark_old_unread_read', { accountId });
      onClose(
        tr(`${changed} messaggi segnati come letti`, `${changed} messages marked as read`),
      );
    } catch (e) {
      setBusy(false);
      onClose(errorText(e), true);
    }
  }
  return (
    <Modal
      size="sm"
      aria-labelledby="history-unread-title"
      onClose={() => onClose()}
      initialFocus={() => leave.current}
    >
      <h2 id="history-unread-title">{tr('Molta posta vecchia da leggere', 'Lots of old unread mail')}</h2>
      <p>
        {tr(
          `Il recupero della posta precedente di ${account} ha portato in Posta in arrivo ${count} messaggi non letti.`,
          `Bringing in the older mail of ${account} added ${count} unread messages to the Inbox.`,
        )}
      </p>
      <div className="button-row">
        <Button variant="primary" disabled={busy} onClick={() => void markRead()}>
          {tr('Segna come letti quelli più vecchi di 30 giorni', 'Mark as read those older than 30 days')}
        </Button>
        <Button ref={leave} variant="secondary" disabled={busy} onClick={() => onClose()}>
          {tr('Lascia così', 'Leave as is')}
        </Button>
      </div>
    </Modal>
  );
}
