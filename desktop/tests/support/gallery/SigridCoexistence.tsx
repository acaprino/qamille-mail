// P2 of the Sigrid plan (tests/sigrid-coexistence.mjs): Sigrid's fields that rest on Ark UI (DateField,
// DateTimeField, Combobox, ChipsInput with suggestions) inside Sigrid's surfaces on Base UI (Dialog, Sheet,
// Popover), and Sigrid's notices beside an open surface. Sigrid's primitives only, from the synced copy: nothing of
// Qamille's src/ds. The relative path reaches the copy without the alias of the Sigrid plan's Task 1; the page
// is the clean room (tests/support/clean-room.html), so only Sigrid's CSS applies.
//
// props: `surface` 'dialog' | 'sheet' | 'popover', or 'none' for the field alone in the page; `field` 'date' | 'datetime' | 'combobox' | 'chips' | 'none';
// `toasts` true for a notice (with «Undo», one action and its «×») in the page, outside the surface.
import { useState } from 'react';
import {
  Button,
  ChipsInput,
  Combobox,
  DateField,
  DateTimeField,
  DialogContent,
  DialogDescription,
  DialogRoot,
  DialogTitle,
  Popover,
  PopoverContent,
  PopoverTrigger,
  Sheet,
  SheetContent,
  SheetDescription,
  SheetTitle,
  Toasts,
} from '../../../vendor/sigrid/dist/index.js';

type Surface = 'dialog' | 'sheet' | 'popover' | 'none';
type Field = 'date' | 'datetime' | 'combobox' | 'chips' | 'none';
type Notice = { id: string; text: string; undo?: { label: string; onClick: () => void }; actions?: { label: string; items: { id: string; label: string; onClick: () => void }[] }[] };

const OWNERS = [
  { value: 'ann', label: 'Ann Rossi' },
  { value: 'bob', label: 'Bob Bianchi' },
  { value: 'cid', label: 'Cid Verdi' },
];
const PEOPLE = ['ann@example.com', 'bob@example.com', 'carla@example.com'];
const FIELD_ID = 'coexistence-field';

export default function SigridCoexistence({ props = {} }: { props?: Record<string, any> }) {
  const surface: Surface = props.surface ?? 'dialog';
  const field: Field = props.field ?? 'date';
  const [open, setOpen] = useState(false);
  const [closes, setCloses] = useState(0);
  const [day, setDay] = useState<string | null>('2026-11-10');
  const [when, setWhen] = useState<string | null>('2026-11-10T09:00');
  const [owner, setOwner] = useState<string | null>(null);
  const [people, setPeople] = useState<string[]>([]);
  const [undos, setUndos] = useState(0);
  const [moves, setMoves] = useState(0);
  const [notices, setNotices] = useState<string[]>(props.toasts ? ['archived'] : []);
  const change = (next: boolean) => {
    setOpen(next);
    if (!next) setCloses((n) => n + 1);
  };
  const items: Notice[] = notices.map((id) => ({
    id,
    text: 'Message archived',
    undo: { label: 'Undo', onClick: () => setUndos((n) => n + 1) },
    actions: [{ label: 'Move to', items: [{ id: 'inbox', label: 'Inbox', onClick: () => setMoves((n) => n + 1) }] }],
  }));
  const value = { date: day, datetime: when, combobox: owner, chips: people.join(', '), none: '' }[field] ?? '';

  const control =
    field === 'date' ? (
      <>
        <label htmlFor={FIELD_ID}>Due date</label>
        <DateField id={FIELD_ID} value={day} onChange={setDay} />
      </>
    ) : field === 'datetime' ? (
      <>
        <label htmlFor={FIELD_ID}>Reminder</label>
        <DateTimeField id={FIELD_ID} value={when} onChange={setWhen} />
      </>
    ) : field === 'combobox' ? (
      <>
        <label htmlFor={FIELD_ID}>Owner</label>
        <Combobox id={FIELD_ID} options={OWNERS} value={owner} onValueChange={setOwner} />
      </>
    ) : field === 'chips' ? (
      <>
        <label htmlFor={FIELD_ID}>To</label>
        <ChipsInput id={FIELD_ID} value={people} onChange={setPeople} suggestions={PEOPLE} dot={false} separators={[',', ';']} />
      </>
    ) : null;
  // The field, then one more control: Tab has somewhere to go inside the surface.
  const body = (
    <div style={{ display: 'grid', gap: 8 }}>
      {control}
      <Button type="button" variant="secondary">
        Save
      </Button>
    </div>
  );

  return (
    <section aria-label="Sigrid coexistence" style={{ padding: 48 }}>
      <div style={{ display: 'flex', gap: 12, alignItems: 'center' }}>
        {surface === 'popover' ? (
          <Popover open={open} onOpenChange={change}>
            <PopoverTrigger asChild>
              <Button type="button">Open popover</Button>
            </PopoverTrigger>
            <PopoverContent aria-label="Filters">{body}</PopoverContent>
          </Popover>
        ) : surface === 'none' ? null : (
          <Button type="button" onClick={() => setOpen(true)}>
            Open {surface}
          </Button>
        )}
        <button type="button">Page button</button>
      </div>
      <output aria-label="Field value">{value ?? ''}</output>
      <output aria-label="Surface closes">{closes}</output>
      <output aria-label="Undo count">{undos}</output>
      <output aria-label="Move count">{moves}</output>
      <output aria-label="Notice count">{notices.length}</output>
      {surface === 'dialog' ? (
        <DialogRoot open={open} onOpenChange={change}>
          <DialogContent>
            <DialogTitle>Edit event</DialogTitle>
            <DialogDescription>Change the event and save it.</DialogDescription>
            {body}
          </DialogContent>
        </DialogRoot>
      ) : surface === 'sheet' ? (
        <Sheet open={open} onOpenChange={change}>
          <SheetContent side="right">
            <SheetTitle>Search</SheetTitle>
            <SheetDescription>Narrow the search.</SheetDescription>
            {body}
          </SheetContent>
        </Sheet>
      ) : surface === 'none' ? (
        body
      ) : null}
      {props.toasts ? <Toasts items={items} onDismiss={(id: string) => setNotices((all) => all.filter((n) => n !== id))} /> : null}
    </section>
  );
}
