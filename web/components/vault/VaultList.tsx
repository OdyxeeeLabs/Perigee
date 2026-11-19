import { useRef, useCallback } from "react";

import { Vault } from "@/types/vault";

import { useVirtualVaultList } from "@/hooks/useVirtualVaultList";

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

  const { rowVirtualizer, containerHeight } = useVirtualVaultList(
    vaults.length,
    parentRef,
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
        ref={parentRef}
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
          {rowVirtualizer.getVirtualItems().map((virtualRow) => {
            const vault = vaults[virtualRow.index];

            return (
              <div
                key={vault.id}
                ref={measureRef}
                data-index={virtualRow.index}
                style={{
                  position: "absolute",
                  insetInlineStart: 0,
                  top: 0,
                  width: "100%",
                  transform: `translateY(${virtualRow.start}px)`,
                }}
                role="listitem"
              >
                {/* onVaultClick (#684) opens the vault detail modal/drawer. */}
                <VaultRow vault={vault} onClick={onVaultClick} />
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}