// Shared mutable state that React components can read (the role Svelte's module-level `$state`
// had). `reactive(object)` returns a deep proxy: reading works as on the plain object, and any
// write or delete, at any depth, bumps a version and tells the subscribers. `useStore(store)`
// re-renders a component whenever the store changes. Arrays and plain objects are wrapped; other
// values (Dates, Maps, DOM nodes) are stored as they are and must be replaced, not mutated.
import { useState, useSyncExternalStore } from 'react';

type Listener = () => void;
interface Meta {
  version: number;
  listeners: Set<Listener>;
  /** This store's proxy of each raw object: an object held by two stores has one proxy in each. */
  proxies: WeakMap<object, object>;
}

const metaOf = new WeakMap<object, Meta>();

const plain = (value: unknown): value is object =>
  !!value &&
  typeof value === 'object' &&
  (Array.isArray(value) || Object.getPrototypeOf(value) === Object.prototype || Object.getPrototypeOf(value) === null);

function wrap<T extends object>(target: T, meta: Meta): T {
  const known = meta.proxies.get(target);
  if (known) return known as T;
  const proxy = new Proxy(target, {
    get(object, key, receiver) {
      const value = Reflect.get(object, key, receiver);
      return plain(value) ? wrap(value, meta) : value;
    },
    set(object, key, value, receiver) {
      const before = Reflect.get(object, key, receiver);
      const raw = plain(value) && rawOf.has(value) ? rawOf.get(value)! : value;
      const done = Reflect.set(object, key, raw, receiver);
      if (before !== raw || (Array.isArray(object) && key === 'length')) notify(meta);
      return done;
    },
    deleteProperty(object, key) {
      const had = Reflect.has(object, key);
      const done = Reflect.deleteProperty(object, key);
      if (had) notify(meta);
      return done;
    },
  });
  meta.proxies.set(target, proxy);
  rawOf.set(proxy, target);
  return proxy;
}

const rawOf = new WeakMap<object, object>();

function notify(meta: Meta) {
  meta.version += 1;
  for (const listener of [...meta.listeners]) listener();
}

/** A deep-reactive copy-free wrapper around `initial`. */
export function reactive<T extends object>(initial: T): T {
  const meta: Meta = { version: 0, listeners: new Set(), proxies: new WeakMap() };
  // A proxy of another store is unwrapped first, so the new store tracks the raw object itself.
  const raw = (rawOf.get(initial) as T | undefined) ?? initial;
  const proxy = wrap(raw, meta);
  metaOf.set(proxy, meta);
  return proxy;
}

/** Calls `listener` after every change of the store; returns the unsubscribe function. */
export function subscribe(store: object, listener: Listener): () => void {
  const meta = metaOf.get(store);
  if (!meta) throw new Error('subscribe: not a reactive() store');
  meta.listeners.add(listener);
  return () => meta.listeners.delete(listener);
}

/** The store's change counter: moves on every write. */
export function versionOf(store: object): number {
  return metaOf.get(store)?.version ?? 0;
}

/** Re-renders the calling component on every change of `store`, and returns the store. */
export function useStore<T extends object>(store: T): T {
  useSyncExternalStore(
    (listener) => subscribe(store, listener),
    () => versionOf(store),
    () => versionOf(store),
  );
  return store;
}

/**
 * Local deep-reactive state for a component (the role of Svelte's `let x = $state({...})` objects):
 * mutate it in place (`form.title = 'x'`, `list.push(y)`, `delete map[k]`) and the component re-renders.
 * Reads are always current, also after an `await` or inside a timer, unlike a useState snapshot.
 */
export function useReactive<T extends object>(init: () => T): T {
  const [store] = useState(() => reactive(init()));
  useStore(store);
  return store;
}
