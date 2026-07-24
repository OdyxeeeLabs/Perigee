import { useRef, useSyncExternalStore } from "react";

type Listener = () => void;

export interface StoreApi<T> {
  getState: () => T;
  setState: (partial: Partial<T> | ((state: T) => Partial<T>)) => void;
  subscribe: (listener: Listener) => () => void;
}

export function createStore<T extends object>(
  initializer: (
    setState: StoreApi<T>["setState"],
    getState: StoreApi<T>["getState"],
  ) => T,
): StoreApi<T> {
  let state: T;
  const listeners = new Set<Listener>();

  const getState: StoreApi<T>["getState"] = () => state;

  const setState: StoreApi<T>["setState"] = (partial) => {
    const partialState =
      typeof partial === "function"
        ? (partial as (state: T) => Partial<T>)(state)
        : partial;
    state = { ...state, ...partialState };
    listeners.forEach((listener) => listener());
  };

  const subscribe: StoreApi<T>["subscribe"] = (listener) => {
    listeners.add(listener);
    return () => listeners.delete(listener);
  };

  state = initializer(setState, getState);

  return { getState, setState, subscribe };
}

/** Shallow-compares selector results so object/array selectors only re-render on real changes. */
export function shallow<T>(a: T, b: T): boolean {
  if (Object.is(a, b)) return true;
  if (typeof a !== "object" || a === null || typeof b !== "object" || b === null) {
    return false;
  }
  const keysA = Object.keys(a as object);
  const keysB = Object.keys(b as object);
  if (keysA.length !== keysB.length) return false;
  return keysA.every((key) =>
    Object.is((a as Record<string, unknown>)[key], (b as Record<string, unknown>)[key]),
  );
}

export function createUseStore<T extends object>(store: StoreApi<T>) {
  return function useStore<U>(
    selector: (state: T) => U,
    equalityFn: (a: U, b: U) => boolean = Object.is,
  ): U {
    const cached = useRef<U>();
    const hasCached = useRef(false);

    const getSnapshot = () => {
      const next = selector(store.getState());
      if (hasCached.current && equalityFn(cached.current as U, next)) {
        return cached.current as U;
      }
      cached.current = next;
      hasCached.current = true;
      return next;
    };

    return useSyncExternalStore(store.subscribe, getSnapshot, getSnapshot);
  };
}
