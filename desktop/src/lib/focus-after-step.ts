import { useEffect, useRef, type RefObject } from 'react';

/** When a step of a flow replaces the control that had the focus (Continue, Subscribe, Stop
 *  waiting), the focus would fall to <body> and a keyboard or screen reader user would start over
 *  from the top of the page. After each change of `step`, a focus left on <body> goes to the step's
 *  heading instead (native audit T15). A focus still somewhere on the page is left where it is. */
export function useFocusAfterStep(container: RefObject<HTMLElement | null>, step: unknown) {
  const first = useRef(true);
  useEffect(() => {
    if (first.current) {
      first.current = false;
      return;
    }
    const root = container.current;
    if (!root || (document.activeElement && document.activeElement !== document.body)) return;
    const target = root.querySelector<HTMLElement>('h2, h3') ?? root;
    if (!target.hasAttribute('tabindex')) target.tabIndex = -1;
    target.focus();
  }, [container, step]);
}
