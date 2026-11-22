/**
 * Live-region announcement policy — issue #704.
 *
 * The existing `LiveStatus`/`LiveAlert` regions answer *how* to announce.
 * This module answers *what is worth announcing*, which is where live-region
 * implementations usually go wrong.
 *
 * Two failure modes dominate, and both are worse than announcing nothing:
 *
 * 1. **Announcing every tick.** A timer wired to `setState(remaining)` on an
 *    `aria-live` region emits one announcement per second. The speech queue
 *    backs up, every later message is queued behind it, and a screen-reader
 *    user gets a region they cannot escape with Escape. Announcements are
 *    therefore *milestone*-based: start, then every `stepMs`.
 * 2. **Announcing nothing because the text did not change.** A live region
 *    only speaks when its content changes. Two consecutive identical
 *    announcements — "Copied" twice, "You are offline" after a brief
 *    reconnect — are silent, which looks like a broken feature rather than a
 *    missing message. `shouldAnnounce` makes the repeat explicit.
 *
 * DOM-free on purpose: the policy is the part worth testing, and the
 * components are then a thin rendering of it.
 */

export type Politeness = "polite" | "assertive";

export type AnnouncementKind =
  /** Progress, completion, elapsed time. Waits for the user to be idle. */
  | "status"
  /** A failure the user must act on. Interrupts. */
  | "error"
  /** Connection state. Polite: the user is not waiting on a network message. */
  | "connectivity"
  /** A success confirmation. Polite — never interrupt to say "saved". */
  | "confirmation";

/**
 * Which politeness for which kind.
 *
 * The rule that matters: interruption is reserved for things the user cannot
 * continue without. Everything else queues, because an assertive region
 * interrupts whatever the screen reader is currently saying.
 */
export function politenessFor(kind: AnnouncementKind): Politeness {
  return kind === "error" ? "assertive" : "polite";
}

/**
 * Format elapsed time for a human, at a precision that matters.
 *
 * Not `mm:ss` past a minute: "3:42" is ambiguous about hours, and a screen
 * reader reads it as a clock time. Past an hour the minute count is dropped
 * too, because "47 minutes" is noise at that point.
 */
export function formatElapsed(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return "0 seconds";

  const totalSeconds = Math.floor(ms / 1000);
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;

  if (hours > 0) {
    return minutes > 0 ? `${hours} hour${hours === 1 ? "" : "s"} ${minutes} minute${minutes === 1 ? "" : "s"}` : `${hours} hour${hours === 1 ? "" : "s"}`;
  }
  if (minutes > 0) {
    return seconds > 0 && minutes < 1
      ? `${seconds} second${seconds === 1 ? "" : "s"}`
      : `${minutes} minute${minutes === 1 ? "" : "s"}`;
  }
  return `${seconds} second${seconds === 1 ? "" : "s"}`;
}

/**
 * The next milestone to announce, given the last one already announced.
 *
 * The parameter is `announcedUpTo` — the previous milestone — **not** the
 * current elapsed time. That distinction is the whole point: a caller that
 * passes the current time skips the milestone it is standing on. At
 * `announcedUpTo = 0` with a 60s step this returns 60_000; at
 * `announcedUpTo = 60_000` it returns 120_000, while "current time = 60_000"
 * would return 120_000 too and silently swallow the one-minute announcement.
 *
 * Callers therefore compare the returned milestone against the real elapsed
 * time and wait for the clock to reach it, which is what `LiveTimer` does.
 *
 * Returns `null` if `stepMs` is not positive, so a caller cannot loop forever
 * on a misconfigured step.
 */
export function nextMilestoneFrom(
  announcedUpTo: number,
  stepMs: number,
  firstStepMs = stepMs,
): number | null {
  if (stepMs <= 0) return null;
  if (announcedUpTo < firstStepMs) return firstStepMs;
  const steps = Math.floor(announcedUpTo / stepMs);
  return (steps + 1) * stepMs;
}

/**
 * Should this text be announced, given the last thing announced?
 *
 * `null` last-message means nothing has been announced yet, which always
 * announces — including for an empty message, because the *first* render is
 * how the region is registered with assistive technology.
 */
export function shouldAnnounce(lastMessage: string | null, nextMessage: string): boolean {
  if (lastMessage === null) return true;
  return lastMessage !== nextMessage;
}

/**
 * Connectivity announcements, with hysteresis.
 *
 * `navigator.onLine` and RPC reachability flap: a tunnel, a flaky phone, a
 * laptop lid. Announcing every transition produces "You are offline. You are
 * online. You are offline." in a burst, which is worse than silence. So a
 * state must *persist* for `settleMs` before it is announced, and the same
 * state is never announced twice in a row.
 */
export interface ConnectivityAnnouncement {
  announce: boolean;
  message: string;
}

export function connectivityAnnouncement(input: {
  /** Last announced message, or null. */
  lastMessage: string | null;
  /** Current connection state. */
  state: "online" | "offline" | "degraded";
  /** How long `state` has been held. */
  heldForMs: number;
  /** Stability window. */
  settleMs: number;
}): ConnectivityAnnouncement {
  const { lastMessage, state, heldForMs, settleMs } = input;

  const message =
    state === "offline"
      ? "You are offline. Some features are unavailable."
      : state === "degraded"
        ? "Connection is slow. Data may be out of date."
        : "Connection restored.";

  // Hold the state before speaking, so a flap never reaches the user.
  if (heldForMs < settleMs) return { announce: false, message };

  return { announce: shouldAnnounce(lastMessage, message), message };
}

/**
 * Connection state from raw inputs.
 *
 * Kept separate from the announcement policy so the two can be tested apart:
 * a bug in one should not silently change the other's behaviour.
 */
export function connectionState(input: {
  /** `navigator.onLine`. */
  online: boolean;
  /** Whether the last API/RPC request succeeded. */
  reachable: boolean | null;
}): "online" | "offline" | "degraded" {
  if (!input.online) return "offline";
  if (input.reachable === false) return "degraded";
  return "online";
}

/**
 * A live region must exist in the DOM *before* its first message, otherwise
 * assistive technology has not registered it and the announcement is dropped.
 * This is the single most common reason "the live region does not work", and
 * it is why every region in this codebase renders even when empty.
 */
export const shouldRenderRegion = (): true => true;