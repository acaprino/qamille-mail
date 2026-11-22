// The words Sigrid says by itself, under Qamille's root (src/ui/SigridRoot.tsx): a notice column with an error and
// a button that switches the interface language, to see the region's name follow it.
import { Toasts } from '@acaprino/sigrid';
import { setLanguageChoice } from '../../../src/lib/i18n';
import { SigridRoot } from '../../../src/ui/SigridRoot';

export default function WordsGallery() {
  return (
    <SigridRoot>
      <section aria-label="Words" style={{ padding: 48 }}>
        <div style={{ position: 'relative', zIndex: 10, display: 'flex', gap: 12 }}>
          <button type="button" onClick={() => setLanguageChoice('it')}>
            Italiano
          </button>
          <button type="button" onClick={() => setLanguageChoice('en')}>
            English
          </button>
        </div>
        <Toasts inline items={[{ id: 'a', text: 'Message sent', undo: { onClick: () => {} } }, { id: 'b', text: 'Not sent', kind: 'error' }]} onDismiss={() => {}} durationMs={0} />
      </section>
    </SigridRoot>
  );
}
