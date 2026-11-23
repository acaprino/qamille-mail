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
  useCallback,
  useContext,
  useLayoutEffect,
  useRef,
  type ComponentPropsWithoutRef,
  type ElementType,
  type ReactNode,
  type Ref,
  type RefObject,
} from 'react';
import { preloadMotion, useMotion } from '@acaprino/sigrid';

/**
 * Loads Sigrid's chunk with Motion before the first render, so the lists do not change from plain elements to moving ones (and
 * remount) when it arrives. It never rejects and waits at most `ms`: with no chunk the app draws without animations.
 */
export async function startMotion(ms = 600): Promise<void> {
  await Promise.race([preloadMotion(), new Promise<void>((done) => setTimeout(done, ms))]);
}

// Nothing moves while the app is loading: what mounts in the first moments is simply there (Sigrid starts no motion on load).
const STARTED = typeof performance === 'undefined' ? 0 : performance.now();
const LOADING_MS = 1500;
const loaded = () => typeof performance !== 'undefined' && performance.now() - STARTED > LOADING_MS;

/**
 * The `ref` of an element that comes into the page because the user did something (a state of a meeting, a card): it
 * settles into place once, coming down a few pixels as it fades in (Sigrid's `landCard`). Not while the app is loading.
 */
export function useAppear(): (element: HTMLElement | null) => void {
  const motion = useMotion();
  const seen = useRef<HTMLElement | null>(null);
  return useCallback(
    (element: HTMLElement | null) => {
      if (!element || element === seen.current) return;
      seen.current = element;
      if (motion && loaded()) motion.landCard(element);
    },
    [motion],
  );
}

/**
 * The same entrance on an element that stays: each time `key` changes (never on the first render, never while loading) and
 * `when` holds, it settles into place again: the view that has just changed, a panel that has just opened.
 */
export function useLand(ref: RefObject<HTMLElement | null>, key: unknown, when = true): void {
  const motion = useMotion();
  const first = useRef(true);
  useLayoutEffect(() => {
    if (first.current) {
      first.current = false;
      return;
    }
    if (motion && when && loaded() && ref.current) motion.landCard(ref.current);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);
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
  /** It leaves by closing up (a row that is done), not only fading (a card). A row comes down into its place, but does not make room (no layout). */
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
    ? {
        initial: enter ? motion.LIST_ITEM.initial : false,
        animate: motion.LIST_ITEM.animate,
        exit: { ...motion.ROW_EXIT.exit, overflow: 'hidden', transition: motion.ROW_EXIT.transition },
        transition: motion.MOTION.enter,
      }
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
