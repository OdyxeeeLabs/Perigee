"use client";

import { useEffect, useState } from "react";
import type { Vault, VaultDetail } from "@/types/vault";
import { fetchVaultData } from "@/lib/vault-metadata";
import { VaultList } from "@/components/vault/VaultList";
import { VaultListSkeleton } from "@/components/vault/VaultCardSkeleton";
import { VaultDetailModal } from "@/components/vault/VaultDetailModal";
import { useToast } from "@/context/ToastContext";

// Demo vault ids — there is no "list all vaults" endpoint wired up yet
// (`vaultService.list` requires an authenticated manager id); `fetchVaultData`
// already has graceful mock fallback per-id (see lib/vault-metadata.ts), so a
// small fixed id set is enough to demonstrate the list -> detail modal flow.
const DEMO_VAULT_IDS = ["1", "2", "3", "4", "5"];

export default function VaultsPage() {
  const [vaults, setVaults] = useState<Vault[] | null>(null);
  const [selected, setSelected] = useState<Pick<Vault, "id" | "name"> | null>(null);
  const toast = useToast();

  useEffect(() => {
    let cancelled = false;
    Promise.all(DEMO_VAULT_IDS.map((id) => fetchVaultData(id))).then((results) => {
      if (cancelled) return;
      setVaults(results.filter((v): v is Vault => v !== null));
    });
    return () => {
      cancelled = true;
    };
  }, []);

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

      {vaults === null ? (
        <VaultListSkeleton count={DEMO_VAULT_IDS.length} />
      ) : (
        <VaultList vaults={vaults} onVaultClick={(vault) => setSelected(vault)} />
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
