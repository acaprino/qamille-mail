import Button from '../../ui/Button';
import Icon from '../../ui/Icon';
import { bookLabel, tr } from '../../lib/i18n';
import type { DuplicateView } from '../../lib/types';

export type DuplicatesPanelProps = {
  groups: DuplicateView[];
  busy: boolean;
  same: (group: DuplicateView) => void;
  different: (group: DuplicateView) => void;
  merge: (group: DuplicateView) => void;
  close: () => void;
};

export default function DuplicatesPanel({
  groups,
  busy,
  same,
  different,
  merge,
  close,
}: DuplicatesPanelProps) {
  return (
    <section className="duplicates-panel" aria-label={tr('Duplicati', 'Duplicates')}>
      <header>
        <h2>{tr('Potenziali duplicati', 'Possible duplicates')}</h2>
        <Button
          variant="icon"
          aria-label={tr('Chiudi duplicati', 'Close duplicates')}
          tip={tr('Chiudi', 'Close')}
          onClick={close}
        >
          <Icon name="close" size={16} />
        </Button>
      </header>
      {!groups.length && (
        <p className="muted">{tr('Nessun duplicato da rivedere.', 'No duplicates to review.')}</p>
      )}
      {groups.map((group) => (
        <article className="duplicate-group" key={group.id}>
          <ul>
            {group.members.map((member) => (
              <li key={member.key}>
                <strong>{member.fields.name || member.fields.emails[0]?.value || '—'}</strong>
                <span className="origin-chip">{bookLabel(member.label)}</span>
                <small className="muted">
                  {member.fields.emails.map((e) => e.value).join(', ')}
                </small>
              </li>
            ))}
          </ul>
          <div className="duplicate-actions">
            <Button variant="secondary" disabled={busy} onClick={() => same(group)}>
              {tr('Stessa persona', 'Same person')}
            </Button>
            <Button variant="secondary" disabled={busy} onClick={() => different(group)}>
              {tr('Persone diverse', 'Different people')}
            </Button>
            {/* The action that goes on to a dialog closes the row, as in a dialog's own buttons. */}
            <Button variant="primary" disabled={busy} onClick={() => merge(group)}>
              {tr('Unisci…', 'Merge…')}
            </Button>
          </div>
        </article>
      ))}
    </section>
  );
}
