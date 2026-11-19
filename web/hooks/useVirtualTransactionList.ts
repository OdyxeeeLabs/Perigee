import { RefObject } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";

import {
  DEFAULT_TRANSACTION_HEIGHT,
  TRANSACTION_OVERSCAN,
  TRANSACTION_ROW_HEIGHT,
} from "@/constant/vault.constants";

/**
 * Virtualiser for the transaction history list.
 *
 * Designed for lists of 1 000+ rows. Uses dynamic height measurement so rows
 * that wrap onto multiple lines (e.g. long memo fields) are handled correctly.
 *
 * Resolves #687 — Implement virtualised lists (transaction history).
 */
export function useVirtualTransactionList(
  count: number,
  parentRef: RefObject<HTMLDivElement>,
) {
  const rowVirtualizer = useVirtualizer({
    count,

    getScrollElement: () => parentRef.current,

    estimateSize: () => TRANSACTION_ROW_HEIGHT,

    overscan: TRANSACTION_OVERSCAN,

    measureElement:
      typeof window !== "undefined"
        ? (element) => element.getBoundingClientRect().height
        : undefined,
  });

  return {
    rowVirtualizer,
    containerHeight: DEFAULT_TRANSACTION_HEIGHT,
  };
}
