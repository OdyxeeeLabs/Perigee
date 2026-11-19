/**
 * Motion decisions, as pure functions — issue #702.
 *
 * `prefers-reduced-motion` is not a boolean "on/off" for the whole app. Some
 * motion is a vestibular trigger (parallax, large-scale movement, spinning)
 * and some carries meaning (a progress bar advancing, a status colour
 * changing). Collapsing that into one boolean is how apps end up either
 * ignoring the preference or stripping meaning out of their UI, so the
 * decision is made per kind of motion and tested here.
 *
 * The rule this encodes: under `reduce`, motion that *decorates* is dropped
 * and motion that *communicates* is kept, slowed, or replaced with a
 * non-motion equivalent. WCAG 2.3.3 (Animation from Interactions, AAA) is
 * about triggered motion; 2.2.2 (Pause, Stop, Hide) is about anything that
 * moves for more than five seconds, which is where the loader rules come
 * from.
 */

export type MotionPreference = "reduce" | "no-preference";

/**
 * Categories of motion the app uses.
 *
 * - `decorative` — entrances, fades, hover lifts, decorative parallax.
 * - `transform`   — movement or scaling of an element (slide, zoom, flip).
 * - `layout`      — reflow/resize transitions; Framer's layout animations.
 * - `loader`      — indeterminate progress that runs until work finishes.
 * - `status`      — progress/state that conveys information (progress bar).
 */
export type MotionKind = "decorative" | "transform" | "layout" | "loader" | "status";

/**
 * The non-motion equivalent a loader must fall back to.
 *
 * A ring that stops spinning is indistinguishable from a broken UI, so the
 * rule is not "stop the animation" but "never let the animation be the only
 * signal". Under `reduce` the caller is expected to render this cue as well.
 */
export interface LoaderCue {
  /** Slow the animation rather than removing it: still visibly busy, no longer a spin. */
  readonly spinMs: number;
  /** Announced to assistive technology so progress is not visual-only. */
  readonly ariaBusy: true;
  /** Text that must accompany the ring, e.g. "Loading vault data…". */
  readonly requiresText: true;
  /** Whether the ring must stay visible when motion is reduced. */
  readonly keepVisible: true;
}

/** Slowed spin for loaders under `reduce` — long enough not to read as motion. */
export const REDUCED_LOADER_CUE: LoaderCue = Object.freeze({
  spinMs: 3000,
  ariaBusy: true,
  requiresText: true,
  keepVisible: true,
});

/** Kinds that are dropped outright when the user asks for reduced motion. */
const DISABLED_WHEN_REDUCED: ReadonlySet<MotionKind> = new Set<MotionKind>([
  "decorative",
  "transform",
  "layout",
]);

/**
 * Should this animation run?
 *
 * `preference` is `null` while the media query is unresolved — during SSR and
 * on the first client render. `null` is treated as "no preference" so the
 * first render matches the server; flipping it afterwards would be a
 * hydration mismatch. This mirrors Framer Motion's own `useReducedMotion`,
 * which returns `null` for exactly this window.
 */
export function shouldAnimate(
  kind: MotionKind,
  preference: MotionPreference | null | undefined,
): boolean {
  if (preference !== "reduce") return true;
  return !DISABLED_WHEN_REDUCED.has(kind);
}

/** Loaders keep moving, slowly, and always need a text cue. */
export function loaderCue(preference: MotionPreference | null | undefined): LoaderCue {
  // Same cue either way: the animation slows, it does not stop. Kept as a
  // function so the contract has one place to change if a static fallback
  // becomes viable.
  void preference;
  return REDUCED_LOADER_CUE;
}

/** Duration for a CSS transition, in ms. */
export function transitionDuration(
  baseMs: number,
  preference: MotionPreference | null | undefined,
): number {
  if (preference !== "reduce") return Math.max(0, baseMs);
  // The 0.01ms idiom used by the global stylesheet: the transition still
  // *fires*, so `transitionend` listeners are not left waiting forever, but
  // there is no perceptible motion.
  return 0.01;
}

/** Framer's `transition` object for a given animation. */
export function motionTransition(
  kind: MotionKind,
  base: { duration: number; delay?: number } = { duration: 0.3 },
  preference: MotionPreference | null | undefined = null,
) {
  if (shouldAnimate(kind, preference)) {
    return base.delay === undefined ? base : { duration: base.duration, delay: base.delay };
  }
  return { duration: 0 };
}