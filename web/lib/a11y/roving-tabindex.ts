/**
 * Roving tabindex maths for composite widgets — issue #698.
 *
 * A composite widget (tab list, menu, toolbar, radio group) is a single tab
 * stop: the container holds `tabIndex={0}` and exactly one child holds
 * `tabIndex={0}`, the rest `tabIndex={-1}`. Arrow keys then move the active
 * child *and* the focus. Without this, a tab bar with six tabs costs six Tab
 * presses to cross and arrow keys do nothing.
 *
 * The logic is pure and dependency-free so it can be unit tested; the React
 * binding is `hooks/useRovingTabIndex.ts`.
 *
 * @see https://www.w3.org/WAI/ARIA/apg/patterns/tabs/
 * @see https://www.w3.org/WAI/ARIA/apg/practices/keyboard-interface/
 */

export type Orientation = "horizontal" | "vertical" | "both";

export type RovingKey =
  | "ArrowRight"
  | "ArrowLeft"
  | "ArrowDown"
  | "ArrowUp"
  | "Home"
  | "End";

/** Keys the widget consumes; anything else falls through to the page. */
export function handlesKey(key: string, orientation: Orientation = "horizontal"): boolean {
  switch (key) {
    case "Home":
    case "End":
      return true;
    case "ArrowRight":
      return orientation === "horizontal" || orientation === "both";
    case "ArrowLeft":
      return orientation === "horizontal" || orientation === "both";
    case "ArrowDown":
      return orientation === "vertical" || orientation === "both";
    case "ArrowUp":
      return orientation === "vertical" || orientation === "both";
    default:
      return false;
  }
}

export interface NextIndexOptions {
  /** Roving tabindex wraps around; grid patterns often do not. */
  wrap?: boolean;
  /** RTL layouts swap the meaning of the inline arrows. */
  rtl?: boolean;
  /** Indices to skip (disabled, filtered out). */
  isDisabled?: (index: number) => boolean;
}

/**
 * Returns the index the roving tabindex should move to, or `current` when the
 * key is not ours or no enabled item is available in that direction.
 */
export function nextRovingIndex(
  key: RovingKey,
  current: number,
  count: number,
  orientation: Orientation = "horizontal",
  options: NextIndexOptions = {},
): number {
  const { wrap = true, rtl = false, isDisabled } = options;

  if (count <= 0) return current;
  if (!handlesKey(key, orientation)) return current;

  const enabled = (index: number) => (isDisabled ? !isDisabled(index) : true);
  const at = (index: number) => ((index % count) + count) % count;

  // Home/End jump to the first/last enabled item, not just index 0/count-1.
  if (key === "Home") {
    for (let step = 0; step < count; step += 1) {
      if (enabled(step)) return step;
    }
    return current;
  }

  if (key === "End") {
    for (let step = count - 1; step >= 0; step -= 1) {
      if (enabled(step)) return step;
    }
    return current;
  }

  // In RTL, ArrowRight means "towards the start" — the inverse of LTR.
  const forward = key === "ArrowRight" ? !rtl : key === "ArrowDown";
  const backward = !forward;

  let index = current;
  for (let step = 0; step < count; step += 1) {
    index = at(index + (forward ? 1 : -1));
    if (enabled(index)) return index;
    if (!wrap && ((forward && index >= current) || (!forward && index <= current))) {
      return current;
    }
  }

  return current;
}

/** The `tabIndex` to place on the item at `index`, given the active one. */
export function rovingTabIndexFor(index: number, activeIndex: number): 0 | -1 {
  return index === activeIndex ? 0 : -1;
}

/**
 * Whether a key should activate an item. Buttons and links handle Enter and
 * Space natively; `role="button"` and `role="option"` on a non-button element
 * have to do it themselves, and must not fire on the key-up for Space (which
 * is when a native button fires) to avoid double activation.
 */
export function shouldActivate(key: string, phase: "keydown" | "keyup" = "keydown"): boolean {
  if (key === "Enter") return true;
  if (key === " " || key === "Spacebar") return phase === "keydown";
  return false;
}
