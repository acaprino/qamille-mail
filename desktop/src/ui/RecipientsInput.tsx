// The people a message goes to, drawn by Sigrid's ChipsInput (Ark's tags input) with its own list of suggestions. `value` is a
// list of entries, each an address ("ann@x.it") or a name with an address ("Ann Rossi <ann@x.it>"); an entry is a removable
// chip that shows the name, or the address when there is none. Typing asks the caller for suggestions (`onQuery`, with the
// text typed; `suggestions` is the answer): the arrows move, Enter picks, Esc closes only the list. Enter, a comma or a
// semicolon turns the typed text into a chip, so does leaving the box (until then the typed text is `pending`, which
// `onPendingChange` reports so the caller can count it as part of the value); pasting a list splits it at commas and
// semicolons; Backspace on the empty box removes the last chip. `invalid` marks the chips whose address the caller refuses.
// What ChipsInput has not is done around it: the name for screen readers (a hidden label), `disabled` (a fieldset), `inputRef`,
// the same address twice, and the tallest the box grows before it scrolls. The words of the chips are the app's own, in a
// WordsProvider of their own. The repeated name in a suggestion is Sigrid's to fix (R11 of the plan's asks).
import { useEffect, useId, useMemo, type Ref } from 'react';
import { ChipsInput, WordsProvider } from '@acaprino/sigrid';
import { useLanguage } from '../lib/i18n';
import { recipientWords } from './words';

type Suggestion = { email: string; name?: string };

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
  /** The text typed and not yet a chip: reported through `onPendingChange`. */
  pending?: string;
  onPendingChange?: (text: string) => void;
  /** Not read: the words are the app's own, in `recipientWords`. */
  translations?: unknown;
  listTranslations?: unknown;
  suggestionsLabel?: string;
  className?: string;
};

// "Ann Rossi <ann@x.it>" to its parts; a bare address has no name.
const split = (entry: string) => {
  const found = /^(.*?)\s*<([^<>]+)>\s*$/.exec(entry);
  return found ? { name: found[1].trim().replace(/^"|"$/g, ''), email: found[2].trim() } : { name: '', email: entry.trim() };
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
  onPendingChange,
  suggestionsLabel,
  className,
}: RecipientsInputProps) {
  const id = useId();
  const italian = useLanguage();
  const words = useMemo(() => recipientWords(italian), [italian]);
  // The suggestions are entries; the ones already in the box are left out (ChipsInput does it too, by entry).
  const valueKey = value.join('\n');
  const offered = useMemo(
    () => suggestions.map(entryOf),
    // eslint-disable-next-line react-hooks/exhaustive-deps -- keyed on the content, not the array identity
    [suggestions.map(entryOf).join('\n')],
  );
  const taken = useMemo(
    () => new Set(value.map((entry) => split(entry).email.toLowerCase())),
    // eslint-disable-next-line react-hooks/exhaustive-deps -- keyed on the content, not the array identity
    [valueKey],
  );
  const fresh = useMemo(() => offered.filter((entry) => !taken.has(split(entry).email.toLowerCase())), [offered, taken]);
  // The caller reads the box through its id (a ref for the focus, as before).
  useEffect(() => {
    assign(inputRef, document.getElementById(id) as HTMLInputElement | null);
  });
  return (
    <WordsProvider words={words}>
      <fieldset className={['recipients-box', className].filter(Boolean).join(' ')} disabled={disabled}>
        <label htmlFor={id} className="visually-hidden">
          {ariaLabel}
        </label>
        <ChipsInput
          id={id}
          value={value}
          onChange={(next) => onChange?.(unique(next))}
          placeholder={placeholder}
          suggestions={fresh}
          onQuery={(text) => {
            onPendingChange?.(text);
            onQuery?.(text);
          }}
          suggestionsLabel={suggestionsLabel}
          display={(entry) => split(entry).name || split(entry).email}
          invalid={(entry) => invalid?.(split(entry).email) ?? false}
          separators={[',', ';']}
          dot={false}
        />
      </fieldset>
    </WordsProvider>
  );
}
