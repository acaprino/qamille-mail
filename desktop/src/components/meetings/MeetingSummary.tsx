// The Sintesi tab. With a summary: the text, the key points, the decisions and the actions (each
// one can be ticked), and "Copia". Without one it says why and what to do: the summary is being
// written (phase, bar, "Annulla"), it failed ("Riprova"), there is no assistant to write it (a
// link to Settings), or it was never asked for ("Genera sintesi"). A meeting from before the
// versions shows its own summary and key points. The page does the work; this draws and reports.
// Every top-level element carries `meeting-summary`, the class its rules are scoped under.
import { useEffect, useRef, useState } from 'react';
import Button from '../../ds/primitives/Button';
import Checkbox from '../../ds/primitives/Checkbox';
import Icon from '../../ds/primitives/Icon';
import { backendText, tr } from '../../lib/i18n';
import type { MeetingAction, MeetingDetail, MeetingJob } from '../../lib/meetings';
import type { SummaryView } from '../../lib/meetings-view';
import MeetingProgress from './MeetingProgress';
import './MeetingSummary.css';

export type MeetingSummaryProps = {
  detail: MeetingDetail;
  view: SummaryView;
  busy: boolean;
  oncopy: () => void;
  ontoggle: (action: MeetingAction, done: boolean) => void;
  onretry: (jobId: string) => void;
  oncancel: (job: MeetingJob) => void;
  ongenerate: () => void;
  onsettings: (section: 'Assistant') => void;
};

const who = (action: MeetingAction) => [action.owner, action.due].filter(Boolean).join(', ');

export default function MeetingSummary({
  detail,
  view,
  busy,
  oncopy,
  ontoggle,
  onretry,
  oncancel,
  ongenerate,
  onsettings,
}: MeetingSummaryProps) {
  const active = detail.active_summary ? detail.summaries.find((s) => s.id === detail.active_summary) : undefined;
  // The active version, or what a meeting from before the versions kept in its own fields.
  const text = active ? active.summary : detail.meeting.summary;
  const keyPoints = active ? active.key_points : detail.meeting.key_points;
  const decisions = active ? active.decisions.map((d) => d.text) : [];
  const actions = active ? active.actions : [];
  const oldActions = active ? [] : detail.meeting.action_items;

  // A box shows what was just pressed until the page has the answer; when the answer comes (the
  // meeting changed, or was read again after a refusal) the meeting decides, so a box can never
  // stay ticked or empty against what the meeting says. While an answer is awaited the boxes stay
  // enabled (a disabled control drops the keyboard focus for good): a press made then is kept, the
  // last one on each box wins, and they are saved one at a time, each as soon as the answer to the
  // one before is in (so each goes out with the revision that answer brought).
  const [shown, setShown] = useState<Record<string, boolean>>({});
  const queued = useRef(new Map<string, boolean>()).current;
  function choose(action: MeetingAction, done: boolean) {
    setShown((before) => ({ ...before, [action.id]: done }));
    if (busy) queued.set(action.id, done);
    else ontoggle(action, done);
  }
  useEffect(() => {
    if (busy) return;
    for (const [id, done] of queued) {
      queued.delete(id);
      const current = actions.find((action) => action.id === id);
      // A press that changes nothing (the box is already as pressed) sends nothing; the next one does.
      if (current && current.done !== done) return ontoggle(current, done);
    }
    setShown((before) => (Object.keys(before).length ? {} : before));
    // Runs when the meeting or `busy` changes, as the Svelte effect did; the rest is read as it is now.
  }, [detail, busy]);

  /** What the failed job said, or the plain words when it said nothing (a job cancelled by hand). */
  let failure = '';
  if (view.kind === 'failed') {
    if (view.message.trim()) failure = backendText(view.message);
    else {
      const cancelled = detail.jobs.find((job) => job.id === view.jobId)?.state === 'cancelled';
      failure = cancelled
        ? tr('La sintesi è stata annullata.', 'The summary was cancelled.')
        : tr('La sintesi non è riuscita.', 'The summary did not work.');
    }
  }
  const ariaBusy = busy ? 'true' : undefined;

  if (view.kind === 'text')
    return (
      <>
        <div className="toolbar meeting-summary">
          <Button variant="secondary" onClick={oncopy}>
            <Icon name="copy" size={15} />
            {tr('Copia', 'Copy')}
          </Button>
        </div>
        <div className="prose selectable meeting-summary">
          {text.trim() && (
            <section>
              <h4>{tr('In breve', 'In brief')}</h4>
              <p>{text}</p>
            </section>
          )}
          {keyPoints.length > 0 && (
            <section>
              <h4>{tr('Punti chiave', 'Key points')}</h4>
              <ul>
                {keyPoints.map((point, i) => (
                  <li key={i}>{point}</li>
                ))}
              </ul>
            </section>
          )}
          {decisions.length > 0 && (
            <section>
              <h4>{tr('Decisioni', 'Decisions')}</h4>
              <ul>
                {decisions.map((decision, i) => (
                  <li key={i}>{decision}</li>
                ))}
              </ul>
            </section>
          )}
          {actions.length > 0 ? (
            <section>
              <h4>{tr('Azioni', 'Actions')}</h4>
              <div className="actions">
                {actions.map((action) => (
                  <div className="act-row" key={action.id}>
                    <Checkbox checked={shown[action.id] ?? action.done} onChange={(done) => choose(action, done)}>
                      <span className={action.done ? 'act-text done' : 'act-text'}>{action.text}</span>
                    </Checkbox>
                    {who(action) && <span className="who">{who(action)}</span>}
                  </div>
                ))}
              </div>
            </section>
          ) : oldActions.length > 0 ? (
            <section>
              <h4>{tr('Azioni', 'Actions')}</h4>
              <ul>
                {oldActions.map((item, i) => (
                  <li key={i}>{item}</li>
                ))}
              </ul>
            </section>
          ) : null}
        </div>
      </>
    );
  if (view.kind === 'pending')
    return <MeetingProgress job={view.job} compact busy={busy} oncancel={() => oncancel(view.job)} />;
  if (view.kind === 'failed')
    return (
      <div className="problem meeting-summary">
        <div className="notice danger" role="alert">
          <Icon name="alert" size={16} />
          <span>{failure}</span>
        </div>
        <Button variant="secondary" aria-busy={ariaBusy} onClick={() => onretry(view.jobId)}>
          <Icon name="refresh" size={15} />
          {tr('Riprova', 'Try again')}
        </Button>
      </div>
    );
  if (view.kind === 'ai_missing')
    return (
      <div className="notice info meeting-summary">
        <Icon name="info" size={16} />
        <span>
          {tr('Configura un assistente in ', 'Set up an assistant in ')}
          <button type="button" className="link" onClick={() => onsettings('Assistant')}>
            {tr('Impostazioni', 'Settings')}
          </button>
          {tr(' per avere la sintesi', ' to get the summary')}
        </span>
      </div>
    );
  return (
    <div className="problem meeting-summary">
      <p className="muted">{tr('Questa riunione non ha ancora una sintesi.', 'This meeting has no summary yet.')}</p>
      <Button variant="primary" aria-busy={ariaBusy} onClick={ongenerate}>
        <Icon name="spark" size={15} />
        {tr('Genera sintesi', 'Generate summary')}
      </Button>
    </div>
  );
}
