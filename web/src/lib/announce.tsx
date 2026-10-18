// One polite live region for status changes worth hearing: a run that
// finished, a submission accepted, an action decided. Progress events are
// never announced one by one.
import { type ReactNode, createContext, useCallback, useContext, useState } from "react";

const Announce = createContext<(message: string) => void>(() => {});

export function AnnouncerProvider({ children }: { children: ReactNode }) {
  const [message, setMessage] = useState("");
  const announce = useCallback((text: string) => {
    // Clearing first makes a repeated message announced again.
    setMessage("");
    window.setTimeout(() => setMessage(text), 50);
  }, []);
  return (
    <Announce.Provider value={announce}>
      {children}
      <div className="visually-hidden" role="status" aria-live="polite" aria-atomic="true">
        {message}
      </div>
    </Announce.Provider>
  );
}

export const useAnnounce = (): ((message: string) => void) => useContext(Announce);
