// A number that changes in place (unread counts, totals): Sigrid's Count. The text and the accessible name are the plain
// number; it does not roll its digits. `className` goes on the number (the app's own spacing around it).
import { Count as SigridCount } from '@acaprino/sigrid/primitives';

export default function Count({ value, className }: { value: number | string; className?: string }) {
  return <SigridCount value={value} className={className} />;
}
