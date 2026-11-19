import { useVirtualizer } from "@tanstack/react-virtual";
import { type RefObject } from "react";

import {
  DEFAULT_VIRTUAL_HEIGHT,
  VAULT_OVERSCAN,
  VAULT_ROW_HEIGHT,
} from "@/constant/vault.constants";

/**
 * Provides a TanStack Virtual `rowVirtualizer` configured for the vault table.
 *
 * Dynamic heights are enabled via `measureElement` so rows with variable
 * content (multi-line names, long addresses) are measured correctly at runtime
 * rather than relying solely on the static `estimateSize` fallback.
 *
 * Resolves #687 — Implement virtualised lists.
 */
export function useVirtualVaultList(
  count: number,
  parentRef: RefObject<HTMLDivElement | null>,
) {
  const rowVirtualizer = useVirtualizer({
    count,
    getScrollElement: () => parentRef.current,
    
    /**
     * Initial height estimate used before the element is measured.
     * Kept close to the real value to avoid layout thrash on first render.
     */
    estimateSize: () => VAULT_ROW_HEIGHT,

    overscan: VAULT_OVERSCAN,

    /**
     * Enable auto-measurement of rendered items.
     * Requires `data-index` and a `ref={rowVirtualizer.measureElement}`
     * on each row element — see VaultList.tsx.
     */
    measureElement:
      typeof window !== "undefined"
        ? (element) => element.getBoundingClientRect().height
        : undefined,
  });

  return {
    rowVirtualizer,
    containerHeight: DEFAULT_VIRTUAL_HEIGHT,
  };
}
