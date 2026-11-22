"use client";

import { useEffect } from "react";
import { X } from "lucide-react";
import type { Vault, VaultDetail } from "@/types/vault";
import { useVaultDetail } from "@/hooks/useVaultDetail";
import { useFocusTrap } from "@/hooks/useFocusTrap";
import { useTransactionNetwork } from "@/hooks/useTransactionNetwork";
import { TransactionNetworkNotice } from "@/components/TransactionNetworkNotice";
import { VaultDetailContent } from "./VaultDetailContent";
import { VaultDetailSkeleton } from "./VaultDetailSkeleton";

export interface VaultDetailModalProps {
  /** The row that was clicked. Only `id`/`name` are used before detail loads. */
  vault: Pick<Vault, "id" | "name"> | null;
  onClose: () => void;
  onDeposit?: (vault: VaultDetail) => void;
  onWithdraw?: (vault: VaultDetail) => void;
  onManage?: (vault: VaultDetail) => void;
}

/**
 * Expandable vault detail modal — issue #684.
 *
 * Data loads on demand via `useVaultDetail`, which only fires once `vault`
 * is non-null (i.e. once the modal actually opens) — see that hook's doc
 * comment. Ships with its own matching skeleton (`VaultDetailSkeleton`,
 * issue #685) instead of a spinner, and reuses the shared focus-trap hook
 * (`useFocusTrap`, issue #703) instead of a bespoke implementation.
 */
export function VaultDetailModal({ vault, onClose, onDeposit, onWithdraw, onManage }: VaultDetailModalProps) {
  const isOpen = vault !== null;
  const detail = useVaultDetail(vault?.id ?? null);
  const network = useTransactionNetwork();
  const { ref, focusInitial } = useFocusTrap<HTMLDivElement>({
    active: isOpen,
    onEscape: onClose,
  });

  // Re-focus the dialog whenever the content changes from skeleton -> loaded,
  // since the focusable elements inside it did not exist at mount time.
  useEffect(() => {
    if (isOpen && detail.status === "success") focusInitial();
  }, [isOpen, detail.status, focusInitial]);

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-start justify-center overflow-y-auto bg-slate-950/70 p-4 sm:items-center">
      <button
        type="button"
        aria-label="Close vault details"
        onClick={onClose}
        className="absolute inset-0 cursor-default"
        tabIndex={-1}
      />
      <div
        ref={ref}
        role="dialog"
        aria-modal="true"
        aria-labelledby="vault-detail-modal-title"
        aria-describedby="vault-detail-modal-description"
        className="relative z-10 w-full max-w-2xl rounded-xl border border-slate-800 bg-slate-950 shadow-2xl"
      >
        <header className="flex items-center justify-between border-b border-slate-800 px-6 py-4">
          <div>
            <h2 id="vault-detail-modal-title" className="text-lg font-semibold text-slate-100">
              {vault.name}
            </h2>
            <p id="vault-detail-modal-description" className="text-xs text-slate-500">Vault details</p>
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label="Close"
            className="rounded p-1.5 text-slate-400 hover:bg-slate-800 hover:text-slate-200"
          >
            <X size={18} />
          </button>
        </header>

        <div className="max-h-[70vh] overflow-y-auto px-6 py-5">
          <TransactionNetworkNotice network={network} />
          {detail.status === "loading" && <VaultDetailSkeleton />}

          {detail.status === "error" && (
            <div role="alert" className="rounded-lg border border-rose-900/60 bg-rose-950/40 p-4 text-sm text-rose-300">
              Could not load vault details: {detail.error}
            </div>
          )}

          {detail.status === "success" && (
            <VaultDetailContent
              vault={detail.data}
              onDeposit={onDeposit}
              onWithdraw={onWithdraw}
              onManage={onManage}
              transactionsEnabled={network.canTransact}
              disabledReason={network.disabledReason}
            />
          )}
        </div>
      </div>
    </div>
  );
}
