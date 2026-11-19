// The people a message goes to, on Ark TagsInput (the chips) and Combobox (the suggestions) sharing one
// text box. `value` is a list of entries, each an address ("ann@x.it") or a name with an address
// ("Ann Rossi <ann@x.it>"); an entry is a removable chip. Typing asks the caller for suggestions
// (`onQuery`, with the text after the last separator; `suggestions` is the answer, shown in a list on
// the toast layer): the arrows move, Enter picks, Esc closes only the list. Enter, a comma or a
// semicolon turns the typed text into a chip, so does leaving the box (until then the typed text is
// `pending`, controllable with `onPendingChange`, so the caller can count it as part of the value);
// pasting a list splits it at commas and semicolons; Backspace on the empty box removes the last chip.
// `invalid` marks the chips whose address the caller refuses (data-invalid). The strings come from
// `translations` (chips) and `listTranslations` (the list). The look is in ds/styles/ark/recipients.css.
import { Combobox, createListCollection, type ComboboxRootProps } from '@ark-ui/react/combobox';
import { Portal } from '@ark-ui/react/portal';
import { TagsInput, type TagsInputRootProps } from '@ark-ui/react/tags-input';
import { useEffect, useMemo, useRef, useState, type FormEvent, type KeyboardEvent as ReactKeyboardEvent, type Ref } from 'react';
import { imeComposing } from '../ime';
import Icon from './Icon';

type Suggestion = { email: string; name?: string };
type Translations = NonNullable<TagsInputRootProps['translations']>;
type ListTranslations = NonNullable<ComboboxRootProps<{ value: string; label: string }>['translations']>;

export type RecipientsInputProps = {
  value?: string[];
  onChange?: (value: string[]) => void;
  suggestions: Suggestion[];
  'aria-label': string;
  onQuery?: (text: string) => void;
  invalid?: (email: string) => boolean;
  placeholder?: string;
  disabled?: boolean;
  inputRef?: Ref<HTMLInputElement>;
  pending?: string;
  onPendingChange?: (text: string) => void;
  translations?: Partial<Translations>;
  listTranslations?: ListTranslations;
  suggestionsLabel?: string;
  className?: string;
};

// "Ann Rossi <ann@x.it>" to its parts; a bare address has no name.
const split = (entry: string) => {
  const found = /^(.*?)\s*<([^<>]+)>\s*$/.exec(entry);
  return found
    ? { name: found[1].trim().replace(/^"|"$/g, ''), email: found[2].trim() }
    : { name: '', email: entry.trim() };
};
const entryOf = (s: Suggestion) => (s.name && s.name !== s.email ? `${s.name} <${s.email}>` : s.email);

// The same address is never in the list twice (case does not matter): the first stays.
function unique(entries: string[]) {
  const seen = new Set<string>();
  return entries.filter((entry) => {
    const email = split(entry).email.toLowerCase();
    if (seen.has(email)) return false;
    seen.add(email);
    return true;
  });
}

const NONE: string[] = [];

function assign<T>(ref: Ref<T> | undefined, node: T | null) {
  if (typeof ref === 'function') ref(node);
  else if (ref) (ref as { current: T | null }).current = node;
}

export default function RecipientsInput({
  value = NONE,
  onChange,
  suggestions,
  'aria-label': ariaLabel,
  onQuery,
  invalid,
  placeholder = '',
  disabled = false,
  inputRef,
  pending: pendingProp,
  onPendingChange,
  translations,
  listTranslations,
  suggestionsLabel = 'Suggestions',
  className,
}: RecipientsInputProps) {
  const [ownPending, setOwnPending] = useState('');
  const pending = pendingProp ?? ownPending;
  const setPending = (text: string) => {
    if (pendingProp === undefined) setOwnPending(text);
    onPendingChange?.(text);
  };
  const input = useRef<HTMLInputElement | null>(null);
  const [open, setOpen] = useState(false);

  // Callers rebuild the entries on every render: the suggestions and Ark's collection follow their
  // content, so the combobox keeps its highlighted item while the composer around it re-renders.
  const valueKey = value.join('\n');
  const taken = useMemo(
    () => new Set(value.map((entry) => split(entry).email.toLowerCase())),
    // eslint-disable-next-line react-hooks/exhaustive-deps -- keyed on the content, not the array identity
    [valueKey],
  );
  const items = useMemo(
    () =>
      suggestions
        .filter((s) => !taken.has(s.email.toLowerCase()))
        .map((s) => ({ value: entryOf(s), label: s.name || s.email, email: s.email, name: s.name ?? '' })),
    [suggestions, taken],
  );
  const collection = useMemo(() => createListCollection({ items }), [items]);

  // Both Ark machines write the typed text; whichever does it first, the caller is asked once per change.
  const query = useRef(onQuery);
  query.current = onQuery;
  useEffect(() => {
    query.current?.(pending);
  }, [pending]);

  function add(entries: string[]) {
    const fresh = entries.map((entry) => entry.trim()).filter(Boolean);
    if (fresh.length) onChange?.(unique([...value, ...fresh]));
  }
  // Leaving the box with text in it keeps the text as a chip.
  function leave() {
    const text = pending.trim();
    if (!text) return;
    setPending('');
    add(text.split(/[,;]/));
  }
  // An Esc with nothing to show closes the list before Ark's own document listener sees the key, so it
  // reaches the window around (the composer minimizes).
  const state = useRef({ open, empty: items.length === 0 });
  state.current = { open, empty: items.length === 0 };
  useEffect(() => {
    const emptyList = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && !imeComposing(event) && state.current.open && state.current.empty && event.target === input.current)
        setOpen(false);
    };
    window.addEventListener('keydown', emptyList, true);
    return () => window.removeEventListener('keydown', emptyList, true);
  }, []);
  // Text that arrives with a separator in one input event (an autofill, a value set at once) becomes
  // chips here: Ark's own delimiter rule adds the text it already had, which in React is the previous
  // render's (the Svelte binding had updated it before Ark read it). A paste goes through addOnPaste.
  function typedDelimiter(event: FormEvent<HTMLInputElement>) {
    const text = event.currentTarget.value;
    if ((event.nativeEvent as InputEvent).inputType === 'insertFromPaste' || !/[,;]/.test(text)) return;
    const box = event.currentTarget;
    add(text.split(/[,;]/));
    // After this event has been handled and rendered (the Combobox writes the typed text into `pending`).
    // The box is emptied by hand too: when `pending` was '' already, nothing re-renders to clear it.
    setTimeout(() => {
      setPending('');
      box.value = '';
    });
  }
  // Backspace on the empty box takes the last chip off at once (Ark would first highlight it).
  function keys(event: ReactKeyboardEvent<HTMLDivElement>) {
    if (event.key !== 'Backspace' || event.target !== input.current || pending !== '' || !value.length) return;
    if (imeComposing(event.nativeEvent)) return;
    event.preventDefault();
    event.stopPropagation();
    onChange?.(value.slice(0, -1));
  }

  return (
    <Combobox.Root
      collection={collection}
      disabled={disabled}
      className={className}
      translations={listTranslations}
      value={[]}
      onValueChange={(details) => {
        add(details.value);
        setPending('');
      }}
      inputValue={pending}
      onInputValueChange={(details) => setPending(details.inputValue)}
      open={open}
      onOpenChange={(details) => setOpen(details.open)}
      allowCustomValue
      selectionBehavior="clear"
      openOnClick={false}
      positioning={{ placement: 'bottom-start', sameWidth: true, gutter: 4, strategy: 'fixed' }}
      lazyMount
      unmountOnExit
    >
      <TagsInput.Root
        disabled={disabled}
        placeholder={placeholder}
        value={value}
        onValueChange={(details) => onChange?.(unique(details.value.filter(Boolean)))}
        inputValue={pending}
        onInputValueChange={(details) => setPending(details.inputValue)}
        delimiter={/[,;]/}
        addOnPaste
        blurBehavior="add"
        translations={{ deleteTagTriggerLabel: (entry: string) => `Remove ${entry}`, ...translations }}
      >
        <Combobox.Control>
          <TagsInput.Control onKeyDownCapture={keys}>
            {value.map((entry, index) => {
              const who = split(entry);
              return (
                <TagsInput.Item
                  key={entry}
                  index={index}
                  value={entry}
                  data-invalid={invalid?.(who.email) ? '' : undefined}
                >
                  <TagsInput.ItemPreview title={entry}>
                    <TagsInput.ItemText>{who.name || who.email}</TagsInput.ItemText>
                    <TagsInput.ItemDeleteTrigger>
                      <Icon name="close" size={12} />
                    </TagsInput.ItemDeleteTrigger>
                  </TagsInput.ItemPreview>
                </TagsInput.Item>
              );
            })}
            <Combobox.Input aria-label={ariaLabel} asChild>
              <TagsInput.Input
                ref={(node) => {
                  input.current = node;
                  assign(inputRef, node);
                }}
                onBlur={leave}
                onInput={typedDelimiter}
              />
            </Combobox.Input>
          </TagsInput.Control>
        </Combobox.Control>
      </TagsInput.Root>
      {/* The list exists whenever Ark has it open, empty and hidden if there is no match yet: Ark
          attaches its Esc and outside-press handling when it opens. */}
      {open && (
        <Portal>
          <Combobox.Positioner>
            <Combobox.Content aria-label={suggestionsLabel} hidden={items.length === 0}>
              {items.map((item) => (
                <Combobox.Item key={item.value} item={item}>
                  <Combobox.ItemText>{item.label}</Combobox.ItemText>
                  {item.name && <small>{item.email}</small>}
                </Combobox.Item>
              ))}
            </Combobox.Content>
          </Combobox.Positioner>
        </Portal>
      )}
    </Combobox.Root>
  );
}
