"use client";

import { useCallback, useEffect, useRef } from "react";
import {
  handlesKey,
  nextRovingIndex,
  rovingTabIndexFor,
  shouldActivate,
  type Orientation,
  type RovingKey,
} from "@/lib/a11y/roving-tabindex";

/**
 * Roving tabindex for a composite widget — issue #698.
 *
 * Implements the APG keyboard pattern for tab lists, menus, toolbars and
 * radio groups: the group is a single Tab stop, arrows move between items
 * (optionally skipping disabled ones), Home/End jump to the ends, and Enter
 * or Space activates. The pure maths live in `lib/a11y/roving-tabindex.ts`
 * and are unit tested there.
 *
 * @param itemCount number of focusable items in DOM order
 * @param orientation which arrow keys the widget owns
 * @param wrap      false for grids, where the ends are hard stops
 * @param rtl        true to mirror the inline arrows (#699)
 * @param onActivate called with the activated index
 * @returns refs for the container and each item, plus the active index
 */
export function useRovingTabIndex(
  itemCount: number,
  options: {
    orientation?: Orientation;
    wrap?: boolean;
    rtl?: boolean;
    isDisabled?: (index: number) => boolean;
    defaultIndex?: number;
    onActivate?: (index: number) => void;
  } = {},
) {
  const {
    orientation = "horizontal",
    wrap = true,
    rtl = false,
    isDisabled,
    defaultIndex = 0,
    onActivate,
  } = options;

  const containerRef = useRef<HTMLElement | null>(null);
  const itemRefs = useRef<Array<HTMLElement | null>>([]);
  const activeRef = useRef(defaultIndex);
  const onActivateRef = useRef(onActivate);
  onActivateRef.current = onActivate;

  // Keep the ref array in step with the item count.
  useEffect(() => {
    itemRefs.current = itemRefs.current.slice(0, itemCount);
  }, [itemCount]);

  const focusItem = useCallback((index: number) => {
    activeRef.current = index;
    itemRefs.current[index]?.focus();
  }, []);

  const handleKeyDown = useCallback(
    (event: React.KeyboardEvent, index: number) => {
      if (itemCount <= 0) return;

      if (shouldActivate(event.key)) {
        // Only non-native activators reach this for Space; native buttons
        // already handle it, and the event is not prevented so that click
        // still fires exactly once.
        if (event.target instanceof HTMLElement && event.target.tagName !== "BUTTON") {
          event.preventDefault();
          onActivateRef.current?.(index);
        }
        return;
      }

      if (!handlesKey(event.key, orientation)) return;
      if (event.altKey || event.ctrlKey || event.metaKey) return;

      const next = nextRovingIndex(
        event.key as RovingKey,
        index,
        itemCount,
        orientation,
        { wrap, rtl, isDisabled },
      );

      if (next === index) return;
      event.preventDefault();
      focusItem(next);
    },
    [itemCount, orientation, wrap, rtl, isDisabled, focusItem],
  );

  const getItemProps = useCallback(
    (index: number) => ({
      ref: (node: HTMLElement | null) => {
        itemRefs.current[index] = node;
      },
      tabIndex: rovingTabIndexFor(index, activeRef.current) as 0 | -1,
      onKeyDown: (event: React.KeyboardEvent) => handleKeyDown(event, index),
      onFocus: () => {
        // Following focus with the arrows (or Tab into the group) must also
        // move the roving tab stop, or Tab would re-enter at the old item.
        activeRef.current = index;
      },
    }),
    [handleKeyDown],
  );

  return {
    containerRef,
    getItemProps,
    focusItem,
    activeIndex: activeRef.current,
  };
}

export default useRovingTabIndex;
