// A date field: a button with the date written the way `locale` writes it, and a calendar it opens (Sigrid's DateField, on
// Ark's date picker). `value` is the string 'YYYY-MM-DD', or '' for no date; it is the only thing that crosses the
// boundary. The date is chosen in the calendar, not typed (R6 of the plan's asks). `clearable` puts a quiet button beside
// the field that takes the date away, for a date that is optional. The name for screen readers is the `aria-label`, as a
// label the field is tied to. The calendar's words are Sigrid's (ui/words.ts): `translations` is kept so the callers
// need not change, and is not read.
import { useId } from 'react';
import { Button, DateField as SigridDateField } from '@acaprino/sigrid';
import Icon from '../ds/primitives/Icon';
import { tr } from '../lib/i18n';

export type DateFieldProps = {
  value: string;
  onChange?: (value: string) => void;
  'aria-label': string;
  min?: string;
  max?: string;
  locale?: string;
  disabled?: boolean;
  /** An optional date: a button beside the field takes it away. */
  clearable?: boolean;
  /** Not read: the words are Sigrid's. */
  translations?: unknown;
};

export default function DateField({ value, onChange, 'aria-label': ariaLabel, min, max, locale = 'en-US', disabled = false, clearable = false }: DateFieldProps) {
  const id = useId();
  return (
    <span className="date-field-row">
      <label htmlFor={id} className="visually-hidden">
        {ariaLabel}
      </label>
      <SigridDateField id={id} value={value || null} onChange={(next) => onChange?.(next ?? '')} min={min} max={max} locale={locale} disabled={disabled} />
      {clearable && value ? (
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          aria-label={tr('Togli la data', 'Remove the date')}
          disabled={disabled}
          onClick={() => onChange?.('')}
        >
          <Icon name="close" size={14} />
        </Button>
      ) : null}
    </span>
  );
}
