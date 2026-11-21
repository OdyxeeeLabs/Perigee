import { useCallback, useEffect, useRef, useState, type KeyboardEvent } from "react";

import { Vault } from "@/types/vault";

import { useRovingTabIndex } from "@/hooks/useRovingTabIndex";
import { useVirtualVaultList } from "@/hooks/useVirtualVaultList";
import { nextRovingIndex, type RovingKey } from "@/lib/a11y/roving-tabindex";

import { VaultRow } from "../VaultRow";

interface VaultListProps {
  vaults: Vault[];

  onVaultClick?(vault: Vault): void;
}

/** Column headers for the vault table — kept in sync with VaultRow layout. */
function VaultTableHeader() {
  return (
    <div
      className="
        sticky
        top-0
        z-10
        hidden
        grid
        grid-cols-[1fr_auto_auto_auto]
        gap-4
        border-b
        border-slate-700
        bg-slate-900
        px-4
        py-2
        text-xs
        font-semibold
        uppercase
        tracking-wider
        text-slate-400
        sm:grid
      "
    >
      <span>Vault</span>
      <span className="text-end">Balance</span>
      <span className="text-end">APY</span>
      <span className="text-end">Status</span>
    </div>
  );
}

/**
 * Virtualised vault table.
 *
 * Uses @tanstack/react-virtual to render only the rows visible in the
 * scrollable window, keeping DOM node count O(1) regardless of how many
 * vaults are loaded. Supports dynamic item heights via the `measureElement`
 * callback passed to the virtualiser.
 *
 * Resolves #687 — Implement virtualised lists.
 */
export function VaultList({
  vaults,
  onVaultClick,
}: VaultListProps) {
  const parentRef = useRef<HTMLDivElement>(null);
  const [activeIndex, setActiveIndex] = useState(0);
  const { containerRef, getItemProps, focusItem } = useRovingTabIndex(
    vaults.length,
    { orientation: "vertical", wrap: false },
  );

  const { rowVirtualizer, containerHeight } = useVirtualVaultList(
    vaults.length,
    parentRef,
  );
  const virtualItems = rowVirtualizer.getVirtualItems();
  const [pendingFocusIndex, setPendingFocusIndex] = useState<number | null>(null);

  useEffect(() => {
    setActiveIndex((current) => Math.min(current, Math.max(vaults.length - 1, 0)));
  }, [vaults.length]);

  useEffect(() => {
    if (pendingFocusIndex === null) return;
    if (!virtualItems.some((item) => item.index === pendingFocusIndex)) return;
    focusItem(pendingFocusIndex);
    setPendingFocusIndex(null);
  }, [focusItem, pendingFocusIndex, virtualItems]);

  const setScrollRef = useCallback(
    (node: HTMLDivElement | null) => {
      parentRef.current = node;
      containerRef.current = node;
    },
    [containerRef],
  );

  /**
   * Attach the virtualiser's measurement callback to each row element so
   * rows with variable content (e.g. truncated addresses) report their true
   * rendered height rather than relying on the static estimate.
   */
  const measureRef = useCallback(
    (node: HTMLDivElement | null) => {
      if (node) {
        rowVirtualizer.measureElement(node);
      }
    },
    [rowVirtualizer],
  );

  const moveFocus = useCallback(
    (index: number) => {
      setActiveIndex(index);
      setPendingFocusIndex(index);
      rowVirtualizer.scrollToIndex(index);
    },
    [rowVirtualizer],
  );

  if (vaults.length === 0) {
    return (
      <div className="flex h-40 items-center justify-center text-sm text-slate-500">
        No vaults found.
      </div>
    );
  }

  return (
    <div className="overflow-hidden rounded-xl border border-slate-800">
      <VaultTableHeader />

      {/* Scrollable virtualised container */}
      <div
        ref={setScrollRef}
        style={{
          height: containerHeight,
          overflow: "auto",
        }}
        aria-label="Vault list"
        role="list"
      >
        {/* Spacer div that gives the scrollbar its full range */}
        <div
          style={{
            height: rowVirtualizer.getTotalSize(),
            position: "relative",
          }}
        >
          {virtualItems.map((virtualRow) => {
            const vault = vaults[virtualRow.index];
            const index = virtualRow.index;
            const roving = getItemProps(index);

            const handleKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
              if (
                event.key === "ArrowDown" ||
                event.key === "ArrowUp" ||
                event.key === "Home" ||
                event.key === "End"
              ) {
                event.preventDefault();
                const next = nextRovingIndex(
                  event.key as RovingKey,
                  index,
                  vaults.length,
                  "vertical",
                  { wrap: false },
                );
                moveFocus(next);
                return;
              }
              roving.onKeyDown(event);
            };

            return (
              <div
                key={vault.id}
                ref={measureRef}
                data-index={index}
                style={{
                  position: "absolute",
                  insetInlineStart: 0,
                  top: 0,
                  width: "100%",
                  transform: `translateY(${virtualRow.start}px)`,
                }}
                role="listitem"
              >
                <VaultRow
                  vault={vault}
                  href={`/vaults/${vault.id}`}
                  onClick={onVaultClick}
                  rowRef={roving.ref}
                  tabIndex={activeIndex === index ? 0 : -1}
                  onKeyDown={handleKeyDown}
                  onFocus={() => {
                    roving.onFocus();
                    setActiveIndex(index);
                  }}
                />
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}