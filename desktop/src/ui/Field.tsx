// A labelled form control: the label text and the control are its children, and a click on the text focuses the control.
// The label above the control is Sigrid's form: 14 px medium text, 6 px above the control. `field` stays on the node for
// the pages' own rules, which place a field by it. Sigrid's `FormField` takes the label, the help and the error as
// props around one control; this one keeps the markup the app already has.
import type { LabelHTMLAttributes } from 'react';
import { cn } from '../ds/cn';

export default function Field({ className, children, ...rest }: LabelHTMLAttributes<HTMLLabelElement>) {
  return (
    <label className={cn('field flex flex-col gap-1.5 text-sm font-medium text-ink', className)} {...rest}>
      {children}
    </label>
  );
}
