"use client";

import { useEffect, useState } from "react";
import type { MotionPreference } from "@/lib/a11y/motion";

/**
 * The OS-level `prefers-reduced-motion` setting — issue #702.
 *
 * `<MotionConfig reducedMotion="user">` already covers Framer Motion, and the
 * global stylesheet covers CSS animations and transitions. What neither covers
 * is a component driven by neither: a canvas plot, a hand-rolled `requestAnimationFrame`
 * loop, a chart library's own animation engine. Those need to read the
 * preference directly, and they need it to be *stable*.
 *
 * Two implementation details that matter:
 *
 * 1. **SSR.** `matchMedia` does not exist on the server, so the first render
 *    must assume "no preference" to match the server's markup. Flipping to
 *    `reduce` on hydration would be a mismatch; resolving in an effect costs
 *    one frame and is correct.
 * 2. **Change events.** The setting is a live OS switch, not a page-load
 *    constant. `addEventListener("change")` is required, not a one-shot read.
 *
 * The returned value is deliberately tri-state: `null` means "not resolved
 * yet", which is different from `false` ("resolved, user does not want
 * reduced motion") and collapses to the wrong answer if flattened.
 */
export function usePrefersReducedMotion(): boolean | null {
  const [prefersReduced, setPrefersReduced] = useState<boolean | null>(null);

  useEffect(() => {
    if (typeof window === "undefined" || typeof window.matchMedia !== "function") {
      return;
    }

    const query = window.matchMedia("(prefers-reduced-motion: reduce)");
    const update = (event: MediaQueryList | MediaQueryListEvent) => {
      setPrefersReduced(event.matches);
    };

    update(query);

    // Safari < 14 only implements the deprecated form; both are guarded
    // because the modern listener is simply absent there.
    if (typeof query.addEventListener === "function") {
      query.addEventListener("change", update);
      return () => query.removeEventListener("change", update);
    }
    if (typeof query.addListener === "function") {
      query.addListener(update);
      return () => query.removeListener(update);
    }
  }, []);

  return prefersReduced;
}

/**
 * The same value in the tri-state form the pure helpers in `lib/a11y/motion`
 * expect, so components pass a value straight through instead of converting
 * `null` to `false` and losing the "not resolved yet" distinction.
 */
export function useMotionPreference(): MotionPreference | null {
  const prefersReduced = usePrefersReducedMotion();
  return prefersReduced === null ? null : prefersReduced ? "reduce" : "no-preference";
}

export default usePrefersReducedMotion;