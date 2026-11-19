// A labelled form control: the label text and the control are its children.
import type { LabelHTMLAttributes } from 'react';
import { cn } from '../cn';

export default function Field({ className, children, ...rest }: LabelHTMLAttributes<HTMLLabelElement>) {
  return (
    <label className={cn('field', className)} {...rest}>
      {children}
    </label>
  );
}
