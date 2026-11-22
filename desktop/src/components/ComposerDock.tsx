import { AnimatePresence } from 'motion/react';
import { useEffect, useRef, useState } from 'react';
import Button from '../ds/primitives/Button';
import Icon from '../ds/primitives/Icon';
import DraftTabs from '../ui/DraftTabs';
import type { TabItem } from '../ui/Tabs';
import { desktop, errorText } from '../lib/api';
import type { ComposerHandle, ComposerState } from '../lib/composer';
import { tr } from '../lib/i18n';
import { useReactive } from '../lib/store';
import { tick } from '../lib/tick';
import type { Draft, UndoAction, Workspace } from '../lib/types';
import { cn } from '../lib/utils';
import { registerCloseGuard } from '../lib/window-close';
import Composer, { saveStatusLabel } from './Composer';

export type ComposerDockProps = {
  drafts: Draft[];
  activeId: number | null;
  onActiveIdChange?: (value: number | null) => void;
  windowClosing?: boolean;
  onWindowClosingChange?: (value: boolean) => void;
  workspace: Workspace;
  close: (id: number) => void;
  compose: () => void;
  changed: () => void;
  notify: (message: string, error?: boolean) => void;
  queued: (action: UndoAction) => void;
  copied: (id: number, draft: Draft) => void;
};

export default function ComposerDock(props: ComposerDockProps) {
  const {
    drafts,
    activeId,
    windowClosing: windowClosingProp,
    workspace,
    close,
    compose,
    changed,
    notify,
    queued,
    copied,
  } = props;
  // Without a parent that controls it, the window-closing flag is the dock's own.
  const [ownClosing, setOwnClosing] = useState(false);
  const windowClosing = windowClosingProp ?? ownClosing;
  // The props and the two bound values as they are now, for the close guard (registered once) and
  // after awaits: a write shows here at once, before the parent renders again.
  const latest = useRef({ props, windowClosing, activeId });
  latest.current = { props, windowClosing, activeId };
  const setWindowClosing = (value: boolean) => {
    latest.current.windowClosing = value;
    if (latest.current.props.windowClosing === undefined) setOwnClosing(value);
    latest.current.props.onWindowClosingChange?.(value);
  };
  const setActiveId = (value: number | null) => {
    latest.current.activeId = value;
    latest.current.props.onActiveIdChange?.(value);
  };

  const handles = useRef(new Map<number, ComposerHandle>()).current;
  const states = useReactive(() => ({}) as Record<number, ComposerState>);
  // A closed composer fades out for a moment. A draft that comes back meanwhile (Undo) must get a new
  // composer, not that one: each time a draft id appears it gets a new generation, part of the key.
  const generations = useRef({ of: new Map<number, number>(), shown: new Set<number>() }).current;
  for (const draft of drafts)
    if (!generations.shown.has(draft.id))
      generations.of.set(draft.id, (generations.of.get(draft.id) ?? 0) + 1);
  generations.shown = new Set(drafts.map((draft) => draft.id));
  const generationOf = (id: number) => generations.of.get(id) ?? 0;
  const [focusedTab, setFocusedTab] = useState<number | null>(null);
  const title = (draft: Draft) =>
    (states[draft.id]?.subject ?? draft.subject).trim() || tr('Nuovo messaggio', 'New message');
  // Drafts with the same title get a number so their tabs stay distinguishable.
  const tabTitle = (draft: Draft) => {
    const same = drafts.filter((item) => title(item) === title(draft));
    return same.length > 1 ? `${title(draft)} ${same.indexOf(draft) + 1}` : title(draft);
  };

  function register(id: number, handle: ComposerHandle) {
    handles.set(id, handle);
    return () => {
      // A newer composer for the same draft may have registered meanwhile (a closed one leaves last).
      if (handles.get(id) !== handle) return;
      handles.delete(id);
      delete states[id];
    };
  }
  // Only the composer of the draft's current generation speaks for its tab.
  function stateChanged(id: number, generation: number, state: ComposerState) {
    if (generationOf(id) === generation) states[id] = state;
  }
  async function minimize(id: number) {
    setActiveId(null);
    void handles.get(id)?.flush();
    await tick();
    document.getElementById(`compose-tab-${id}`)?.focus();
  }
  function select(id: number) {
    const current = latest.current.activeId;
    if (current !== null && current !== id) void handles.get(current)?.flush();
    setActiveId(id);
  }
  // A click on the open draft's tab minimizes it; on another tab it opens that one.
  function pick(value: string) {
    const id = Number(value);
    if (latest.current.activeId === id) void minimize(id);
    else select(id);
  }
  // Only one tab is in the tab order: the one that had the focus, else the open one, else the first.
  const tabbable = drafts.some((item) => item.id === focusedTab)
    ? focusedTab
    : (activeId ?? drafts[0]?.id);
  const tabItems: TabItem[] = drafts.map((draft) => {
    const state = states[draft.id];
    return {
      value: String(draft.id),
      label: tabTitle(draft),
      className: cn(
        'composer-tab',
        activeId === draft.id && 'active',
        state?.status === 'Could not save' && 'save-error',
      ),
      props: {
        'aria-label': tabTitle(draft),
        'aria-expanded': activeId === draft.id,
        tabIndex: tabbable === draft.id ? 0 : -1,
        onFocus: () => setFocusedTab(draft.id),
        title: `${tabTitle(draft)}${state?.recipient ? ` — ${state.recipient}` : ''} · ${
          state?.status
            ? saveStatusLabel(state.status)
            : tr('Apertura dell’editor…', 'Opening editor…')
        }`,
      },
    };
  });

  useEffect(() => {
    if (!desktop) return;
    return registerCloseGuard('composers', async () => {
      const now = () => latest.current.props.drafts;
      if (latest.current.windowClosing) return false;
      if (now().some((draft) => !handles.has(draft.id) || handles.get(draft.id)!.isBusy())) {
        latest.current.props.notify(
          tr(
            'Attendi la fine dell’operazione sulla bozza prima di chiudere.',
            'Wait for the current draft operation to finish before closing.',
          ),
        );
        return false;
      }
      setWindowClosing(true);
      try {
        await tick();
        const ids = now().map((draft) => draft.id);
        const results = await Promise.all(ids.map((id) => handles.get(id)!.flush()));
        const failed = results.findIndex((saved) => !saved);
        if (failed !== -1) {
          setActiveId(ids[failed]);
          return false;
        }
        const after = now();
        if (after.length !== ids.length || after.some((draft) => !ids.includes(draft.id)))
          return false;
        return true;
      } catch (error) {
        latest.current.props.notify(errorText(error), true);
        return false;
      } finally {
        setWindowClosing(false);
      }
    }, {
      // Session end: every open draft is written now, not 1.2 s after the last key. A draft that
      // fails to save answers false; one with no editor open yet has nothing to save.
      sessionEnd: async () =>
        (await Promise.all(latest.current.props.drafts.map((draft) => handles.get(draft.id)?.flush()))).every(
          (ok) => ok !== false,
        ),
    });
  }, []);

  const find = (value: string) => drafts.find((d) => String(d.id) === value);

  return (
    <>
      <AnimatePresence>
        {drafts.map((draft) => {
          const generation = generationOf(draft.id);
          return (
            <Composer
              key={`${draft.id}:${generation}`}
              initial={draft}
              workspace={workspace}
              active={activeId === draft.id}
              windowClosing={windowClosing}
              close={() => close(draft.id)}
              minimize={() => void minimize(draft.id)}
              register={(handle) => register(draft.id, handle)}
              stateChanged={(state) => stateChanged(draft.id, generation, state)}
              changed={changed}
              notify={notify}
              queued={queued}
              copied={(saved) => copied(draft.id, saved)}
            />
          );
        })}
      </AnimatePresence>

      <footer className="composer-dock" aria-label={tr('Bozze aperte', 'Open drafts')}>
        {/* The drafts open here, not all the saved ones the sidebar counts: the two numbers say different things. */}
        <span className="dock-label">
          <Icon name="edit" size={15} />
          <span>{tr('Bozze aperte', 'Open drafts')}</span>
          <strong>{drafts.length}</strong>
        </span>
        <div className="composer-tabs">
          <DraftTabs
            value={activeId === null ? '' : String(activeId)}
            items={tabItems}
            aria-label={tr('Schede delle bozze', 'Draft tabs')}
            ids={{ trigger: (id) => `compose-tab-${id}`, content: (id) => `compose-panel-${id}` }}
            onPick={pick}
            tab={(item) => {
              const draft = find(item.value);
              const state = draft ? states[draft.id] : undefined;
              return (
                <>
                  <Icon name={activeId === draft?.id ? 'edit' : 'mail'} size={14} />
                  <span title={item.label}>{item.label}</span>
                  {state?.dirty && (
                    <span className="draft-dirty" aria-label={saveStatusLabel(state.status)}></span>
                  )}
                </>
              );
            }}
            after={(item) => {
              const draft = find(item.value);
              const state = draft ? states[draft.id] : undefined;
              return (
                <button
                  className="tab-close"
                  aria-label={`${tr('Salva e chiudi', 'Save and close')} ${item.label}`}
                  title={tr('Salva e chiudi', 'Save and close')}
                  disabled={!state || state.busy || windowClosing}
                  onClick={() => draft && void handles.get(draft.id)?.close()}
                >
                  <Icon name="close" size={13} />
                </button>
              );
            }}
          />
        </div>
        <Button
          variant="icon"
          className="dock-new"
          aria-label={tr('Nuova scheda bozza', 'New draft tab')}
          tip={tr('Nuovo messaggio', 'New message')}
          shortcut="N"
          aria-keyshortcuts="N"
          onClick={compose}
        >
          <Icon name="plus" size={17} />
        </Button>
      </footer>
    </>
  );
}
