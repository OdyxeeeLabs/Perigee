"use client";

import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { LiveAlert, LiveStatus } from "@/components/LiveRegion";
import {
  DEFAULT_VERBOSITY,
  formatAnnouncement,
  readStoredVerbosity,
  shouldAnnounce,
  storeVerbosity,
  type Verbosity,
} from "@/lib/a11y/announcer";

export type Politeness = "polite" | "assertive";

export interface AnnouncerApi {
  /** Queue a message. Polite by default; assertive for failures. */
  announce: (message: string, politeness?: Politeness, source?: string) => void;
  /** Polite announcement for progress and data updates (balances, portfolio). */
  announceStatus: (message: string, source?: string) => void;
  /** Assertive announcement for errors and confirmations the user must see. */
  announceAlert: (message: string, source?: string) => void;
  /**
   * Announce only when the watched value actually changes — the right shape
   * for a balance or a sync status that re-renders on a poll.
   */
  announceOnChange: (value: unknown, message: string, source?: string) => void;
  verbosity: Verbosity;
  setVerbosity: (verbosity: Verbosity) => void;
}

const AnnouncerContext = createContext<AnnouncerApi | null>(null);

/**
 * Global live-region announcer — issue #697.
 *
 * The app already had `LiveStatus`/`LiveAlert` and used them for toasts and
 * the network banner. What was missing was a single API for the rest of the
 * app to announce through, with two properties that matter in practice:
 *
 *  * **De-duplication.** A live region only speaks when its content changes,
 *    so a component that re-renders with the same message would otherwise
 *    either be silent (and look broken) or need a counter hack to force it.
 *  * **Verbosity.** Balance and portfolio updates can be very chatty. The
 *    user picks `terse` / `normal` / `verbose` and the choice is persisted
 *    and applied from the first render.
 *
 * The two regions are always mounted (an empty string renders nothing), which
 * is what lets assistive technology register them before the first message.
 */
export function AnnouncerProvider({ children }: { children: ReactNode }) {
  const [politeMessage, setPoliteMessage] = useState("");
  const [assertiveMessage, setAssertiveMessage] = useState("");
  const [verbosity, setVerbosityState] = useState<Verbosity>(DEFAULT_VERBOSITY);

  const politeRef = useRef("");
  const assertiveRef = useRef("");
  const lastValueRef = useRef<unknown>(undefined);

  useEffect(() => {
    setVerbosityState(readStoredVerbosity());
  }, []);

  const setVerbosity = useCallback((next: Verbosity) => {
    setVerbosityState(next);
    storeVerbosity(next);
  }, []);

  const announce = useCallback<AnnouncerApi["announce"]>(
    (message, politeness = "polite", source) => {
      const formatted = formatAnnouncement(message, verbosity, { source });
      const isAssertive = politeness === "assertive";

      if (isAssertive) {
        if (!shouldAnnounce(assertiveRef.current, formatted)) return;
        assertiveRef.current = formatted;
        setAssertiveMessage(formatted);
        return;
      }

      if (!shouldAnnounce(politeRef.current, formatted)) return;
      politeRef.current = formatted;
      setPoliteMessage(formatted);
    },
    [verbosity],
  );

  const announceStatus = useCallback(
    (message: string, source?: string) => announce(message, "polite", source),
    [announce],
  );

  const announceAlert = useCallback(
    (message: string, source?: string) => announce(message, "assertive", source),
    [announce],
  );

  const announceOnChange = useCallback(
    (value: unknown, message: string, source?: string) => {
      if (lastValueRef.current === value) return;
      lastValueRef.current = value;
      announce(message, "polite", source);
    },
    [announce],
  );

  const api = useMemo<AnnouncerApi>(
    () => ({
      announce,
      announceStatus,
      announceAlert,
      announceOnChange,
      verbosity,
      setVerbosity,
    }),
    [announce, announceStatus, announceAlert, announceOnChange, verbosity, setVerbosity],
  );

  return (
    <AnnouncerContext.Provider value={api}>
      {children}
      <LiveStatus message={politeMessage} label="Status updates" />
      <LiveAlert message={assertiveMessage} />
    </AnnouncerContext.Provider>
  );
}

/**
 * Access the announcer. Returns a no-op API when no provider is mounted so a
 * component rendered in isolation (a test, a Storybook story) cannot crash.
 */
export function useAnnouncer(): AnnouncerApi {
  const context = useContext(AnnouncerContext);

  const noop = useMemo<AnnouncerApi>(
    () => ({
      announce: () => {},
      announceStatus: () => {},
      announceAlert: () => {},
      announceOnChange: () => {},
      verbosity: DEFAULT_VERBOSITY,
      setVerbosity: () => {},
    }),
    [],
  );

  return context ?? noop;
}

export default AnnouncerProvider;
