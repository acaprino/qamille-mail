// A choice from a short list, drawn by Sigrid's Select (Base UI's select): a button, as high as a control, that shows the
// chosen item's label, and a list that opens under it. Keyboard: Enter, Space or the arrows open it; arrows move, Enter
// chooses, Escape closes and the focus stays on the button; typing a letter jumps to the item that starts with it.
// `label` is the visible label and the button's name; without it pass `aria-label`. `onChange` is told the new value
// ('' = nothing chosen). `className` goes on the button. Sigrid's Select has no disabled item and no `aria-required`
// (asked of Sigrid as R18): an item written `disabled` is shown as any other.
import { useId } from 'react';
import { Select as SigridSelect } from '@acaprino/sigrid';
import { cn } from './cn';

export type SelectItem = { value: string; label: string; disabled?: boolean };

export type SelectProps = {
  value: string;
  onChange?: (value: string) => void;
  items: SelectItem[];
  label?: string;
  /** Shown, muted, while `value` is '' (nothing chosen yet). */
  placeholder?: string;
  'aria-label'?: string;
  'aria-required'?: 'true' | 'false' | boolean;
  'aria-invalid'?: 'true' | 'false' | boolean;
  disabled?: boolean;
  className?: string;
};

export default function Select({ value, onChange, items, label, placeholder, 'aria-label': ariaLabel, 'aria-invalid': ariaInvalid, disabled = false, className }: SelectProps) {
  const id = useId();
  const select = (
    <SigridSelect
      id={id}
      options={items.map(({ value: itemValue, label: itemLabel }) => ({ value: itemValue, label: itemLabel }))}
      value={value}
      onValueChange={(next) => onChange?.(next)}
      aria-label={label ? undefined : ariaLabel}
      aria-invalid={ariaInvalid}
      placeholder={placeholder}
      disabled={disabled}
      className={label ? undefined : className}
    />
  );
  if (!label) return select;
  return (
    <div className={cn('flex flex-col gap-1.5', className)}>
      <label htmlFor={id} className="text-sm font-medium text-ink">
        {label}
      </label>
      {select}
    </div>
  );
}
