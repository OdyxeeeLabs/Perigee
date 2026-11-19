/**
 * Focus-management primitives — issue #703.
 *
 * The focus-trap logic that was needed for three features existed once,
 * inline, inside `NavDrawer`. That is the usual way this goes wrong: the copy
 * in the error dialog never filtered out hidden controls, and the copy in the
 * wizard never handled the empty case, so each one is correct only by
 * accident.
 *
 * Everything here is deliberately DOM-free. The functions take a structural
 * type, so the decision logic ("is this tabbable", "where does Tab go next")
 * can be tested without jsdom, and the hook in `hooks/useFocusTrap.ts` is
 * left with nothing but wiring.
 */

export interface FocusableLike {
  tabIndex: number;
  getAttribute(name: string): string | null;
}

/**
 * Move focus to a recovery panel once it mounts. Structural type keeps this
 * DOM-free (and therefore testable); `preventScroll` avoids yanking a
 * screen-reader user around the page when a boundary activates mid-scroll.
 */
export interface RecoveryFocusTarget {
  focus(options?: { preventScroll?: boolean }): void;
}

export function focusRecovery(target: RecoveryFocusTarget | null | undefined): void {
  if (!target || typeof target.focus !== "function") return;
  target.focus({ preventScroll: true });
}

/**
 * Selectors that can receive focus. `contenteditable` and `summary` are the
 * two that are routinely forgotten; `[tabindex]` is last so an author can
 * always override.
 */
export const FOCUSABLE_SELECTOR = [
  "a[href]",
  "area[href]",
  "button:not([disabled])",
  "input:not([disabled])",
  "select:not([disabled])",
  "textarea:not([disabled])",
  "summary",
  "iframe",
  "object",
  "embed",
  "audio[controls]",
  "video[controls]",
  "[contenteditable]:not([contenteditable='false'])",
  "[tabindex]",
].join(",");

/**
 * Why an element is not tabbable. Exposed as a reason rather than a boolean
 * because the three cases need different handling: a `disabled` control is
 * correctly skipped, an element hidden by CSS must be skipped but re-checked
 * if the dialog resizes, and `tabindex="-1"` is programmatic-only.
 */
export type UntabbableReason = "negative-tabindex" | "hidden" | "aria-hidden" | "inert";

export interface TabbabilityInput {
  tabIndex: number;
  disabled?: boolean;
  /** Result of an offsetParent / getClientRects visibility probe. */
  visible?: boolean;
  ariaHidden?: boolean | null;
  inert?: boolean;
}

/**
 * Is this element reachable with the Tab key?
 *
 * `visible` is passed in rather than computed here: reading layout forces a
 * reflow, which in a keydown handler is a jank source, and the caller usually
 * already has the measurement.
 *
 * `display:none` and `visibility:hidden` both remove an element from the tab
 * order; that is the single most common bug in hand-rolled traps — a "hidden"
 * dialog that is still in the DOM still swallows Tab.
 */
export function tabbability(item: TabbabilityInput): UntabbableReason | null {
  if (item.disabled) return "hidden";
  if (item.inert) return "inert";
  if (item.ariaHidden === true) return "aria-hidden";
  if (item.visible === false) return "hidden";
  if (item.tabIndex < 0) return "negative-tabindex";
  return null;
}

export const isTabbable = (item: TabbabilityInput): boolean => tabbability(item) === null;

/**
 * Tab order: positive `tabindex` values first in ascending order, then
 * everything with no/zero tabindex in document order.
 *
 * This is what the spec requires and what every browser does, and getting it
 * wrong in a trap is subtle: the trap believes `items[0]` is the first item,
 * so wrapping at the "edges" lands the user somewhere they cannot predict.
 */
export function orderByTabIndex<T extends FocusableLike>(items: readonly T[]): T[] {
  const positive: Array<{ item: T; index: number }> = [];
  const natural: Array<{ item: T; index: number }> = [];

  items.forEach((item, index) => {
    if (item.tabIndex > 0) positive.push({ item, index });
    else natural.push({ item, index });
  });

  positive.sort((a, b) => a.item.tabIndex - b.item.tabIndex || a.index - b.index);
  return [...positive, ...natural].map((entry) => entry.item);
}

/**
 * Where focus should go when Tab or Shift+Tab is pressed.
 *
 * Returns the index of the next target, wrapping at both ends. Returns `null`
 * when there is nothing to focus, which the hook turns into "focus the
 * container" — the trap must never let focus escape, not even into a dialog
 * that is momentarily empty.
 *
 * `activeIndex` is the currently focused item's index in the ordered list, or
 * `-1` when focus is outside the list entirely (a common state after a
 * re-render removes the focused element).
 */
export function nextFocusIndex(
  count: number,
  activeIndex: number,
  shiftKey: boolean,
): number | null {
  if (count <= 0) return null;

  // Focus escaped, or nothing in the list is focused: pull it back in rather
  // than letting the next Tab keypress go to the browser's default target,
  // which is outside the dialog.
  if (activeIndex < 0 || activeIndex >= count) return shiftKey ? count - 1 : 0;

  if (shiftKey) {
    if (activeIndex === 0) return count - 1;
    return activeIndex - 1;
  }
  if (activeIndex === count - 1) return 0;
  return activeIndex + 1;
}

/**
 * Initial focus for a dialog, following the APG order of preference:
 * an explicit target, then the first element marked `data-autofocus`, then the
 * first tabbable element, then the container itself.
 *
 * The container fallback is why `useFocusTrap` can promise that focus is
 * always inside — an empty dialog still lands somewhere sensible instead of
 * leaving focus on the page behind it.
 */
export type InitialFocus<T extends FocusableLike> = "container" | "first" | "custom" | (() => T | null);

export function resolveInitialFocus<T extends FocusableLike>(
  tabbables: readonly T[],
  requested: InitialFocus<T> | undefined,
  marked: readonly T[],
): "container" | T {
  if (requested === "container" || (!requested && tabbables.length === 0)) {
    return "container";
  }
  if (typeof requested === "function") {
    return requested() ?? "container";
  }
  if (requested === "custom") {
    return marked[0] ?? tabbables[0] ?? "container";
  }
  return tabbables[0] ?? "container";
}

/**
 * Restore focus only if the trigger is still in the document and still
 * focusable. Restoring unconditionally is a common bug: close a dialog whose
 * trigger was deleted in the meantime (a list row, a toast) and focus lands on
 * `<body>`, which silently drops the user at the top of the page.
 */
export function shouldRestoreFocus(target: {
  isConnected: boolean;
  disabled?: boolean;
  tabIndex: number;
  visible?: boolean;
} | null | undefined): boolean {
  if (!target) return false;
  if (!target.isConnected) return false;
  if (target.disabled) return false;
  if (target.tabIndex < 0) return false;
  if (target.visible === false) return false;
  return true;
}