"use client";

import { useCallback, useEffect, useRef, type RefObject } from "react";
import {
  FOCUSABLE_SELECTOR,
  nextFocusIndex,
  orderByTabIndex,
  pickFocusRestoreTarget,
  resolveInitialFocus,
  tabbability,
  type FocusableLike,
  type InitialFocus,
} from "@/lib/a11y/focus";

export interface FocusTrapOptions {
  /** Whether the trap is active. The element stays mounted; only the trap toggles. */
  active?: boolean;
  /** Called on Escape. Omit to leave Escape alone. */
  onEscape?: () => void;
  /**
   * Where focus goes on open. `"container"` focuses the dialog itself, which
   * is correct for a confirmation whose first control is destructive.
   */
  initialFocus?: InitialFocus<HTMLElement>;
  /**
   * Restore focus to whatever was focused before opening. Defaults to true —
   * WCAG 2.4.3. Set false only for a trap the user did not open by keyboard.
   */
  restoreFocus?: boolean;
  /** Set false for panels that should not capture Tab. */
  trapTab?: boolean;
}

export interface FocusTrap<T extends HTMLElement = HTMLElement> {
  /** Attach to the dialog element. */
  ref: RefObject<T | null>;
  /** Focus the first appropriate element now (e.g. after async content loads). */
  focusInitial: () => void;
}

/**
 * Reusable focus trap for dialogs, drawers and popovers — issue #703.
 *
 * Replaces the inline implementation in `NavDrawer`, which had three defects
 * worth not reproducing elsewhere:
 *
 * 1. It selected focusable elements with `querySelectorAll` and no visibility
 *    filter, so a control hidden with `display:none` or `visibility:hidden`
 *    was still in the trap. Tab would move focus to something invisible.
 * 2. It ordered by document position only, ignoring positive `tabindex`,
 *    which is the order the browser actually uses. The trap's idea of
 *    "first" and "last" therefore disagreed with the page's, and wrapping
 *    landed the user somewhere unpredictable.
 * 3. If the dialog contained no focusable element at all, `Tab` was left
 *    alone — so focus walked straight out of the dialog into the page behind
 *    it. WCAG 2.4.3 does not care that the dialog is empty.
 *
 * The keydown listener is on `document`, not the dialog, so Escape still works
 * after a re-render moves focus out of it. Focusables are re-queried on each
 * keypress rather than cached, because dialog contents change while open (an
 * async list, a step in a wizard).
 */
export function useFocusTrap<T extends HTMLElement = HTMLElement>(
  options: FocusTrapOptions = {},
): FocusTrap<T> {
  const {
    active = true,
    onEscape,
    initialFocus,
    restoreFocus = true,
    trapTab = true,
  } = options;

  const ref = useRef<T>(null);
  const previouslyFocused = useRef<HTMLElement | null>(null);

  const restoreFocusTarget = useCallback((target: HTMLElement | null) => {
    const fallback =
      document.getElementById("main-content") ??
      document.querySelector<HTMLElement>("[data-focus-fallback]") ??
      document.querySelector<HTMLElement>("main");
    const destination = pickFocusRestoreTarget<HTMLElement>(target, fallback);
    if (!destination) return;
    if (destination.tabIndex < 0 && !destination.hasAttribute("tabindex")) {
      destination.setAttribute("tabindex", "-1");
    }
    destination.focus();
  }, []);

  const focusInitial = useCallback(() => {
    const container = ref.current;
    if (!container) return;
    const target = initialFocusTarget(container, initialFocus);
    if (target === container) {
      // A container that is not focusable cannot receive focus programmatically,
      // which is the usual reason a dialog silently fails to trap.
      if (!container.hasAttribute("tabindex")) container.setAttribute("tabindex", "-1");
      container.focus();
      return;
    }
    target.focus();
  }, [initialFocus]);

  // Remember the trigger before focus moves, and restore it on close.
  useEffect(() => {
    if (!active) return;
    previouslyFocused.current =
      document.activeElement instanceof HTMLElement ? document.activeElement : null;
    focusInitial();

    return () => {
      if (!restoreFocus) return;
      restoreFocusTarget(previouslyFocused.current);
    };
  }, [active, restoreFocus, focusInitial, restoreFocusTarget]);

  useEffect(() => {
    if (!active) return;

    const onKeyDown = (event: KeyboardEvent) => {
      const container = ref.current;

      if (event.key === "Escape") {
        // Only a trap that contains focus should react, so two stacked dialogs
        // do not both close on one Escape.
        if (!onEscape || !container) return;
        if (!(document.activeElement instanceof Node) || !container.contains(document.activeElement)) {
          return;
        }
        event.stopPropagation();
        onEscape();
        return;
      }

      if (event.key !== "Tab" || !trapTab || !container) return;

      const items = getFocusableElements(container);
      const activeIndex = items.findIndex(
        (el) => el === document.activeElement || el.contains(document.activeElement),
      );

      if (items.length === 0) {
        // Nothing to focus, including the container: swallow the keypress so
        // Tab cannot reach the page behind the dialog.
        event.preventDefault();
        makeContainerFocusable(container).focus();
        return;
      }

      const target = nextFocusIndex(items.length, activeIndex, event.shiftKey);
      if (target === null) {
        event.preventDefault();
        return;
      }

      // Only intervene where the browser cannot: the two wrap points, and the
      // case where focus has escaped the list (after a re-render removes the
      // focused node). Anywhere else the browser's own Tab order is correct
      // and cheaper.
      const mustWrap =
        activeIndex < 0 || (event.shiftKey ? activeIndex === 0 : activeIndex === items.length - 1);

      if (!mustWrap) return;

      event.preventDefault();
      items[target].focus();
    };

    document.addEventListener("keydown", onKeyDown, true);
    return () => document.removeEventListener("keydown", onKeyDown, true);
  }, [active, onEscape, trapTab]);

  return { ref, focusInitial };
}

const makeContainerFocusable = (container: HTMLElement): HTMLElement => {
  if (!container.hasAttribute("tabindex")) container.setAttribute("tabindex", "-1");
  return container;
};

/**
 * Is this element rendered?
 *
 * `getClientRects()` rather than `offsetParent`: a `position: fixed` dialog
 * has a null `offsetParent` and would be reported as hidden, which would
 * disable its own trap.
 */
const isRendered = (element: HTMLElement): boolean => {
  if (element.hidden) return false;
  const style = typeof window !== "undefined" ? window.getComputedStyle?.(element) : null;
  if (style && (style.display === "none" || style.visibility === "hidden")) return false;
  if (typeof element.getClientRects === "function" && element.getClientRects().length === 0) {
    // Closed `<details>`, or an ancestor that is display:none.
    return false;
  }
  return true;
};

/**
 * DOM adapter for `tabbability` from `lib/a11y/focus`. An ancestor's
 * `aria-hidden` or `inert` removes the whole subtree from the tab order, so
 * the check has to walk up, not just look at the element.
 */
export const isFocusableElement = (element: HTMLElement): boolean => {
  const ariaHidden = element.closest('[aria-hidden="true"]') !== null;
  const inert = element.closest("[inert]") !== null;
  const disabled =
    (element as HTMLButtonElement).disabled === true ||
    element.getAttribute("aria-disabled") === "true";

  return (
    tabbability({
      tabIndex: element.tabIndex,
      disabled,
      visible: isRendered(element),
      ariaHidden,
      inert,
    }) === null
  );
};

/** Focusable elements inside `container`, in true tab order. */
export function getFocusableElements(container: HTMLElement | null): HTMLElement[] {
  if (!container) return [];
  return orderByTabIndex(
    Array.from(container.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)).filter(
      isFocusableElement,
    ),
  );
}

/**
 * Where focus should land on open. Follows the APG order of preference:
 * explicit target, then `[data-autofocus]`, then the first tabbable element,
 * then the container itself.
 */
export function initialFocusTarget(
  container: HTMLElement,
  requested?: InitialFocus<HTMLElement>,
): HTMLElement {
  const items = getFocusableElements(container);
  const marked = items.filter((el) => el.getAttribute("data-autofocus") !== null);
  const resolved = resolveInitialFocus<FocusableLike & HTMLElement>(items, requested, marked);
  return resolved === "container" ? container : resolved;
}

export default useFocusTrap;