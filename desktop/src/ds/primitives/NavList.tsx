// A list of navigation rows in the manner of unlumen's Sidebar 001 (ui.unlumen.com; its two effects, the file is not copied): a soft
// highlight follows the pointer from row to row on a spring, and the current row carries a short accent
// bar that slides to the next current row (a shared motion layoutId). The rows are <button>s; `hint`
// turns a row into a Tooltip trigger (the collapsed sidebar, where a row is only an icon).
import { AnimatePresence, motion, useReducedMotion } from 'motion/react';
import {
  createContext,
  useCallback,
  useContext,
  useId,
  useRef,
  useState,
  type ButtonHTMLAttributes,
  type HTMLAttributes,
  type RefObject,
} from 'react';
import { cn } from '../cn';
import Tooltip from './Tooltip';

type Rect = { top: number; height: number; left: number; width: number };
type Hover = {
  container: RefObject<HTMLElement | null>;
  hovered: string | null;
  show: (id: string | null, node?: HTMLElement | null) => void;
  group: string;
};
const HoverContext = createContext<Hover | null>(null);

/** The bar of the current row slides between all NavLists that share `group` (default: one per app). */
export function NavList({
  className,
  children,
  group = 'nav',
  ...rest
}: HTMLAttributes<HTMLElement> & { group?: string }) {
  const container = useRef<HTMLElement>(null);
  const [hovered, setHovered] = useState<string | null>(null);
  const [rect, setRect] = useState<Rect | null>(null);
  const still = useReducedMotion();
  const show = useCallback((id: string | null, node?: HTMLElement | null) => {
    setHovered(id);
    const box = container.current;
    if (!id || !node || !box) return setRect(null);
    const a = node.getBoundingClientRect();
    const b = box.getBoundingClientRect();
    setRect({ top: a.top - b.top, height: a.height, left: a.left - b.left, width: a.width });
  }, []);
  return (
    <HoverContext.Provider value={{ container, hovered, show, group }}>
      <nav ref={container} className={cn('nav-list', className)} {...rest}>
        <AnimatePresence>
          {!still && hovered && rect && (
            <motion.span
              key="hover"
              aria-hidden="true"
              className="nav-hover"
              initial={{ opacity: 0, top: rect.top, height: rect.height, left: rect.left, width: rect.width }}
              animate={{ opacity: 1, top: rect.top, height: rect.height, left: rect.left, width: rect.width }}
              exit={{ opacity: 0 }}
              transition={{ type: 'spring', stiffness: 380, damping: 32 }}
            />
          )}
        </AnimatePresence>
        {children}
      </nav>
    </HoverContext.Provider>
  );
}

export type NavItemProps = ButtonHTMLAttributes<HTMLButtonElement> & {
  /** The current row: draws the sliding accent bar. */
  active?: boolean;
  /** A tooltip text: the row becomes its trigger. */
  hint?: string;
};

export function NavItem({ active = false, hint, className, children, onMouseEnter, onMouseLeave, ...rest }: NavItemProps) {
  const hover = useContext(HoverContext);
  const id = useId();
  const button = (
    <button
      className={cn(className, active && 'active')}
      data-nav-item=""
      onMouseEnter={(event) => {
        hover?.show(id, event.currentTarget);
        onMouseEnter?.(event);
      }}
      onMouseLeave={(event) => {
        hover?.show(null);
        onMouseLeave?.(event);
      }}
      {...rest}
    >
      {active && (
        <motion.span
          aria-hidden="true"
          className="nav-active-bar"
          layoutId={`nav-active-${hover?.group ?? 'nav'}`}
          transition={{ type: 'spring', stiffness: 800, damping: 40 }}
        />
      )}
      {children}
    </button>
  );
  return hint ? <Tooltip text={hint}>{button}</Tooltip> : button;
}
