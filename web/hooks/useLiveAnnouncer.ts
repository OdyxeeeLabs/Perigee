"use client";

import { useCallback, useEffect, useRef, useState } from "react";

/**
 * Announcement queue that survives the identical-message case.
 *
 * A live region only speaks when its content *changes*. Two consecutive
 * identical announcements are therefore silent, which is not a rare edge
 * case: "Address copied" twice, "You are offline" after a 500 ms reconnect,
 * "Upload failed" retried twice. From the user's side the feature looks
 * broken, and there is nothing in the DOM to debug.
 *
 * The fix is the standard clear-then-set: write an empty string, let React
 * commit it, then write the real message on the next frame. The empty commit
 * is what makes the second write a change.
 *
 * `clearMs` is not decorative. Assistive technology debounces live-region
 * updates; clearing and re-setting inside the same tick can be coalesced back
 * into the original string and the announcement is lost again. A short delay
 * is the pragmatic cost of a message that is reliably heard.
 */
export interface UseLiveAnnouncerOptions {
  /** Politeness of the region. Defaults to `polite`. */
  politeness?: "polite" | "assertive";
  /** Delay before re-setting the same message, in ms. */
  clearMs?: number;
  /** Accessible name for the region. */
  label?: string;
}

export interface LiveAnnouncer {
  /** Current message to render. Keep the region mounted with this value. */
  message: string;
  /** Announce a message, repeating it even if it matches the last one. */
  announce: (message: string) => void;
  /** Reset to silent. */
  clear: () => void;
  /** Remove the region from the DOM (e.g. unmounting the owning view). */
  stop: () => void;
}

export function useLiveAnnouncer({
  politeness = "polite",
  clearMs = 50,
}: UseLiveAnnouncerOptions = {}): LiveAnnouncer {
  const [message, setMessage] = useState("");
  const lastRef = useRef<string | null>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const clearTimer = useCallback(() => {
    if (timerRef.current !== null) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }
  }, []);

  // A pending announcement must not fire into an unmounted component.
  useEffect(() => clearTimer, [clearTimer]);

  const announce = useCallback(
    (next: string) => {
      clearTimer();
      if (lastRef.current === null || lastRef.current !== next) {
        lastRef.current = next;
        setMessage(next);
        return;
      }
      // Identical repeat: clear first so the second write is a real change.
      setMessage("");
      timerRef.current = setTimeout(() => {
        timerRef.current = null;
        setMessage(next);
      }, clearMs);
    },
    [clearTimer, clearMs],
  );

  const clear = useCallback(() => {
    clearTimer();
    lastRef.current = null;
    setMessage("");
  }, [clearTimer]);

  const stop = clear;

  return { message, announce, clear, stop };
}

/**
 * Announce once per change, skipping repeats — the non-repeating variant for
 * callers where a duplicate message is genuinely not news (see
 * `shouldAnnounce` in `lib/a11y/live-region`).
 */
export function useLiveAnnouncerOnce() {
  const { message, announce } = useLiveAnnouncer();
  const lastRef = useRef<string | null>(null);

  const announceOnce = useCallback(
    (next: string) => {
      if (lastRef.current === next) return;
      lastRef.current = next;
      announce(next);
    },
    [announce],
  );

  return { message, announce: announceOnce };
}

export default useLiveAnnouncer;