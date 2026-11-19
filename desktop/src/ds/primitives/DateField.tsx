// A date field: a text box that takes a typed date and a button that opens a calendar. `value` is the
// string 'YYYY-MM-DD', or '' for no date; it is the only thing that crosses the boundary. The text shows
// the date the way `locale` writes it (day first for it-IT, month first for en-US); typing accepts that
// form and the ISO form. The typed text is read on Enter and when the focus leaves: a real date calls
// `onChange` (pulled into `min`..`max` when outside), empty text clears it, and anything else
// (2026-13-40) is refused and the box goes back to the last good date; while it is wrong the box is
// aria-invalid. `disabled` locks the text box and the calendar button.
// The calendar is not moved to a portal: it stays inside the field, so it is part of the Modal or
// SidePanel that holds it (focus trap, Esc), and `fixed` keeps a scrolling modal body from clipping it.
// `translations` (Ark's own shape) names the calendar buttons in the app's language.
// The look is in ds/styles/ark/date-picker.css.
import { DatePicker, type DatePickerRootProps } from '@ark-ui/react/date-picker';
import { CalendarDate, parseDate, type DateValue } from '@internationalized/date';
import { useMemo, useState, type KeyboardEvent } from 'react';
import { imeComposing } from '../ime';
import Icon from './Icon';

export type DateFieldProps = {
  value: string;
  onChange?: (value: string) => void;
  'aria-label': string;
  min?: string;
  max?: string;
  locale?: string;
  disabled?: boolean;
  translations?: DatePickerRootProps['translations'];
};

// 'YYYY-MM-DD' to a date; '' and anything that is not a real day give nothing.
function fromIso(text: string | undefined): CalendarDate | undefined {
  if (!text) return undefined;
  try {
    return parseDate(text);
  } catch {
    return undefined;
  }
}

// The order of day, month and year in the locale, and the separators between them.
function layout(where: string) {
  return new Intl.DateTimeFormat(where, { day: 'numeric', month: 'numeric', year: 'numeric' }).formatToParts(
    new Date(2000, 11, 25),
  );
}

// Typed text to a date: three numbers, the year with four digits. A first number of four digits is
// the ISO order (year, month, day); otherwise the order is the locale's. Impossible days are refused.
function fromText(text: string, locale: string): CalendarDate | undefined {
  const found = /^\D*(\d+)\D+(\d+)\D+(\d+)\D*$/.exec(text.trim());
  if (!found) return undefined;
  const numbers = found.slice(1, 4);
  const order =
    numbers[0].length === 4
      ? ['year', 'month', 'day']
      : layout(locale)
          .map((part) => part.type as string)
          .filter((type) => type === 'day' || type === 'month' || type === 'year');
  const parts: Record<string, string> = {};
  order.forEach((type, index) => (parts[type] = numbers[index]));
  if (parts.year?.length !== 4) return undefined;
  const year = Number(parts.year);
  const month = Number(parts.month);
  const day = Number(parts.day);
  // CalendarDate pulls a day that does not exist into the month: a changed number means it was not real.
  const date = new CalendarDate(year, month, day);
  return date.year === year && date.month === month && date.day === day ? date : undefined;
}

function write(date: DateValue | undefined, locale: string): string {
  if (!date) return '';
  return new Intl.DateTimeFormat(locale, {
    timeZone: 'UTC',
    day: '2-digit',
    month: '2-digit',
    year: 'numeric',
  }).format(date.toDate('UTC'));
}

function ViewControl() {
  return (
    <DatePicker.ViewControl>
      <DatePicker.PrevTrigger>
        <Icon name="chevron" size={14} />
      </DatePicker.PrevTrigger>
      <DatePicker.ViewTrigger>
        <DatePicker.RangeText />
      </DatePicker.ViewTrigger>
      <DatePicker.NextTrigger>
        <Icon name="chevron" size={14} />
      </DatePicker.NextTrigger>
    </DatePicker.ViewControl>
  );
}

export default function DateField({
  value,
  onChange,
  'aria-label': ariaLabel,
  min,
  max,
  locale = 'en-US',
  translations,
  disabled = false,
}: DateFieldProps) {
  const minDate = useMemo(() => fromIso(min), [min]);
  const maxDate = useMemo(() => fromIso(max), [max]);
  const dates = useMemo(() => {
    const date = fromIso(value);
    return date ? [date] : [];
  }, [value]);
  const shown = write(dates[0], locale);
  const words = translations?.placeholder?.(locale);
  const placeholder = layout(locale)
    .map((part) =>
      part.type === 'day'
        ? (words?.day ?? 'dd')
        : part.type === 'month'
          ? (words?.month ?? 'mm')
          : part.type === 'year'
            ? (words?.year ?? 'yyyy')
            : part.value,
    )
    .join('');
  // The text while it is being typed; undefined when the box just shows `value`.
  const [draft, setDraft] = useState<string | undefined>();
  const invalid = draft !== undefined && draft.trim() !== '' && !fromText(draft, locale);

  function commit() {
    if (draft === undefined) return;
    const text = draft.trim();
    setDraft(undefined);
    if (text === '') {
      onChange?.('');
      return;
    }
    let date = fromText(text, locale);
    if (!date) return;
    if (minDate && date.compare(minDate) < 0) date = minDate;
    if (maxDate && date.compare(maxDate) > 0) date = maxDate;
    onChange?.(date.toString());
  }
  function pressed(event: KeyboardEvent<HTMLInputElement>) {
    if (event.key !== 'Enter' || imeComposing(event.nativeEvent)) return;
    event.preventDefault();
    commit();
  }

  return (
    <DatePicker.Root
      value={dates}
      onValueChange={(details) => {
        setDraft(undefined);
        onChange?.(details.value[0]?.toString() ?? '');
      }}
      min={minDate}
      max={maxDate}
      locale={locale}
      translations={translations}
      disabled={disabled}
      positioning={{ placement: 'bottom-start', gutter: 4, strategy: 'fixed' }}
    >
      <DatePicker.Control>
        <input
          data-scope="date-picker"
          data-part="input"
          type="text"
          autoComplete="off"
          aria-label={ariaLabel}
          aria-invalid={invalid ? 'true' : undefined}
          disabled={disabled}
          placeholder={placeholder}
          value={draft ?? shown}
          onChange={(event) => setDraft(event.currentTarget.value)}
          onKeyDown={pressed}
          onBlur={commit}
        />
        <DatePicker.Trigger>
          <Icon name="calendar" size={16} />
        </DatePicker.Trigger>
      </DatePicker.Control>
      <DatePicker.Positioner>
        <DatePicker.Content>
          <DatePicker.View view="day">
            <DatePicker.Context>
              {(datePicker) => (
                <>
                  <ViewControl />
                  <DatePicker.Table>
                    <DatePicker.TableHead>
                      <DatePicker.TableRow>
                        {datePicker.weekDays.map((weekDay) => (
                          <DatePicker.TableHeader key={weekDay.short}>{weekDay.narrow}</DatePicker.TableHeader>
                        ))}
                      </DatePicker.TableRow>
                    </DatePicker.TableHead>
                    <DatePicker.TableBody>
                      {datePicker.weeks.map((week, index) => (
                        <DatePicker.TableRow key={index}>
                          {week.map((day) => (
                            <DatePicker.TableCell key={day.toString()} value={day} visibleRange={datePicker.visibleRange}>
                              <DatePicker.TableCellTrigger>{day.day}</DatePicker.TableCellTrigger>
                            </DatePicker.TableCell>
                          ))}
                        </DatePicker.TableRow>
                      ))}
                    </DatePicker.TableBody>
                  </DatePicker.Table>
                </>
              )}
            </DatePicker.Context>
          </DatePicker.View>
          <DatePicker.View view="month">
            <DatePicker.Context>
              {(datePicker) => (
                <>
                  <ViewControl />
                  <DatePicker.Table>
                    <DatePicker.TableBody>
                      {datePicker.getMonthsGrid({ columns: 4, format: 'short' }).map((months, index) => (
                        <DatePicker.TableRow key={index}>
                          {months.map((month) => (
                            <DatePicker.TableCell key={month.value} value={month.value} columns={4}>
                              <DatePicker.TableCellTrigger>{month.label}</DatePicker.TableCellTrigger>
                            </DatePicker.TableCell>
                          ))}
                        </DatePicker.TableRow>
                      ))}
                    </DatePicker.TableBody>
                  </DatePicker.Table>
                </>
              )}
            </DatePicker.Context>
          </DatePicker.View>
          <DatePicker.View view="year">
            <DatePicker.Context>
              {(datePicker) => (
                <>
                  <ViewControl />
                  <DatePicker.Table>
                    <DatePicker.TableBody>
                      {datePicker.getYearsGrid({ columns: 4 }).map((years, index) => (
                        <DatePicker.TableRow key={index}>
                          {years.map((year) => (
                            <DatePicker.TableCell key={year.value} value={year.value} columns={4}>
                              <DatePicker.TableCellTrigger>{year.label}</DatePicker.TableCellTrigger>
                            </DatePicker.TableCell>
                          ))}
                        </DatePicker.TableRow>
                      ))}
                    </DatePicker.TableBody>
                  </DatePicker.Table>
                </>
              )}
            </DatePicker.Context>
          </DatePicker.View>
        </DatePicker.Content>
      </DatePicker.Positioner>
    </DatePicker.Root>
  );
}
