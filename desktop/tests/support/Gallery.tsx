// One section per primitive, chosen by `which` (see mount.mjs). Test code, so it lives under
// tests/ and not in the ds module. A task that adds a primitive appends its section here.
import { lazy, Suspense, useState } from 'react';
import Button from '../../src/ds/primitives/Button';
import Icon from '../../src/ds/primitives/Icon';
import SelectSection from './gallery/Select';
import CheckboxGallery from './gallery/CheckboxGallery';
import DateFieldGallery from './gallery/DateFieldGallery';
import SliderGallery from './gallery/SliderGallery';
import TabsGallery from './gallery/TabsGallery';
import RecipientsGallery from './gallery/RecipientsGallery';
import ModalSection from './gallery/Modal';
import PopoverSection from './gallery/Popover';
import MenuSection from './gallery/Menu';
import SidePanelSection from './gallery/SidePanel';
import NoticesGallery from './gallery/NoticesGallery';

// The Sigrid plan (docs/superpowers/plans/2026-11-22-qamille-su-sigrid.md): one insertion point per task. A task
// writes its imports under its own comment and nothing elsewhere; to remove a line it removes only that line, and
// the blank lines stay, so two tasks never touch neighbouring lines.

// Task 3: words

// Task 4: switch

// Task 5: tabs

// Task 6: count and slider

// Task 7: button

// Task 8: checkbox

// Task 9: field

// Task 10: toaster

// Task 11: date field

// Task 12: select

// Task 13: popover, menu and tooltip

// Task 14: modal and side panel

// Task 15: recipients

// Task 17: nav list

// Task 19: icons

// P2 of the Sigrid plan, loaded only by its own section: the other sections do not load the synced Sigrid.
const SigridCoexistence = lazy(() => import('./gallery/SigridCoexistence'));

export type GalleryProps = { which: string; props?: Record<string, any> };

function ButtonSection() {
  const [archived, setArchived] = useState(0);
  return (
    <section aria-label="Button" style={{ padding: '96px 48px' }}>
      {/* The first tab stop is the icon button that carries a tip. */}
      <div id="tipped-row" style={{ display: 'flex', gap: 12, alignItems: 'center' }}>
        <Button variant="icon" type="button" tip="Archive" shortcut="E" aria-label="Archive" onClick={() => setArchived((n) => n + 1)}>
          <Icon name="archive" size={18} />
        </Button>
        <output id="archived" aria-label="Archived count">
          {archived}
        </output>
      </div>
      <div id="plain-row" style={{ display: 'flex', gap: 12, alignItems: 'center', marginTop: 24 }}>
        <Button variant="primary" type="button">
          Send
        </Button>
        <Button variant="secondary" type="button">
          Cancel
        </Button>
        <Button variant="text" type="button">
          Later
        </Button>
        <Button variant="icon" type="button" aria-label="Delete">
          <Icon name="trash" size={18} />
        </Button>
      </div>
    </section>
  );
}

export default function Gallery({ which, props = {} }: GalleryProps) {
  switch (which) {
    case 'button':
      return <ButtonSection />;
    case 'icons':
      // Every name in `names` once, each in its own span; an unknown name shows the mail fallback.
      return (
        <section aria-label="Icons" style={{ padding: 24, display: 'flex', gap: 12 }}>
          {(props.names ?? []).map((name: string) => (
            <span key={name} data-icon={name}>
              <Icon name={name as never} size={24} />
            </span>
          ))}
        </section>
      );
    case 'select':
      return <SelectSection props={props} />;
    case 'window-controls':
      return <NoticesGallery />;
    case 'checkbox':
      return <CheckboxGallery props={props} />;
    case 'tabs':
      return <TabsGallery props={props} />;
    case 'slider':
      return <SliderGallery props={props} />;
    case 'date':
      return <DateFieldGallery props={props} />;
    case 'modal':
      return <ModalSection />;
    case 'side-panel':
      return <SidePanelSection />;
    case 'popover':
      return <PopoverSection />;
    case 'menu':
      return <MenuSection />;
    case 'recipients':
      return <RecipientsGallery props={props} />;
    // P2 of the Sigrid plan: Sigrid's own fields inside Sigrid's own surfaces (tests/sigrid-coexistence.mjs).
    case 'sigrid-coexistence':
      return (
        <Suspense fallback={null}>
          <SigridCoexistence props={props} />
        </Suspense>
      );

    // The Sigrid plan's insertion points for the cases (see above, under the imports).

    // Task 3: words

    // Task 4: switch

    // Task 5: tabs

    // Task 6: count and slider

    // Task 7: button

    // Task 8: checkbox

    // Task 9: field

    // Task 10: toaster

    // Task 11: date field

    // Task 12: select

    // Task 13: popover, menu and tooltip

    // Task 14: modal and side panel

    // Task 15: recipients

    // Task 17: nav list

    // Task 19: icons

    default:
      return null;
  }
}
