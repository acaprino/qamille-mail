// An on/off switch with its label: the children are the label, and a click anywhere on the row toggles it. For settings
// that apply at once; `checked` is controlled and `onChange` is told the new value. Drawn by Sigrid's Switch (Ark's
// switch: a hidden `role="switch"` checkbox in a label); without a visible label the name is the `aria-label`.
import type { ReactNode } from 'react';
import { Switch as SigridSwitch } from '@acaprino/sigrid/primitives';

export type SwitchProps = {
  checked: boolean;
  disabled?: boolean;
  onChange?: (checked: boolean) => void;
  'aria-label'?: string;
  children?: ReactNode;
};

export default function Switch({ checked, disabled = false, onChange, 'aria-label': ariaLabel, children }: SwitchProps) {
  return (
    <SigridSwitch checked={checked} disabled={disabled} onCheckedChange={(next) => onChange?.(next)} aria-label={ariaLabel}>
      {children}
    </SigridSwitch>
  );
}
