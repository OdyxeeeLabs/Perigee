"use client";

import { useVaultDetail } from "@/hooks/useVaultDetail";
import { useTransactionNetwork } from "@/hooks/useTransactionNetwork";
import { TransactionNetworkNotice } from "@/components/TransactionNetworkNotice";
import { VaultDetailContent } from "./VaultDetailContent";
import { VaultDetailSkeleton } from "./VaultDetailSkeleton";
import { useToast } from "@/context/ToastContext";
import type { VaultDetail } from "@/types/vault";

/**
 * Full-page vault detail — issue #684.
 *
 * Shares `VaultDetailContent`/`VaultDetailSkeleton` with `VaultDetailModal`
 * so the modal (list -> quick look) and this deep-linkable page (direct
 * navigation, shared links) never render different data or drift out of
 * sync with each other.
 */
export function VaultDetailPageClient({ id }: { id: string }) {
  const detail = useVaultDetail(id);
  const toast = useToast();
  const network = useTransactionNetwork();

  const handleAction = (action: "deposit" | "withdraw" | "manage") => (vault: VaultDetail) => {
    toast.info({
      title: `${action[0].toUpperCase()}${action.slice(1)} not yet available`,
      description: `${vault.name}: this is a UI preview — transaction building is out of scope for #684.`,
    });
  };

  return (
    <div className="mx-auto w-full max-w-3xl px-4 py-8 text-slate-100">
      <TransactionNetworkNotice network={network} />
      {detail.status === "loading" && (
        <>
          <div className="mb-6 h-7 w-48 animate-pulse rounded bg-slate-800" />
          <VaultDetailSkeleton />
        </>
      )}

      {detail.status === "error" && (
        <div role="alert" className="rounded-lg border border-rose-900/60 bg-rose-950/40 p-4 text-sm text-rose-300">
          Could not load vault {id}: {detail.error}
        </div>
      )}

      {detail.status === "success" && (
        <>
          <header className="mb-6">
            <h1 className="text-2xl font-bold text-slate-100">{detail.data.name}</h1>
            <p className="mt-1 text-xs text-slate-500">{detail.data.owner}</p>
          </header>
          <VaultDetailContent
            vault={detail.data}
            onDeposit={handleAction("deposit")}
            onWithdraw={handleAction("withdraw")}
            onManage={handleAction("manage")}
            transactionsEnabled={network.canTransact}
            disabledReason={network.disabledReason}
          />
        </>
      )}
    </div>
  );
}
