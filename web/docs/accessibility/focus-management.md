# Focus management — issue #703

## The problem with the "just trap the focus" approach

The focus-trap logic this issue needed existed once, inline, inside
`NavDrawer`. That is how this goes wrong: the copy added to the error dialog
never filtered hidden controls, and the copy in the wizard never handled the
empty case, so each one is correct only by accident.

Three defects in the original inline implementation, all of which are now
fixed in `hooks/useFocusTrap.ts`:

1. **No visibility filter.** `querySelectorAll` returns controls hidden with
   `display:none`, `visibility:hidden`, a closed `<details>`, or an
   `aria-hidden`/`inert` ancestor. Tab moved focus to things the user could not
   see, which is worse than no trap at all — focus appears to be lost.
2. **Document order, not tab order.** Positive `tabindex` values were ignored,
   so the trap's "first" and "last" disagreed with the browser's, and wrapping
   landed the user somewhere unpredictable.
3. **An empty dialog had no trap at all.** With zero focusable children the
   handler returned early and left `Tab` alone, so focus walked straight out
   into the page behind the dialog. WCAG 2.4.3 does not exempt empty dialogs.

## Layering

`lib/a11y/focus.ts` is deliberately DOM-free. It takes structural types, so
the decisions — *is this tabbable*, *where does Tab go*, *what gets focus on
open*, *is this trigger still restorable* — are testable without jsdom, and the
hook is left containing only wiring. The DOM adapters (`isFocusableElement`,
`getFocusableElements`, `initialFocusTarget`) live in the hook.

| Module | Responsibility |
|---|---|
| `lib/a11y/focus.ts` | Pure decisions. No DOM. |
| `hooks/useFocusTrap.ts` | Listeners, focus calls, DOM adapters. |

`WalletModal` joins `NavDrawer` and `VaultDetailModal` in using the shared
hook, so Escape and focus restoration behave the same in every overlay.

## Decisions worth knowing about

**Focus is restored to the trigger if it survived, otherwise to a stable
fallback.** Closing a dialog whose trigger was deleted in the meantime — a
list row, a toast, a control removed by a route transition — and calling
`.focus()` on the detached node silently drops the user at the top of the
document. `shouldRestoreFocus` checks `isConnected`, `disabled`, `tabIndex`
and visibility first; `pickFocusRestoreTarget` then falls back to
`#main-content` (or `[data-focus-fallback]`, or `<main>`). The fallback is
programmatically focusable, so focus never lands on `<body>` after a close,
an error state, or a navigation that removed the opener.

**`tabindex` is compared with `getClientRects()`, not `offsetParent`.** A
`position: fixed` dialog has a null `offsetParent`; using it would report the
dialog itself as hidden and disable its own trap.

**The keydown listener is on `document`, not the dialog.** A re-render can
move focus outside the dialog, and Escape must still work — otherwise the only
escape from a trap that has failed is to reload. The handler also checks that
focus is *inside* the trap before acting, so two stacked dialogs do not both
close on one Escape.

**The trap only intervenes where the browser cannot.** Tab is left alone in
the middle of the list, where the browser's own order is correct and cheaper;
`preventDefault` happens only at the two wrap points and when focus has
escaped (`activeIndex < 0`, e.g. the focused node was removed by a re-render).

**`orderByTabIndex` does not mutate its input.** A sort that reorders the
caller's array is a data race waiting for the next feature.

## Initial focus

`resolveInitialFocus` follows the APG order of preference:

1. an explicit `initialFocus` callback or `"container"`,
2. `[data-autofocus]`, via `initialFocus: "custom"`,
3. the first tabbable element,
4. the container itself.

Step 4 is what makes "focus is always inside" a guarantee. A container with no
`tabindex` cannot receive focus programmatically, so the hook adds
`tabindex="-1"` before focusing it — that missing attribute is the usual
reason a dialog silently fails to trap.

`NavDrawer` uses `"custom"` because its close button is the first tabbable
child: focusing the first *nav link* would scroll past the only control that
dismisses the drawer.

## Error summaries

`components/ErrorSummary.tsx` implements the APG error-summary pattern.
Focusing the offending field on submit is worse than it looks — it scrolls
past the other problems, announces one message, and hides the fact that four
things are wrong. Instead the summary takes focus and each entry links to its
field.

Two details that are easy to get wrong:

- **Links move focus, not just the scroll position.** A plain `href="#id"`
  scrolls but leaves a keyboard user focused on the summary, so the next Tab
  goes somewhere unrelated. The handler calls `target.focus()` then
  `scrollIntoView`.
- **`role="alert"` is used deliberately but not as a focus mechanism.** It
  interrupts whatever the screen reader is currently saying, which is wrong
  for validation arriving after a deliberate submit. The focus move is what
  announces; the live region covers the case where focus was moved elsewhere
  on purpose (a field re-checked while the user typed in another one).

## Manual verification still required

The 27 assertions cover the decision logic and prove `NavDrawer` no longer
carries its own trap. They cannot cover how the trap *feels*, so before
merging, with a keyboard only:

- [ ] Open the nav drawer: focus lands on the close button, not a link.
- [ ] Tab cycles within the drawer in both directions, including past the edges.
- [ ] Escape closes the drawer **and** focus returns to the hamburger button.
- [ ] Close the drawer after the page has re-rendered behind it — focus must
      still return, not jump to the top of the document.
- [ ] Open the wallet modal, press Escape, and confirm focus returns to the
      button that opened it.
- [ ] Navigate away from a page while an overlay is open: closing the overlay
      must leave focus on the main content, never on `<body>`.
- [ ] Remove the opener (e.g. delete its list row) while a modal is open, then
      close it: focus moves to `#main-content`, not the document top.
- [ ] Submit a form with several invalid fields: the summary takes focus, each
      entry moves focus to its field, and nothing scrolls the other errors
      out of view.
- [ ] With a `display:none` control inside a dialog, Tab must skip it.