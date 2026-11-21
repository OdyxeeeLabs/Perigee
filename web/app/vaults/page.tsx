"use client";

import { useEffect, useState } from "react";
import type { Vault, VaultDetail } from "@/types/vault";
import { managerService, vaultService } from "@/lib/api";
import { vaultFromRecord } from "@/lib/vault-metadata";
import { VaultList } from "@/components/vault/VaultList";
import { VaultListSkeleton } from "@/components/vault/VaultCardSkeleton";
import { VaultDetailModal } from "@/components/vault/VaultDetailModal";
import { useToast } from "@/context/ToastContext";
import { useWalletStore } from "@/context/WalletContext";

type VaultListState =
  | { status: "loading" }
  | { status: "empty" }
  | { status: "error"; message: string; action: "connect" | "retry" }
  | { status: "populated"; vaults: Vault[] };

export default function VaultsPage() {
  const address = useWalletStore((wallet) => wallet.address);
  const openWalletModal = useWalletStore((wallet) => wallet.openModal);
  const [state, setState] = useState<VaultListState>({ status: "loading" });
  const [retryCount, setRetryCount] = useState(0);
  const [selected, setSelected] = useState<Pick<Vault, "id" | "name"> | null>(null);
  const toast = useToast();

  useEffect(() => {
    let cancelled = false;

    if (!address) {
      setState({
        status: "error",
        message: "Connect your wallet to view your vaults.",
        action: "connect",
      });
      return () => {
        cancelled = true;
      };
    }

    setState({ status: "loading" });
    const loadVaults = async () => {
      try {
        const manager = await managerService.checkStatus(address);
        if (manager.status !== "approved") {
          if (!cancelled) setState({ status: "empty" });
          return;
        }
        if (!manager.id.trim()) {
          throw new Error("Approved manager response did not include an ID");
        }

        const records = await vaultService.list(manager.id);
        if (cancelled) return;

        const vaults = records.map(vaultFromRecord);
        setState(vaults.length > 0
          ? { status: "populated", vaults }
          : { status: "empty" });
      } catch {
        if (!cancelled) {
          setState({
            status: "error",
            message: "We couldn't load your vaults. Please try again.",
            action: "retry",
          });
        }
      }
    };

    void loadVaults();
    return () => {
      cancelled = true;
    };
  }, [address, retryCount]);

  const handleAction = (action: "deposit" | "withdraw" | "manage") => (vault: VaultDetail) => {
    toast.info({
      title: `${action[0].toUpperCase()}${action.slice(1)} not yet available`,
      description: `${vault.name}: this is a UI preview — transaction building is out of scope for #684.`,
    });
  };

  return (
    <div className="mx-auto w-full max-w-5xl px-4 py-8 text-slate-100">
      <header className="mb-6">
        <h1 className="text-2xl font-bold text-slate-100">Vaults</h1>
        <p className="mt-1 text-sm text-slate-400">Select a vault to view its full details.</p>
      </header>

      {state.status === "loading" && <VaultListSkeleton />}

      {state.status === "empty" && (
        <div className="flex min-h-40 items-center justify-center rounded-xl border border-slate-800 text-sm text-slate-400">
          No vaults found for this account.
        </div>
      )}

      {state.status === "error" && (
        <div
          role="alert"
          className="flex flex-col items-start gap-3 rounded-xl border border-rose-900/60 bg-rose-950/40 p-4 text-sm text-rose-300 sm:flex-row sm:items-center sm:justify-between"
        >
          <p>{state.message}</p>
          <button
            type="button"
            onClick={() => state.action === "connect"
              ? openWalletModal()
              : setRetryCount((count) => count + 1)}
            className="min-h-10 rounded-lg border border-rose-800 px-3 py-2 font-medium hover:bg-rose-900/40"
          >
            {state.action === "connect" ? "Connect wallet" : "Try again"}
          </button>
        </div>
      )}

      {state.status === "populated" && (
        <VaultList
          vaults={state.vaults}
          onVaultClick={(vault) => setSelected(vault)}
        />
      )}

      <VaultDetailModal
        vault={selected}
        onClose={() => setSelected(null)}
        onDeposit={handleAction("deposit")}
        onWithdraw={handleAction("withdraw")}
        onManage={handleAction("manage")}
      />
    </div>
  );
}
