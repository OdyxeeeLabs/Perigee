"use client";

import { useEffect, useRef, useState } from "react";
import {
  connectionState,
  connectivityAnnouncement,
  politenessFor,
  shouldAnnounce,
  type AnnouncementKind,
} from "@/lib/a11y/live-region";
import { useLiveAnnouncer } from "@/hooks/useLiveAnnouncer";

export interface NetworkStatusRegionProps {
  /** Explicit override; otherwise `navigator.onLine` is polled. */
  online?: boolean;
  /** Whether the API/RPC is currently reachable. `null` = not checked yet. */
  reachable?: boolean | null;
  /**
   * How long a state must hold before it is announced. Defaults to 2000ms.
   *
   * Connection state flaps — a tunnel, a flaky phone, a closing laptop lid —
   * and announcing every transition produces "You are offline. You are
   * online. You are offline." in a burst, which is worse than saying nothing.
   */
  settleMs?: number;
  /** Test hook. */
  "data-testid"?: string;
}

/**
 * Announces connectivity changes — issue #704.
 *
 * `NetworkStatusBanner` and `RpcFallbackBanner` already show this state
 * visually. Neither announced it: a screen-reader user who loses the backend
 * is left with a page that silently stops updating, and has no way to tell a
 * network problem from a bug.
 *
 * Three details that matter more than the markup:
 *
 * - **Hysteresis.** A state must hold for `settleMs` before it is announced,
 *   so a flap never reaches the user.
 * - **No repeats.** The same state twice in a row is not news, and a live
 *   region would be silent anyway.
 * - **Recovery is polite, failure is not.** Going offline interrupts nothing;
 *   "connection restored" arriving while the user is mid-sentence is worse
 *   than a one-second delay. Both are polite here, which
 *   `politenessFor("connectivity")` encodes.
 *
 * The region is rendered even when silent: a region that mounts together with
 * its first message is frequently never announced at all.
 */
export function NetworkStatusRegion({
  online,
  reachable = null,
  settleMs = 2000,
  "data-testid": testId,
}: NetworkStatusRegionProps) {
  const { message, announce } = useLiveAnnouncer({ politeness: "polite" });
  const [state, setState] = useState<"online" | "offline" | "degraded">("online");
  const heldSince = useRef<number>(Date.now());
  const lastMessage = useRef<string | null>(null);

  // Track the browser's own connectivity flag when no override is supplied.
  const [browserOnline, setBrowserOnline] = useState(true);
  useEffect(() => {
    if (typeof window === "undefined") return;
    const update = () => setBrowserOnline(window.navigator.onLine);
    update();
    window.addEventListener("online", update);
    window.addEventListener("offline", update);
    return () => {
      window.removeEventListener("online", update);
      window.removeEventListener("offline", update);
    };
  }, []);

  useEffect(() => {
    const next = connectionState({ online: online ?? browserOnline, reachable });
    if (next === state) return;

    const now = Date.now();
    const heldForMs = now - heldSince.current;
    const result = connectivityAnnouncement({
      lastMessage: lastMessage.current,
      state: next,
      heldForMs,
      settleMs,
    });

    if (!result.announce) {
      // Still settling. Do not flip the visual state yet — announcing a state
      // that immediately reverts is the flap we are trying to hide.
      if (heldForMs < settleMs) return;
    }

    heldSince.current = now;
    setState(next);

    if (shouldAnnounce(lastMessage.current, result.message)) {
      lastMessage.current = result.message;
      announce(result.message);
    }
  }, [state, online, browserOnline, reachable, settleMs, announce]);

  const kind: AnnouncementKind = "connectivity";

  return (
    <div
      role="status"
      aria-live={politenessFor(kind)}
      aria-atomic="true"
      aria-label="Network status"
      data-testid={testId}
      className="sr-only"
    >
      {message}
    </div>
  );
}

export default NetworkStatusRegion;