import React from "react";
import { TRANSACTION_ROW_HEIGHT } from "@/constant/vault.constants";

/**
 * Skeleton for a transaction history row — issue #685.
 *
 * No `TransactionRow` component exists in the codebase yet (only
 * `hooks/useVirtualTransactionList.ts` and the `TRANSACTION_ROW_HEIGHT`
 * constant it shares with it, from #687's virtualised-list groundwork), so
 * this placeholder is sized to that same row height and laid out for the
 * fields a transaction row will need (hash, type, amount, timestamp,
 * status) — ready to drop in once that row ships, instead of a generic
 * spinner being added now and swapped out later.
 */
export const TransactionRowSkeleton: React.FC = () => {
  return (
    <div
      style={{ height: TRANSACTION_ROW_HEIGHT }}
      className="flex items-center justify-between gap-4 border-b border-slate-800 px-4 animate-pulse"
      role="row"
      aria-label="Transaction loading"
      aria-busy="true"
    >
      <div className="flex min-w-0 flex-col gap-1.5" role="cell">
        <div className="h-3 w-40 rounded bg-slate-700" />
        <div className="h-2.5 w-24 rounded bg-slate-800" />
      </div>
      <div className="h-3 w-20 shrink-0 rounded bg-slate-800" role="cell" />
      <div className="h-3 w-16 shrink-0 rounded bg-slate-700" role="cell" />
      <div className="h-4 w-16 shrink-0 rounded-full bg-slate-800" role="cell" />
    </div>
  );
};

/** Stacked `TransactionRowSkeleton`s for an initial transaction history load. */
export const TransactionListSkeleton: React.FC<{ count?: number }> = ({ count = 8 }) => (
  <div
    className="overflow-hidden rounded-xl border border-slate-800"
    role="list"
    aria-label="Transaction history loading"
    aria-busy="true"
  >
    {Array.from({ length: count }).map((_, i) => (
      <TransactionRowSkeleton key={i} />
    ))}
  </div>
);
