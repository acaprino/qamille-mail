// A choice from a short list, on Ark Select: a button (role combobox, --control-height high) that
// shows the chosen item's label, and a listbox in a portal on --z-toast, above Modal and SidePanel.
// Inside those the list stays usable: Ark's layer stack counts a click or a key in it as inside the
// dialog. Keyboard: Enter, Space or the arrows open it; arrows move, Enter chooses, Escape closes and
// the focus stays on the button; typing a letter jumps to the item that starts with it. `label` is the
// visible label and the button's name; without it pass `aria-label`. `onChange` is told the new value
// ('' = nothing chosen). The look is in ds/styles/ark/select.css.
import { Portal } from '@ark-ui/react/portal';
import { Select as ArkSelect, createListCollection } from '@ark-ui/react/select';
import { useMemo } from 'react';
import Icon from './Icon';

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

export default function Select({
  value,
  onChange,
  items,
  label,
  placeholder,
  'aria-label': ariaLabel,
  'aria-required': ariaRequired,
  'aria-invalid': ariaInvalid,
  disabled = false,
  className,
}: SelectProps) {
  const collection = useMemo(() => createListCollection({ items }), [items]);
  const name = label ? undefined : ariaLabel;
  return (
    <ArkSelect.Root
      collection={collection}
      disabled={disabled}
      value={value === '' ? [] : [value]}
      onValueChange={(details) => onChange?.(details.value[0] ?? '')}
      positioning={{ placement: 'bottom-start', sameWidth: true, gutter: 4, strategy: 'fixed' }}
      lazyMount
      unmountOnExit
      className={className}
    >
      {label && <ArkSelect.Label>{label}</ArkSelect.Label>}
      <ArkSelect.Control>
        <ArkSelect.Trigger aria-label={name} aria-required={ariaRequired} aria-invalid={ariaInvalid}>
          <ArkSelect.ValueText placeholder={placeholder} />
          <ArkSelect.Indicator>
            <Icon name="down" size={14} />
          </ArkSelect.Indicator>
        </ArkSelect.Trigger>
      </ArkSelect.Control>
      <Portal>
        <ArkSelect.Positioner>
          <ArkSelect.Content aria-label={name}>
            {items.map((item) => (
              <ArkSelect.Item key={item.value} item={item}>
                <ArkSelect.ItemText>{item.label}</ArkSelect.ItemText>
                <ArkSelect.ItemIndicator>
                  <Icon name="check" size={14} />
                </ArkSelect.ItemIndicator>
              </ArkSelect.Item>
            ))}
          </ArkSelect.Content>
        </ArkSelect.Positioner>
      </Portal>
    </ArkSelect.Root>
  );
}
