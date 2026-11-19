"use client";

import { useEffect, useRef, useState } from "react";
import {
  formatElapsed,
  nextMilestoneFrom,
  shouldAnnounce,
} from "@/lib/a11y/live-region";
import { usePrefersReducedMotion } from "@/hooks/usePrefersReducedMotion";

export interface LiveTimerProps {
  /** Whether the operation is still running. */
  running: boolean;
  /** Set false while there is nothing to time (idle), to avoid a stray region. */
  active?: boolean;
  /** Milliseconds between announcements. Defaults to 60s. */
  stepMs?: number;
  /** Label for what is being timed, e.g. "Analysis". */
  label?: string;
  /** Called once when the operation ends, so a completion can be announced. */
  onComplete?: () => void;
}

/**
 * Announces the elapsed time of a long-running operation — issue #704.
 *
 * The point of this component is what it *does not* do. The obvious
 * implementation renders `{formatElapsed(elapsed)}` inside an `aria-live`
 * region and updates it every second; that floods the speech queue, every
 * later message queues behind it, and the user cannot escape the region with
 * Escape. Announcements are therefore milestone-based: a `setInterval` that
 * wakes only when the next milestone is due, announcing at most one message
 * per `stepMs`.
 *
 * The interval itself ticks every second to keep the *visual* readout
 * accurate, but it only writes to the live region when a milestone is
 * crossed, and it never writes the same text twice.
 *
 * Under `prefers-reduced-motion: reduce` the visual readout stops updating
 * entirely: a number changing every second is motion, and an unnecessary one
 * for a sighted user who can read the operation is still in progress. The
 * announcements continue — they carry information, they are not decoration.
 *
 * @example
 * <LiveTimer running={isAnalysing} active={isAnalysing} label="Analysis" />
 */
export function LiveTimer({
  running,
  active = true,
  stepMs = 60_000,
  label = "Operation",
  onComplete,
}: LiveTimerProps) {
  const [elapsed, setElapsed] = useState(0);
  const [announcement, setAnnouncement] = useState("");
  const startedAt = useRef<number | null>(null);
  const announcedUpTo = useRef<number>(0);
  const prefersReduced = usePrefersReducedMotion();
  const lastText = useRef<string | null>(null);

  useEffect(() => {
    if (!running) {
      startedAt.current = null;
      announcedUpTo.current = 0;
      setElapsed(0);
      setAnnouncement("");
      lastText.current = null;
      return;
    }

    startedAt.current = Date.now();
    const tick = setInterval(() => {
      const started = startedAt.current;
      if (started === null) return;

      const now = Date.now() - started;
      // The visual readout is motion; under `reduce` only the milestones move.
      if (prefersReduced !== true) setElapsed(now);

      // Note the argument: the last milestone announced, not `now`. Passing
      // `now` would skip the milestone the clock is currently sitting on.
      const milestone = nextMilestoneFrom(announcedUpTo.current, stepMs);
      if (milestone === null || now < milestone) return;

      announcedUpTo.current = milestone;
      const text = `${label} running for ${formatElapsed(milestone)}`;
      // Identical text is silent in a live region, so check before writing.
      if (shouldAnnounce(lastText.current, text)) {
        lastText.current = text;
        setAnnouncement(text);
      }
    }, 1000);

    return () => clearInterval(tick);
  }, [running, stepMs, label, prefersReduced]);

  // A completed run should say so exactly once, at the point it ends.
  useEffect(() => {
    if (running) return;
    onComplete?.();
  }, [running, onComplete]);

  if (!active) return null;

  return (
    <div className="sr-only" aria-live="polite" aria-atomic="true" role="status">
      {announcement}
      {/*
       * The visual readout is a separate, non-live element: a value changing
       * every second inside a live region is exactly the flood described above.
       */}
      {prefersReduced !== true && elapsed > 0 ? (
        <span aria-hidden="true">{formatElapsed(elapsed)}</span>
      ) : null}
    </div>
  );
}

export default LiveTimer;