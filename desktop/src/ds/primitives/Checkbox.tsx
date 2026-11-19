// A checkbox with its label: the children are the label, and a click anywhere on the row toggles it.
// `checked` is controlled; `onChange` is told the new value. `indeterminate` draws the dash of "some,
// not all" (the checked state is then false, and a click ticks it). Without a visible label pass
// `aria-label`. The look is in ds/styles/ark/checkbox.css.
import { Checkbox as ArkCheckbox } from '@ark-ui/react/checkbox';
import type { ReactNode } from 'react';
import Icon from './Icon';

export type CheckboxProps = {
  checked: boolean;
  indeterminate?: boolean;
  disabled?: boolean;
  'aria-label'?: string;
  onChange?: (checked: boolean) => void;
  children?: ReactNode;
};

export default function Checkbox({
  checked,
  indeterminate = false,
  disabled = false,
  'aria-label': ariaLabel,
  onChange,
  children,
}: CheckboxProps) {
  return (
    <ArkCheckbox.Root
      checked={indeterminate ? 'indeterminate' : checked}
      disabled={disabled}
      onCheckedChange={(details) => onChange?.(details.checked === true)}
    >
      <ArkCheckbox.Control>
        <ArkCheckbox.Indicator>
          <Icon name="check" size={12} stroke={2.6} />
        </ArkCheckbox.Indicator>
        <ArkCheckbox.Indicator indeterminate>
          <Icon name="minus" size={12} stroke={2.6} />
        </ArkCheckbox.Indicator>
      </ArkCheckbox.Control>
      {children && <ArkCheckbox.Label>{children}</ArkCheckbox.Label>}
      {/* With an aria-label the name is that, not the visible label (Zag points aria-labelledby at the label). */}
      <ArkCheckbox.HiddenInput aria-label={ariaLabel} aria-labelledby={ariaLabel ? '' : undefined} />
    </ArkCheckbox.Root>
  );
}
