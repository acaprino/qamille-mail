// The app's animations are Sigrid's, and Sigrid's motion is designed for its own parts (a list item that closes up, a notice, a
// card): Qamille's own springs and layout animations came from a copy of Motion that Qamille no longer has. Until each place
// is drawn with Sigrid's `ListItem`, `Presence` and `Leaving` (the Sigrid plan, Task 18), these stand in for the calls of
// that copy: every element is drawn still, in its final place, the way it is drawn when the system asks for reduced motion,
// and what an animation told the component (`useReducedMotion`, `useIsPresent`) says «still» and «here».
import { Fragment, createElement, forwardRef, type ComponentType, type JSX, type ReactNode } from 'react';

/** The props of an animation: they say how to move, and a still element has nothing to move. */
const ANIMATION = new Set(['initial', 'animate', 'exit', 'transition', 'variants', 'custom', 'layout', 'layoutId', 'layoutDependency', 'layoutScroll', 'layoutRoot', 'whileHover', 'whileTap', 'whileFocus', 'whileInView', 'onAnimationComplete', 'onAnimationStart']);

/** What an animated element takes besides the props of the plain one. */
type AnimationProps = {
  initial?: unknown;
  animate?: unknown;
  exit?: unknown;
  transition?: unknown;
  variants?: unknown;
  custom?: unknown;
  layout?: unknown;
  layoutId?: unknown;
  layoutDependency?: unknown;
  layoutScroll?: unknown;
  layoutRoot?: unknown;
  whileHover?: unknown;
  whileTap?: unknown;
  whileFocus?: unknown;
  whileInView?: unknown;
  onAnimationComplete?: (definition: string) => void;
  onAnimationStart?: (definition: string) => void;
};
type MotionTags = { [Tag in keyof JSX.IntrinsicElements]: ComponentType<JSX.IntrinsicElements[Tag] & AnimationProps> };

function still(tag: string): ComponentType<Record<string, unknown>> {
  const Element = forwardRef<unknown, Record<string, unknown>>((props, ref) => {
    const rest = Object.fromEntries(Object.entries(props).filter(([name]) => !ANIMATION.has(name)));
    return createElement(tag, { ...rest, ref });
  });
  Element.displayName = `still.${tag}`;
  return Element as ComponentType<Record<string, unknown>>;
}

const cache = new Map<string, ComponentType<Record<string, unknown>>>();

/** `motion.div`, `motion.li`…: the plain element without the animation props. */
export const motion = new Proxy({} as MotionTags, {
  get: (_target, tag: string) => {
    let element = cache.get(tag);
    if (!element) {
      element = still(tag);
      cache.set(tag, element);
    }
    return element;
  },
});

/** What leaves is gone at once: the children stand as they are. */
export function AnimatePresence({ children }: { children?: ReactNode; initial?: boolean; mode?: string; custom?: unknown }) {
  return <Fragment>{children}</Fragment>;
}

/** Still: every component takes the way it is drawn without animations. */
export const useReducedMotion = () => true;

/** Nothing is leaving: an element is present. */
export const useIsPresent = () => true;

/** The animation targets of a component, by name: unused while everything is still. */
export type Variants = Record<string, unknown>;
