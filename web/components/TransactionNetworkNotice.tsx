"use client";

import type { TransactionNetworkStatus } from "../hooks/useTransactionNetwork";

export function TransactionNetworkNotice({
  network,
}: {
  network: TransactionNetworkStatus;
}) {
  if (network.canTransact) return null;

  return (
    <div
      role="status"
      aria-live="polite"
      data-testid="transaction-network-notice"
      className="mb-4 rounded-lg border border-amber-800/70 bg-amber-950/30 p-3 text-xs text-amber-200"
    >
      <p>{network.disabledReason}</p>
      <div className="mt-2 flex flex-wrap items-center gap-3">
        {network.showingCachedData && (
          <span
            data-testid="cached-data-badge"
            className="inline-flex items-center gap-1 rounded-full border border-amber-700/60 bg-amber-900/60 px-2 py-0.5 text-[10px] font-medium"
          >
            Cached data — read only
          </span>
        )}
        <button
          type="button"
          onClick={network.retry}
          disabled={network.checking}
          className="rounded border border-amber-700 px-2 py-1 text-[11px] font-medium disabled:cursor-not-allowed disabled:opacity-50"
        >
          {network.checking ? "Checking…" : "Retry connection"}
        </button>
      </div>
    </div>
  );
}
