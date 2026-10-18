// Text formatting for times, durations, sizes and identifiers.

const dateTime = new Intl.DateTimeFormat(undefined, {
  year: "numeric",
  month: "short",
  day: "numeric",
  hour: "2-digit",
  minute: "2-digit",
  second: "2-digit",
});

const timeOnly = new Intl.DateTimeFormat(undefined, {
  hour: "2-digit",
  minute: "2-digit",
  second: "2-digit",
});

export function formatTime(iso: string | null | undefined): string {
  if (!iso) return "—";
  const date = new Date(iso);
  return Number.isNaN(date.getTime()) ? iso : dateTime.format(date);
}

export function formatClock(iso: string | Date | null | undefined): string {
  if (!iso) return "—";
  const date = iso instanceof Date ? iso : new Date(iso);
  return Number.isNaN(date.getTime()) ? String(iso) : timeOnly.format(date);
}

/** "3 min ago" style text for recent times, the date otherwise. */
export function formatRelative(iso: string, now: number = Date.now()): string {
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return iso;
  const seconds = Math.round((now - then) / 1000);
  if (seconds < 45) return "just now";
  if (seconds < 3600) return `${Math.round(seconds / 60)} min ago`;
  if (seconds < 86_400) return `${Math.round(seconds / 3600)} h ago`;
  return formatTime(iso);
}

/** A duration in milliseconds as `850 ms`, `12.3 s` or `4 min 05 s`. */
export function formatMs(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)} ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)} s`;
  const minutes = Math.floor(ms / 60_000);
  const seconds = Math.round((ms % 60_000) / 1000);
  return `${minutes} min ${String(seconds).padStart(2, "0")} s`;
}

export function formatSeconds(seconds: number): string {
  if (seconds % 3600 === 0 && seconds >= 3600) return `${seconds / 3600} h`;
  if (seconds % 60 === 0 && seconds >= 60) return `${seconds / 60} min`;
  return `${seconds} s`;
}

export function between(start: string | null | undefined, end: string | null | undefined): string {
  if (!start || !end) return "—";
  const ms = new Date(end).getTime() - new Date(start).getTime();
  return Number.isFinite(ms) && ms >= 0 ? formatMs(ms) : "—";
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
}

/** `run-3f1c9a2b…` for a full run ID. */
export function shortId(id: string, keep = 8): string {
  const prefix = id.startsWith("run-") ? "run-" : "";
  const rest = id.slice(prefix.length);
  return rest.length > keep + 1 ? `${prefix}${rest.slice(0, keep)}…` : id;
}

export function plural(count: number, one: string, many = `${one}s`): string {
  return `${count} ${count === 1 ? one : many}`;
}
