// The backend connection: health, client authentication and the one API
// client. The bearer token lives in memory only, never in storage, URLs or
// logs. A 401 or a disconnect clears it together with every cached
// response, by remounting the screens under a new generation.
import {
  type ReactNode,
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";

import { type Client, createClient } from "../../api/client";
import { useInterval } from "../../lib/resource";

export type Health = "checking" | "reachable" | "unreachable";
/** `open`: the backend needs no token (loopback). `token`: signed in.
 * `needed`: a token is required. `rejected`: the last token was refused. */
export type Auth = "unknown" | "open" | "token" | "needed" | "rejected";

export interface Connection {
  client: Client;
  health: Health;
  auth: Auth;
  /** When the backend last answered. */
  lastSeen: Date | null;
  /** Changes whenever cached data must be dropped. */
  generation: number;
  /** Tries `token`; resolves once it is accepted or refused. */
  connect: (token: string) => Promise<boolean>;
  /** Forgets the token and every cached response. */
  disconnect: () => void;
  recheck: () => Promise<void>;
}

const Context = createContext<Connection | null>(null);

export function useConnection(): Connection {
  const connection = useContext(Context);
  if (!connection) throw new Error("useConnection outside ConnectionProvider");
  return connection;
}

export const useClient = (): Client => useConnection().client;

async function healthy(): Promise<boolean> {
  try {
    const response = await fetch("/api/v1/health", { cache: "no-store" });
    if (!response.ok) return false;
    const body = (await response.json()) as { status?: unknown };
    return body.status === "ok";
  } catch {
    return false;
  }
}

export function ConnectionProvider({ children }: { children: ReactNode }) {
  const token = useRef<string | null>(null);
  const [health, setHealth] = useState<Health>("checking");
  const [auth, setAuth] = useState<Auth>("unknown");
  const [lastSeen, setLastSeen] = useState<Date | null>(null);
  const [generation, setGeneration] = useState(0);

  const forget = useCallback((next: Auth) => {
    token.current = null;
    setAuth(next);
    setGeneration((g) => g + 1);
  }, []);

  const client = useMemo(
    () =>
      createClient({
        token: () => token.current,
        onUnauthorized: () => forget(token.current ? "rejected" : "needed"),
      }),
    [forget],
  );

  /** Whether the current token (or none) may use client routes. It reads
   * outside the client, so a refusal here does not reset other screens. */
  const probe = useCallback(async (): Promise<Auth | "unreachable"> => {
    const headers: Record<string, string> = { Accept: "application/json" };
    if (token.current) headers.Authorization = `Bearer ${token.current}`;
    try {
      const response = await fetch("/api/v1/runs?limit=1", { headers, cache: "no-store" });
      if (response.status === 401) return "needed";
      if (!response.ok) return "unreachable";
      return token.current ? "token" : "open";
    } catch {
      return "unreachable";
    }
  }, []);

  const recheck = useCallback(async () => {
    if (!(await healthy())) {
      setHealth("unreachable");
      return;
    }
    setHealth("reachable");
    setLastSeen(new Date());
    const result = await probe();
    if (result === "unreachable") return;
    setAuth((previous) => (result === "needed" && previous === "rejected" ? previous : result));
  }, [probe]);

  useEffect(() => {
    void recheck();
  }, [recheck]);

  useInterval(() => void recheck(), health === "unreachable" ? 10_000 : 30_000, true);

  const connect = useCallback(
    async (candidate: string) => {
      token.current = candidate.trim();
      setGeneration((g) => g + 1);
      const result = await probe();
      if (result === "token" || result === "open") {
        setAuth("token");
        setHealth("reachable");
        setLastSeen(new Date());
        return true;
      }
      if (result === "unreachable") {
        setHealth("unreachable");
        return false;
      }
      forget("rejected");
      return false;
    },
    [probe, forget],
  );

  const disconnect = useCallback(() => {
    forget("unknown");
    void recheck();
  }, [forget, recheck]);

  const value = useMemo<Connection>(
    () => ({ client, health, auth, lastSeen, generation, connect, disconnect, recheck }),
    [client, health, auth, lastSeen, generation, connect, disconnect, recheck],
  );
  return <Context.Provider value={value}>{children}</Context.Provider>;
}

/** The short state the navigation shows. */
export function connectionSummary(c: Pick<Connection, "health" | "auth">): {
  label: string;
  tone: "pass" | "warn" | "fail" | "neutral";
} {
  if (c.health === "checking") return { label: "Checking backend", tone: "neutral" };
  if (c.health === "unreachable") return { label: "Backend unreachable", tone: "fail" };
  if (c.auth === "needed" || c.auth === "rejected") return { label: "Token needed", tone: "warn" };
  return { label: "Backend connected", tone: "pass" };
}
