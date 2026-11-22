// A checkbox with its label: the children are the label, and a click anywhere on the row toggles it. `checked` is
// controlled; `onChange` is told the new value. `indeterminate` draws the dash of "some, not all" (the checked state is
// then false, and a click ticks it). Without a visible label pass `aria-label`. Drawn by Sigrid's Checkbox, which is
// only the box (a button with role="checkbox"): the row and its label are a <label> around it, as Sigrid's own notes
// ask of an app. The row keeps Ark's anatomy attributes (`data-scope="checkbox"`) that the pages' own rules place it by.
import type { ReactNode } from 'react';
import { Checkbox as SigridCheckbox } from '@acaprino/sigrid';

export type CheckboxProps = {
  checked: boolean;
  indeterminate?: boolean;
  disabled?: boolean;
  'aria-label'?: string;
  onChange?: (checked: boolean) => void;
  children?: ReactNode;
};

export default function Checkbox({ checked, indeterminate = false, disabled = false, 'aria-label': ariaLabel, onChange, children }: CheckboxProps) {
  const box = (
    <SigridCheckbox
      checked={indeterminate ? 'indeterminate' : checked}
      disabled={disabled}
      aria-label={ariaLabel}
      onCheckedChange={(next) => onChange?.(next === true)}
    />
  );
  if (!children) return box;
  return (
    <label className="checkbox-row" data-scope="checkbox" data-part="root" data-disabled={disabled ? '' : undefined}>
      {box}
      <span data-scope="checkbox" data-part="label">
        {children}
      </span>
    </label>
  );
}
