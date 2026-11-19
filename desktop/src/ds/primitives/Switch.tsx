// An on/off switch with its label: the children are the label, and a click anywhere on the row toggles
// it. For settings that apply at once; `checked` is controlled and `onChange` is told the new value.
// The control is unlumen's Apple Switch (ds/unlumen/apple-switch.tsx): role="switch", spring thumb,
// drag to toggle. The row's look is in ds/styles/ark/switch.css.
import type { ReactNode } from 'react';
import { AppleSwitch } from '../unlumen/apple-switch';

export type SwitchProps = {
  checked: boolean;
  disabled?: boolean;
  onChange?: (checked: boolean) => void;
  'aria-label'?: string;
  children?: ReactNode;
};

// The row, the track and the thumb carry the anatomy attributes of the Ark Switch this replaced
// (data-scope="switch", data-part root/control/thumb, data-state, data-disabled), which the tests and
// the design-system export select on.
export default function Switch({ checked, disabled = false, onChange, 'aria-label': ariaLabel, children }: SwitchProps) {
  const state = checked ? 'checked' : 'unchecked';
  const anatomy = { 'data-scope': 'switch', 'data-state': state, 'data-disabled': disabled ? '' : undefined };
  const control = (
    <AppleSwitch
      checked={checked}
      disabled={disabled}
      onCheckedChange={(next) => onChange?.(next)}
      size="xs"
      tone="app"
      aria-label={ariaLabel}
      {...anatomy}
      data-part="control"
    />
  );
  if (!children)
    return (
      <span className="switch-row" {...anatomy} data-part="root">
        {control}
      </span>
    );
  return (
    <label className="switch-row" {...anatomy} data-part="root">
      {control}
      <span className="switch-label" data-scope="switch" data-part="label">
        {children}
      </span>
    </label>
  );
}
