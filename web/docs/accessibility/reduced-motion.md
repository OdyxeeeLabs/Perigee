# Reduced motion — issue #702

## What was already here, and what was actually missing

`prefers-reduced-motion` was already honoured in two places:

| Layer | Mechanism | Covers |
|---|---|---|
| CSS | `@media (prefers-reduced-motion: reduce)` in `styles/globals.css` | animations, transitions, `scroll-behavior` |
| Framer Motion | `<MotionConfig reducedMotion="user">` in `components/MotionProvider.tsx` | every `motion.*` component |

Both are app-wide defaults, which is why new `motion.*` components do not have
to remember to opt in. That part was fine.

The gap was everything that is neither CSS nor Framer: canvas plots, chart
libraries with their own animation engine, hand-rolled `requestAnimationFrame`
loops. Those had no way to read the preference, and no rule telling an author
what "respect it" means when the animation is the status message.

## The rule

`prefers-reduced-motion` is not a boolean for the whole app. Motion that
*decorates* and motion that *communicates* have opposite answers, and
collapsing them into one boolean is how an app either ignores the preference
or strips the meaning out of its own UI.

`lib/a11y/motion.ts` encodes that as one function per kind:

| Kind | Example | Under `reduce` |
|---|---|---|
| `decorative` | fade-in, hover lift | **dropped** |
| `transform` | slide, zoom, flip | **dropped** |
| `layout` | reflow/resize transitions | **dropped** |
| `status` | progress bar advancing | **kept** — it carries information |
| `loader` | indeterminate spinner | **kept and slowed** |

WCAG references: 2.3.3 *Animation from Interactions* (AAA) governs motion the
user triggers, 2.2.2 *Pause, Stop, Hide* (A) governs anything that moves for
more than five seconds — which is where the loader rules come from.

## Loaders must keep a non-motion cue

This is the part that is easy to get backwards. The intuitive fix — stop the
spinner — is wrong: **a stopped ring is indistinguishable from a broken UI.**
The existing stylesheet already slowed `.animate-spin` to 3s rather than
stopping it, which is the correct call, but the rule was only implied by a
class selector and the `Spinner` documentation actually claimed the opposite
("the CSS animation is disabled by the global `prefers-reduced-motion`
rule"). The comment and the CSS contradicted each other; the CSS is right and
the comment is now fixed.

The enforceable rule, encoded as `loaderCue()`:

```
{ spinMs: 3000, ariaBusy: true, requiresText: true, keepVisible: true }
```

So: the ring stays visible and keeps turning slowly, it carries `aria-busy`,
and it must never be the only signal — pass `label`, or pair it with adjacent
text. Any loader that animates without a non-motion cue fails review.

## Reading the preference in JS

`hooks/usePrefersReducedMotion.ts` returns a tri-state `boolean | null`:

- `true` — the user asked for reduced motion.
- `false` — resolved, no preference.
- `null` — **not resolved yet** (SSR, and the first client render).

`null` is deliberately distinct from `false`. `matchMedia` does not exist on
the server, so rendering `true` on the first client render would be a
hydration mismatch on every page for every user with the setting on. Resolving
it in an effect costs one frame. Framer's own `useReducedMotion()` has the
same `null` window, and `shouldAnimate` treats `null` as "no preference" for
exactly this reason.

The hook subscribes to the `change` event — the setting is a live OS switch,
not a page-load constant — and falls back to the deprecated `addListener` for
Safari < 14.

## Durations collapse to 0.01ms, not 0

`transitionDuration()` returns `0.01`, not `0`. A zero-duration transition does
not fire `transitionend`, so any completion listener (upload, wizard step,
auto-advance carousel) would wait forever. `0.01ms` is imperceptible but still
fires. This matches the value the stylesheet already uses, and the test asserts
it so the two cannot drift.

## Known gaps

Honest list, in priority order:

1. **Canvas and chart animation.** Any `requestAnimationFrame` loop or chart
   library that animates on load needs to adopt `useMotionPreference()`
   explicitly. A grep for canvas usage is the next piece of work; nothing in
   `lib/a11y/motion.ts` can fix a library that ignores it.
2. **Autoplaying video.** Not present today. If added, it needs `autoplay`
   suppressed under `reduce` and an explicit play control.
3. **Per-user override.** There is deliberately no in-app toggle for motion.
   The OS setting is the mechanism, and adding a second one invites the two to
   disagree.
4. **Testing.** The assertions here cover the decision logic and the presence
   of the app-level configuration. That motion *looks* reduced still needs a
   manual pass with the OS setting on — which is part of the manual
   validation this PR is not running.