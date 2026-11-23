// The Trascrizione tab: one line per segment, the time (`mm:ss`, `hh:mm:ss` past the hour) and the
// text, read-only, and "Copia" (the text with the times). A meeting from before the versions has
// its plain text, with no times. `highlight` is the segment a search result pointed at: it is
// marked and scrolled into view once, when it appears, and not again when the page re-reads the
// meeting (the reader may have scrolled away). Both top-level elements carry `meeting-transcript`,
// the class its rules are scoped under.
import { useEffect, useRef } from 'react';
import Button from '../../ui/Button';
import Icon from '../../ui/Icon';
import { tr } from '../../lib/i18n';
import type { MeetingDetail } from '../../lib/meetings';
import { transcriptRows } from '../../lib/meetings-view';
import { tick } from '../../lib/tick';
import './MeetingTranscript.css';

export type MeetingTranscriptProps = {
  detail: MeetingDetail;
  highlight?: string | null;
  oncopy: () => void;
};

export default function MeetingTranscript({ detail, highlight = null, oncopy }: MeetingTranscriptProps) {
  const rows = transcriptRows(detail);
  // The caption promises the time of every passage: it is only true for lines that have one.
  const timed = rows.some((row) => row.time);
  const list = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const id = highlight;
    if (!id) return;
    void tick().then(() => {
      const line = [...(list.current?.querySelectorAll<HTMLElement>('[data-segment-id]') ?? [])].find(
        (node) => node.dataset.segmentId === id,
      );
      line?.scrollIntoView({ block: 'center' });
    });
  }, [highlight]);

  return (
    <>
      <div className={timed ? 'toolbar meeting-transcript' : 'toolbar bare meeting-transcript'}>
        {timed && <span>{tr('Testo unico, con l’ora di ogni passaggio', 'One text, with the time of every passage')}</span>}
        <Button variant="secondary" onClick={oncopy}>
          <Icon name="copy" size={15} />
          {tr('Copia', 'Copy')}
        </Button>
      </div>
      <div className="lines selectable meeting-transcript" ref={list}>
        {rows.map((row) => (
          <div
            key={row.id}
            className={['seg', !row.time && 'plain', row.id === highlight && 'hit'].filter(Boolean).join(' ')}
            data-segment-id={row.id}
            aria-current={row.id === highlight ? 'true' : undefined}
          >
            {row.time && <time>{row.time}</time>}
            <span>{row.text}</span>
          </div>
        ))}
      </div>
    </>
  );
}
