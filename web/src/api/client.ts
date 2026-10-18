// The one HTTP client of the console: same-origin `/api/v1`, bearer
// authentication from memory, and typed errors.
import type { PlanDiagnostic } from "./types";

export const API_BASE = "/api/v1";

/** The backend answered with a structured error. */
export class ApiError extends Error {
  readonly status: number;
  readonly code: string;
  readonly details: string[];
  readonly diagnostics: PlanDiagnostic[];

  constructor(
    status: number,
    code: string,
    message: string,
    details: string[] = [],
    diagnostics: PlanDiagnostic[] = [],
  ) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.code = code;
    this.details = details;
    this.diagnostics = diagnostics;
  }
}

/** No answer arrived: the backend is unreachable, the connection dropped,
 * or a proxy answered with something that is not the API. Whether a
 * mutation took effect is unknown. */
export class NetworkError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "NetworkError";
  }
}

export interface RequestOptions {
  signal?: AbortSignal;
  headers?: Record<string, string>;
}

export interface Answer<T> {
  status: number;
  body: T;
}

export interface Client {
  get<T>(path: string, options?: RequestOptions): Promise<T>;
  post<T>(path: string, body: unknown, options?: RequestOptions): Promise<Answer<T>>;
}

export interface ClientConfig {
  /** The bearer token, or null without one. Read on every request. */
  token: () => string | null;
  /** Called on a 401, before the error is thrown. */
  onUnauthorized: () => void;
  fetch?: typeof fetch;
}

interface ErrorShape {
  error?: { code?: unknown; message?: unknown; details?: unknown; diagnostics?: unknown };
}

function strings(value: unknown): string[] {
  return Array.isArray(value) ? value.filter((v): v is string => typeof v === "string") : [];
}

function diagnostics(value: unknown): PlanDiagnostic[] {
  if (!Array.isArray(value)) return [];
  return value.filter(
    (d): d is PlanDiagnostic =>
      typeof d === "object" &&
      d !== null &&
      typeof (d as PlanDiagnostic).document === "string" &&
      typeof (d as PlanDiagnostic).line === "number" &&
      typeof (d as PlanDiagnostic).message === "string",
  );
}

/** Turns a non-2xx answer into an [ApiError]; anything that is not the
 * API's JSON error body becomes a [NetworkError]. */
export function errorFrom(status: number, body: unknown): ApiError | NetworkError {
  const error = (body as ErrorShape | null)?.error;
  if (!error || typeof error.code !== "string" || typeof error.message !== "string") {
    return new NetworkError(`The backend answered ${status} without an API error.`);
  }
  return new ApiError(
    status,
    error.code,
    error.message,
    strings(error.details),
    diagnostics(error.diagnostics),
  );
}

export function createClient(config: ClientConfig): Client {
  const doFetch = config.fetch ?? ((input, init) => fetch(input, init));

  async function request<T>(
    method: "GET" | "POST",
    path: string,
    body: unknown,
    options: RequestOptions = {},
  ): Promise<Answer<T>> {
    const headers: Record<string, string> = { Accept: "application/json", ...options.headers };
    const token = config.token();
    if (token) headers.Authorization = `Bearer ${token}`;
    if (body !== undefined) headers["Content-Type"] = "application/json";
    let response: Response;
    try {
      response = await doFetch(`${API_BASE}${path}`, {
        method,
        headers,
        body: body === undefined ? null : JSON.stringify(body),
        signal: options.signal ?? null,
        credentials: "same-origin",
        cache: "no-store",
        redirect: "error",
      });
    } catch (cause) {
      if (options.signal?.aborted) throw cause;
      throw new NetworkError("Couldn't reach the backend.");
    }
    const parsed = await readJson(response);
    if (response.ok && parsed.ok) return { status: response.status, body: parsed.value as T };
    if (response.ok) throw new NetworkError("The backend answered without JSON.");
    if (response.status === 401) config.onUnauthorized();
    throw errorFrom(response.status, parsed.ok ? parsed.value : null);
  }

  return {
    get: async <T>(path: string, options?: RequestOptions) =>
      (await request<T>("GET", path, undefined, options)).body,
    post: <T>(path: string, body: unknown, options?: RequestOptions) =>
      request<T>("POST", path, body, options),
  };
}

async function readJson(response: Response): Promise<{ ok: true; value: unknown } | { ok: false }> {
  const type = response.headers.get("content-type") ?? "";
  if (!type.includes("application/json")) return { ok: false };
  try {
    return { ok: true, value: await response.json() };
  } catch {
    return { ok: false };
  }
}

/** Escapes one path segment, such as a run ID or action name. */
export const seg = (value: string): string => encodeURIComponent(value);

/** A query string from the defined values. */
export function query(params: Record<string, string | number | undefined>): string {
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value !== undefined) search.set(key, String(value));
  }
  const text = search.toString();
  return text ? `?${text}` : "";
}
