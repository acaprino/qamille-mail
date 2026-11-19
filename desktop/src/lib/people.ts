// Names for the people on a message. A message stores addresses; the name to show is the one
// already known for that address (people seen as a sender or a recipient in saved mail), and
// failing that the address itself. Pure functions: the reader and the list both use them.

/** Lowercase address -> the name known for it. */
export type PersonIndex = Map<string, string>;

export interface RecipientView {
  /** The first few names, the reader first. */
  shown: string[];
  /** How many more there are beyond `shown`. */
  more: number;
  /** Everyone, in the same order, for the full list. */
  all: string[];
}

export function personIndex(people: { name: string; address: string }[]): PersonIndex {
  const index: PersonIndex = new Map();
  for (const { name, address } of people) {
    const key = addressOf(address);
    if (key && name.trim() && !index.has(key)) index.set(key, name.trim());
  }
  return index;
}

/** The lowercase address of a recipient written as "Name <address>" or as a bare address. */
export function addressOf(value: string): string {
  const angle = /<([^<>]*)>/.exec(value);
  return (angle ? angle[1] : value).trim().toLowerCase();
}

function nameWrittenIn(value: string): string {
  const angle = /^\s*"?([^"<]*?)"?\s*<[^<>]*>\s*$/.exec(value);
  return angle ? angle[1].trim() : '';
}

/** The name known for an address, else the name written beside it, else the address. */
export function displayName(index: PersonIndex, recipient: string): string {
  const address = addressOf(recipient);
  return index.get(address) || nameWrittenIn(recipient) || address;
}

/**
 * Who is on a message, shortened. The reader comes first and counts once however many of their
 * addresses are on it; the others keep the order they were written in, each once.
 * `shownCount` names are shown, the rest are counted in `more`.
 */
export function recipientView(
  recipients: string[],
  index: PersonIndex,
  self: Set<string>,
  youLabel: string,
  shownCount = 3,
): RecipientView {
  const seen = new Set<string>();
  const others: string[] = [];
  let reader = false;
  for (const recipient of recipients) {
    const address = addressOf(recipient);
    if (!address) continue;
    if (self.has(address)) {
      reader = true;
      continue;
    }
    if (seen.has(address)) continue;
    seen.add(address);
    others.push(displayName(index, recipient));
  }
  const all = reader ? [youLabel, ...others] : others;
  return { shown: all.slice(0, shownCount), more: Math.max(0, all.length - shownCount), all };
}
