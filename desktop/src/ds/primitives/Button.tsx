// The app's button. The look lives in ds/styles/system.css (the Compatto buttons);
// `className` adds page-level classes and `ref` hands out the DOM node. `tip` (with an optional
// `shortcut`) makes the button the trigger of a Tooltip: a hint, never the name, so an icon button
// still needs its aria-label. Without `tip` the markup is the plain <button>.
import type { ButtonHTMLAttributes, Ref } from 'react';
import { cn } from '../cn';
import Tooltip from './Tooltip';

export type ButtonVariant = 'primary' | 'secondary' | 'icon' | 'text';

const variants: Record<ButtonVariant, string> = {
  primary: 'primary',
  secondary: 'secondary',
  icon: 'icon-button',
  text: 'text-button',
};

export type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  variant: ButtonVariant;
  tip?: string;
  shortcut?: string;
  ref?: Ref<HTMLButtonElement>;
};

export default function Button({ variant, className, tip, shortcut, ref, children, ...rest }: ButtonProps) {
  const button = (
    <button ref={ref} className={cn(variants[variant], className)} {...rest}>
      {children}
    </button>
  );
  return tip ? (
    <Tooltip text={tip} shortcut={shortcut}>
      {button}
    </Tooltip>
  ) : (
    button
  );
}
