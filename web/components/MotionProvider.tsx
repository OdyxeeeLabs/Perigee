"use client";

import { MotionConfig } from "framer-motion";
import type { ReactNode } from "react";

/**
 * WEB-15 (#469): app-wide Framer Motion configuration.
 *
 * `reducedMotion="user"` makes every descendant `motion.*` component honour
 * the operating-system `prefers-reduced-motion` setting automatically:
 * transform and layout animations are skipped (elements snap straight to
 * their final state) instead of animating. Opacity/colour changes that carry
 * meaning still work because individual components keep their existing
 * `useReducedMotion()` guards for those.
 *
 * Wrapping the whole tree once here means every Framer Motion animation in
 * the app respects the preference by default — new `motion.*` components no
 * longer need to remember to opt in, which is exactly the audit WEB-15 asks
 * for.
 *
 * This is a client component because `MotionConfig` reads a media query; both
 * `pages/_app.tsx` (active Pages Router) and `app/layout.tsx` (App Router
 * foundation) render it.
 */
export function MotionProvider({ children }: { children: ReactNode }) {
  return <MotionConfig reducedMotion="user">{children}</MotionConfig>;
}

export default MotionProvider;
