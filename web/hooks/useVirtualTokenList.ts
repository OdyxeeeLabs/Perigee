import { RefObject } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";

import {
  DEFAULT_TOKEN_DROPDOWN_HEIGHT,
  TOKEN_OVERSCAN,
  TOKEN_ROW_HEIGHT,
} from "@/constant/vault.constants";

/**
 * Virtualiser for token selector dropdowns.
 *
 * Handles lists of 500+ tokens without degrading frame rate. Each token row
 * has a fixed height so the static estimateSize is sufficient; dynamic
 * measurement is still wired up to handle custom token logos that may change
 * row height at runtime.
 *
 * Resolves #687 — Implement virtualised lists (token selector).
 */
export function useVirtualTokenList(
  count: number,
  parentRef: RefObject<HTMLDivElement>,
) {
  const rowVirtualizer = useVirtualizer({
    count,

    getScrollElement: () => parentRef.current,

    estimateSize: () => TOKEN_ROW_HEIGHT,

    overscan: TOKEN_OVERSCAN,

    measureElement:
      typeof window !== "undefined"
        ? (element) => element.getBoundingClientRect().height
        : undefined,
  });

  return {
    rowVirtualizer,
    containerHeight: DEFAULT_TOKEN_DROPDOWN_HEIGHT,
  };
}
