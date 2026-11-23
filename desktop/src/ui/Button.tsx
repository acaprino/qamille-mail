// The app's button, drawn by Sigrid's Button. `variant` says the role: `primary` (the action; with the class `danger`, the
// action that destroys, Sigrid's destructive), `secondary`, `icon` (a quiet icon-only button) and `text` (a quiet text
// button). `className` adds page-level classes and `ref` hands out the DOM node. `tip` (with an optional `shortcut`)
// makes the button the trigger of a Tooltip: a hint, never the name, so an icon button still needs its aria-label.
// The old names of the variants stay on the node (`primary`, `secondary`, `icon-button`, `text-button`): the pages'
// own rules place buttons by them. Sigrid's Button is `type="button"` unless told otherwise, so a button that sends
// a form says `type="submit"`.
import type { ButtonHTMLAttributes, Ref } from 'react';
import { Button as SigridButton } from '@acaprino/sigrid';
import { cn } from './cn';
import Tooltip from '../ui/Tooltip';

export type ButtonVariant = 'primary' | 'secondary' | 'icon' | 'text';

const NAMES: Record<ButtonVariant, string> = {
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
  const danger = /(?:^|\s)danger(?:\s|$)/.test(className ?? '');
  const look = variant === 'primary' ? (danger ? 'destructive' : 'default') : variant === 'secondary' ? 'secondary' : 'ghost';
  const button = (
    <SigridButton
      ref={ref}
      variant={look}
      size={variant === 'icon' ? 'icon' : 'default'}
      className={cn(NAMES[variant], variant === 'text' && danger && 'text-destructive', className)}
      {...rest}
    >
      {children}
    </SigridButton>
  );
  return tip ? (
    <Tooltip text={tip} shortcut={shortcut}>
      {button}
    </Tooltip>
  ) : (
    button
  );
}
