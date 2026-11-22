// Qamille's movement is Sigrid's. Sigrid's motion (its `useMotion()`) answers a gesture and nothing else: an entry comes
// down a few pixels into its place, what leaves fades, a row that is done closes up, and the others make room; nothing
// starts on load, nothing repeats, nothing is a spring. This file puts those pieces in the shape the app's lists use:
//   `Presence`    what stays in the page for the length of its exit (Sigrid's `Presence`, around the items of a list);
//   `Item`        an element of such a list, as a div, li, section or button: it enters, moves to make room and leaves;
//   `useIsPresent` false while an item is on its way out, so it can let go of focus and take no click.
// With animations off (the system asks for less motion, or Sigrid's chunk has not arrived) every piece is the plain DOM:
// no animation, and the same markup. Qamille keeps no animation of its own and imports no animation library.
import {
  Children,
  Fragment,
  createContext,
  isValidElement,
  useContext,
  type ComponentPropsWithoutRef,
  type ElementType,
  type ReactNode,
  type Ref,
} from 'react';
import { preloadMotion, useMotion } from '@acaprino/sigrid';

/**
 * Loads Sigrid's chunk with Motion before the first render, so the lists do not change from plain elements to moving ones (and
 * remount) when it arrives. It never rejects and waits at most `ms`: with no chunk the app draws without animations.
 */
export async function startMotion(ms = 600): Promise<void> {
  await Promise.race([preloadMotion(), new Promise<void>((done) => setTimeout(done, ms))]);
}

/** True while Sigrid's motion is on: the app animates, the system allows motion and the chunk with Motion is loaded. */
export function useAnimated(): boolean {
  return useMotion() !== null;
}

/** True unless the item this is called from is leaving (inside a `Presence`; anywhere else it is always true). */
const LeavingContext = createContext(false);
export function useIsPresent(): boolean {
  return !useContext(LeavingContext);
}

/** Tells what is under it whether the keyed child of the `Presence` it sits in is leaving. */
function Present({ children }: { children: ReactNode }) {
  const motion = useMotion();
  if (!motion) return <Fragment>{children}</Fragment>;
  return <motion.Leaving>{(leaving) => <LeavingContext.Provider value={leaving}>{children}</LeavingContext.Provider>}</motion.Leaving>;
}

/**
 * The items of a list, kept in the page while they leave. Every keyed child is wrapped so that what is under it can ask
 * `useIsPresent()`. `pop` lifts the leaving item out of the flow at once and the others slide (for an item that forwards
 * its `ref`, as `Item` does).
 */
export function Presence({ children, pop = false }: { children?: ReactNode; pop?: boolean }) {
  const motion = useMotion();
  if (!motion) return <Fragment>{children}</Fragment>;
  return (
    <motion.Presence pop={pop}>
      {Children.toArray(children).map((child) => (isValidElement(child) ? <Present key={child.key}>{child}</Present> : child))}
    </motion.Presence>
  );
}

type Tag = 'div' | 'li' | 'section' | 'button';

export type ItemProps<T extends Tag> = Omit<ComponentPropsWithoutRef<T>, 'children' | 'ref'> & {
  as?: T;
  /** It comes in animated: a new entry. False for what was already there when the list appeared (no animation on load). */
  enter?: boolean;
  /** It leaves by closing up (a row that is done), not only fading (a card). A row appears still. */
  closes?: boolean;
  ref?: Ref<HTMLElement>;
  children?: ReactNode | ((leaving: boolean) => ReactNode);
};

/**
 * An element of a list inside a `Presence`. Without animations it is the plain element. Leaving, it is `inert`, and the
 * children can be a function of `leaving` to draw what a leaving item shows.
 */
export function Item<T extends Tag = 'div'>({ as, enter = true, closes = false, ref, children, ...rest }: ItemProps<T>) {
  const motion = useMotion();
  const tag = (as ?? 'div') as T;
  const Plain = tag as ElementType;
  if (!motion) return <Plain ref={ref} {...rest}>{typeof children === 'function' ? children(false) : children}</Plain>;
  const Moving = motion.m[tag] as ElementType;
  const way = closes
    ? { ...motion.ROW_EXIT, exit: { ...motion.ROW_EXIT.exit, overflow: 'hidden' } }
    : { layout: true, ...motion.LIST_ITEM, initial: enter ? motion.LIST_ITEM.initial : false };
  return (
    <motion.Leaving>
      {(leaving) => (
        <Moving ref={ref} {...way} {...rest} inert={leaving ? true : undefined}>
          {typeof children === 'function' ? children(leaving) : children}
        </Moving>
      )}
    </motion.Leaving>
  );
}
