# Live regions — issue #704

`LiveStatus` and `LiveAlert` (from #470) already answered *how* to announce.
This issue is about *what is worth announcing*, which is where live-region
implementations usually break.

## Two failure modes, both worse than silence

**1. Announcing every tick.** The obvious timer renders `{formatElapsed(x)}`
inside an `aria-live` region and updates it once per second. The speech queue
backs up, every later message queues behind it, and a screen-reader user
cannot escape the region with Escape. The test suite asserts this directly:
60 seconds of per-second ticking must produce exactly one announcement, and
ten minutes must produce ten rather than 600.

**2. Announcing nothing because the text did not change.** A live region only
speaks when its content changes, so two consecutive identical messages are
silent. This is not a rare edge case — "Address copied" twice, "You are
offline" after a 500 ms reconnect, "Upload failed" retried. There is nothing
in the DOM to debug; the feature just looks broken. `hooks/useLiveAnnouncer.ts`
implements the standard clear-then-set, and the 50 ms delay is load-bearing:
assistive technology debounces live-region updates, and clearing and re-setting
inside the same tick can be coalesced back into the original string.

## New in this change

| Component | Announces | Notes |
|---|---|---|
| `LiveTimer` | Elapsed time of a long operation | Milestone-based; visual readout is `aria-hidden` |
| `NetworkStatusRegion` | Online / offline / degraded | Hysteresis, no repeats |
| `useLiveAnnouncer` | Anything, repeat-safe | Clear-then-set |

`lib/a11y/live-region.ts` holds the policy: `politenessFor`,
`nextMilestoneFrom`, `formatElapsed`, `shouldAnnounce`,
`connectivityAnnouncement`, `connectionState`. It is DOM-free, so the policy is
tested directly and the components are a thin rendering of it.

## `nextMilestoneFrom` takes the last milestone, not the current time

This is a trap worth naming. The parameter is the previous milestone already
announced:

```
nextMilestoneFrom(0,      60_000) ===  60_000
nextMilestoneFrom(60_000, 60_000) === 120_000
```

A caller that passes the *current elapsed time* instead gets the same answer at
`t = 60_000` (120_000) and therefore silently swallows the one-minute
announcement. The component compares the returned milestone against real
elapsed time and waits for the clock to reach it. The function was renamed from
`nextMilestone` to make that contract visible at the call site, and a test
pins the difference.

## Loaders, timers and reduced motion

Under `prefers-reduced-motion: reduce`, `LiveTimer` stops updating its visual
readout entirely: a number changing every second is motion, and an unnecessary
one. The announcements continue — they carry information, they are not
decoration. Same principle as #702's `loaderCue`: never let the motion be the
only signal, and never remove the signal.

## Connectivity hysteresis

`navigator.onLine` and RPC reachability flap — a tunnel, a flaky phone, a
closing laptop lid. Announcing every transition produces "You are offline. You
are online. You are offline." in a burst. So a state must persist for
`settleMs` (default 2s) before it is announced, the visual state does not flip
while settling, and the same state is never announced twice.

Both directions are polite. Interrupting to say "connection restored" while the
user is mid-sentence is worse than a one-second delay; only
`politenessFor("error")` is assertive.

## The rule that has not changed

A live region must exist in the DOM *before* its first message, or assistive
technology has not registered it and the announcement is dropped. That is why
every region here renders even when its message is empty, and a test asserts
none of them are conditionally rendered.

## Manual verification still required

Automated tests cannot check whether a screen reader actually speaks. With
VoiceOver (Safari) or NVDA (Firefox), and with the platform's own live-region
settings:

- [ ] Start a long operation: announcements at the first milestone only, not one per second.
- [ ] Confirm `LiveTimer` does not flood the queue and Escape still interrupts.
- [ ] Toggle the network mid-operation: one announcement per settled state, none during a flap.
- [ ] Announce the same message twice through `useLiveAnnouncer` — both must be heard.
- [ ] Confirm the visual readout is not announced (it is `aria-hidden`).
- [ ] Turn on "reduce motion" and confirm the visual counter stops while announcements continue.
