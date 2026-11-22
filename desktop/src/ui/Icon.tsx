// Qamille's icon, drawn by Sigrid's `Icon` in the pack in context. The call sites keep asking for a size in pixels: it is
// snapped to the nearest step Sigrid's scale has (12, 14, 16, 20, 28, 32, 36). The stroke is Sigrid's (`--icon-stroke`), so
// `stroke` is accepted and ignored: every visible difference from the old drawing is accepted (the owner's ruling).
import { Icon as SigridIcon } from '@acaprino/sigrid';
import { sigridIcon } from './icon-names';

function sizeClass(size: number): string {
  if (size <= 13) return 'size-3';
  if (size <= 14) return 'size-3.5';
  if (size <= 18) return 'size-4';
  if (size <= 22) return 'size-5';
  if (size <= 31) return 'size-7';
  if (size <= 35) return 'size-8';
  return 'size-9';
}

export type IconProps = { name: string; size?: number; stroke?: number };

export default function Icon({ name, size = 20 }: IconProps) {
  return <SigridIcon name={sigridIcon(name)} className={sizeClass(size)} />;
}
