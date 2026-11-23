// A list of navigation rows for the sidebar, drawn by Sigrid's NavList and NavItem on the `sidebar` tone: a row has an icon
// slot, a name and, at its end, whatever the caller adds (a `Count`). The row of the place the user is in carries
// `aria-current="page"`, the selection tint, stronger text and a short bar in the accent at its left edge; under the pointer
// a row takes a lighter share of the tint. `hint` turns a row into a Tooltip trigger (the collapsed sidebar, where a row is
// only an icon). The highlight that followed the pointer and the sliding bar of the old list are gone.
import type { ButtonHTMLAttributes, ReactNode } from 'react';
import { NavItem as SigridNavItem, NavList as SigridNavList } from '@acaprino/sigrid';
import Tooltip from './Tooltip';

export function NavList({ className, children, ...rest }: { 'aria-label': string; className?: string; children?: ReactNode; group?: string }) {
  const { group: _group, ...own } = rest;
  return (
    <SigridNavList tone="sidebar" className={className} {...own}>
      {children}
    </SigridNavList>
  );
}

export type NavItemProps = Omit<ButtonHTMLAttributes<HTMLButtonElement>, 'type'> & {
  /** The current row: the tint and the bar. */
  active?: boolean;
  /** A tooltip text: the row becomes its trigger. */
  hint?: string;
  /** The icon (or the small picture) before the name. */
  icon?: ReactNode;
  /** What follows the name, at the end of the row (a `Count`). */
  end?: ReactNode;
};

export function NavItem({ active = false, hint, icon, end, children, ...rest }: NavItemProps) {
  const row = (
    <SigridNavItem current={active} icon={icon} end={end} {...rest}>
      {children}
    </SigridNavItem>
  );
  return hint ? <Tooltip text={hint}>{row}</Tooltip> : row;
}
