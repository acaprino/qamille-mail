import { useRef, useState } from 'react';
import Button from '../ui/Button';
import Icon from '../ui/Icon';
import SidePanel from '../ui/SidePanel';
import AttachedMessageView from './AttachedMessageView';
import AttachmentPreview from './AttachmentPreview';
import { isMessageAttachment } from '../lib/attached-message';
import { tr } from '../lib/i18n';
import { tick } from '../lib/tick';
import type { AttachedMessage, AttachedPart, Attachment } from '../lib/types';
import './AttachmentPanel.css';

export type AttachmentPanelProps = {
  emailId: number;
  attachment: Attachment;
  notify: (message: string, error?: boolean) => void;
  dark: boolean;
  close: () => void;
};

/** One opened level: the attachment itself (`path` empty) or a part inside it. */
interface Level {
  name: string;
  size: number;
  mime: string;
  path: number[];
  message: boolean;
  /** Where the reader was when a deeper level opened, restored on the way back. */
  scrollTop: number;
}

const levelKey = (level: Level) => level.path.join('/');

export default function AttachmentPanel({ emailId, attachment, notify, dark, close }: AttachmentPanelProps) {
  // The parent re-creates the panel for every top-level attachment (key).
  const [levels, setLevels] = useState<Level[]>(() => [
    {
      name: attachment.name,
      size: attachment.size,
      mime: attachment.mime,
      path: [],
      message: isMessageAttachment(attachment.name, attachment.mime),
      scrollTop: 0,
    },
  ]);
  const current = levels[levels.length - 1];
  const parent = levels.length > 1 ? levels[levels.length - 2] : null;
  const panel = useRef<HTMLDivElement>(null);
  const scroller = useRef<HTMLDivElement>(null);

  // Parsed messages by level, so going back does not read and parse the file again.
  const messages = useRef(new Map<string, AttachedMessage>());

  // The content swap removes the button that had focus: keep focus in the
  // panel, or the app's single-key shortcuts would act on the mail behind it.
  async function settle(scrollTop: number) {
    await tick();
    panel.current?.focus({ preventScroll: true });
    const area = scroller.current;
    if (!area) return;
    area.scrollTop = scrollTop;
    requestAnimationFrame(() => {
      if (scroller.current) scroller.current.scrollTop = scrollTop;
    });
  }
  async function openPart(part: AttachedPart) {
    const here = { ...current, scrollTop: scroller.current?.scrollTop ?? 0 };
    setLevels([
      ...levels.slice(0, -1),
      here,
      {
        name: part.name,
        size: part.size,
        mime: part.mime,
        path: [...current.path, part.index],
        message: part.message,
        scrollTop: 0,
      },
    ]);
    await settle(0);
  }
  async function back() {
    if (levels.length < 2) return;
    const next = levels.slice(0, -1);
    setLevels(next);
    await settle(next[next.length - 1].scrollTop);
  }
  async function showAsFile() {
    setLevels([...levels.slice(0, -1), { ...current, message: false }]);
    await settle(0);
  }

  return (
    <SidePanel
      className="attachment-panel"
      aria-label={current.name}
      ref={panel}
      onOpen={() => panel.current?.focus()}
      onClose={close}
      onScrim={close}
    >
      <div className="side-panel-bar">
        {parent && (
          <>
            <Button variant="text" className="panel-back" onClick={() => void back()} tip={parent.name}>
              <span className="chevron-left">
                <Icon name="chevron" size={15} />
              </span>
              <span className="panel-crumb">{parent.name}</span>
            </Button>
            <span className="panel-separator" aria-hidden="true">
              ›
            </span>
          </>
        )}{' '}
        <strong className="panel-title" title={current.name}>
          {current.name}
        </strong>{' '}
        <Button
          variant="icon"
          className="panel-close"
          aria-label={tr('Chiudi l’allegato', 'Close attachment')}
          tip={tr('Chiudi l’allegato', 'Close attachment')}
          onClick={close}
        >
          <Icon name="close" size={16} />
        </Button>
      </div>
      <div className="side-panel-body attachment-panel-scroll" ref={scroller}>
        {current.message ? (
          <AttachedMessageView
            key={levelKey(current)}
            emailId={emailId}
            attachmentId={attachment.id}
            path={current.path}
            notify={notify}
            dark={dark}
            cached={messages.current.get(levelKey(current)) ?? null}
            loaded={(message) => messages.current.set(levelKey(current), message)}
            openPart={(part) => void openPart(part)}
            showAsFile={() => void showAsFile()}
          />
        ) : (
          <AttachmentPreview
            key={levelKey(current)}
            emailId={emailId}
            attachment={{ ...attachment, name: current.name, size: current.size, mime: current.mime }}
            path={current.path}
            embedded
            notify={notify}
            close={close}
          />
        )}
      </div>
    </SidePanel>
  );
}
