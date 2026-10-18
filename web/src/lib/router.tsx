// A small history router for the console's few routes. Every route lives
// under /ui/, and the backend serves the client for any of them, so a
// deep link reloads to the same screen.
import {
  type AnchorHTMLAttributes,
  type MouseEvent,
  type ReactNode,
  forwardRef,
  useSyncExternalStore,
} from "react";

export const UI_BASE = "/ui";

type Blocker = () => string | null;

const listeners = new Set<() => void>();
const blockers = new Set<Blocker>();

function emit(): void {
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  window.addEventListener("popstate", listener);
  return () => {
    listeners.delete(listener);
    window.removeEventListener("popstate", listener);
  };
}

const snapshot = (): string => window.location.pathname + window.location.search;

export interface Location {
  /** The path under /ui, starting with `/`. */
  path: string;
  search: URLSearchParams;
}

function parse(href: string): Location {
  const url = new URL(href, window.location.origin);
  let path = url.pathname.startsWith(UI_BASE) ? url.pathname.slice(UI_BASE.length) : url.pathname;
  if (path === "") path = "/";
  return { path, search: url.searchParams };
}

export function useLocation(): Location {
  const href = useSyncExternalStore(subscribe, snapshot, snapshot);
  return parse(href);
}

/** Registers a check run before in-app navigation; it returns a warning to
 * confirm, or null to allow leaving. */
export function addBlocker(blocker: Blocker): () => void {
  blockers.add(blocker);
  return () => blockers.delete(blocker);
}

function allowed(): boolean {
  for (const blocker of blockers) {
    const warning = blocker();
    if (warning && !window.confirm(warning)) return false;
  }
  return true;
}

/** Navigates to a console path (`/runs/…?tab=…`), relative to /ui. */
export function navigate(to: string, options: { replace?: boolean } = {}): void {
  const href = `${UI_BASE}${to}`;
  if (href === snapshot()) return;
  if (!options.replace && !allowed()) return;
  if (options.replace) window.history.replaceState(null, "", href);
  else window.history.pushState(null, "", href);
  emit();
}

/** Updates query parameters of the current path without a history entry. */
export function setSearch(changes: Record<string, string | null>, push = false): void {
  const url = new URL(window.location.href);
  for (const [key, value] of Object.entries(changes)) {
    if (value === null) url.searchParams.delete(key);
    else url.searchParams.set(key, value);
  }
  const to = url.pathname.slice(UI_BASE.length) + url.search;
  navigate(to, { replace: !push });
}

export function href(to: string): string {
  return `${UI_BASE}${to}`;
}

type LinkProps = Omit<AnchorHTMLAttributes<HTMLAnchorElement>, "href"> & {
  to: string;
  children: ReactNode;
};

export const Link = forwardRef<HTMLAnchorElement, LinkProps>(function Link(
  { to, onClick, ...rest },
  ref,
) {
  const handle = (event: MouseEvent<HTMLAnchorElement>) => {
    onClick?.(event);
    if (
      event.defaultPrevented ||
      event.button !== 0 ||
      event.metaKey ||
      event.ctrlKey ||
      event.shiftKey ||
      event.altKey ||
      rest.target === "_blank"
    ) {
      return;
    }
    event.preventDefault();
    navigate(to);
  };
  return <a ref={ref} href={href(to)} onClick={handle} {...rest} />;
});

/** Matches `/runs/:id` style patterns; returns the parameters or null. */
export function match(pattern: string, path: string): Record<string, string> | null {
  const want = pattern.split("/");
  const have = path.replace(/\/+$/, "").split("/");
  if (want.length !== have.length) return null;
  const params: Record<string, string> = {};
  for (let i = 0; i < want.length; i++) {
    const part = want[i] ?? "";
    const value = have[i] ?? "";
    if (part.startsWith(":")) {
      if (!value) return null;
      params[part.slice(1)] = decodeURIComponent(value);
    } else if (part !== value) {
      return null;
    }
  }
  return params;
}
