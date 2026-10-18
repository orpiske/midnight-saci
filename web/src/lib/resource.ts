// Loading one API resource into component state, aborted when the
// component goes away or its inputs change.
import { useCallback, useEffect, useRef, useState } from "react";

export type Resource<T> =
  | { state: "loading"; data?: undefined; error?: undefined }
  | { state: "ready"; data: T; error?: undefined }
  | { state: "error"; data?: T; error: unknown };

export interface Loaded<T> {
  resource: Resource<T>;
  /** Fetches again, keeping the current data visible meanwhile. */
  reload: () => Promise<void>;
  /** Replaces the data, for a mutation's authoritative answer. */
  set: (data: T) => void;
}

export function useResource<T>(
  load: (signal: AbortSignal) => Promise<T>,
  deps: readonly unknown[],
): Loaded<T> {
  const [resource, setResource] = useState<Resource<T>>({ state: "loading" });
  const loader = useRef(load);
  loader.current = load;
  const current = useRef<AbortController | null>(null);

  const run = useCallback(async (keep: boolean) => {
    current.current?.abort();
    const controller = new AbortController();
    current.current = controller;
    if (!keep) setResource({ state: "loading" });
    try {
      const data = await loader.current(controller.signal);
      if (!controller.signal.aborted) setResource({ state: "ready", data });
    } catch (error) {
      if (controller.signal.aborted) return;
      setResource((previous) => ({ state: "error", error, data: previous.data }));
    }
  }, []);

  useEffect(() => {
    void run(false);
    return () => current.current?.abort();
  }, deps);

  const reload = useCallback(() => run(true), [run]);
  const set = useCallback((data: T) => setResource({ state: "ready", data }), []);
  return { resource, reload, set };
}

/** Calls `tick` every `ms` while `active`, pausing while the tab is hidden. */
export function useInterval(tick: () => void, ms: number, active: boolean): void {
  const saved = useRef(tick);
  saved.current = tick;
  useEffect(() => {
    if (!active) return;
    const id = window.setInterval(() => {
      if (document.visibilityState === "visible") saved.current();
    }, ms);
    return () => window.clearInterval(id);
  }, [ms, active]);
}
