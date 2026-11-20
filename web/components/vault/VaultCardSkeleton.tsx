import React from "react";

/**
 * Skeleton matching `VaultRow`'s dimensions and layout — issue #685.
 *
 * Mirrors `VaultRow` exactly: same outer padding/border, same two-column
 * (name/owner vs. balance/apy/status) layout, so a list of these placeholders
 * occupies the same height as the real rows once they arrive (no layout
 * shift).
 */
export const VaultCardSkeleton: React.FC = () => {
  return (
    <div
      className="flex flex-col items-stretch gap-3 rounded-lg border border-slate-800 bg-slate-900/60 px-4 py-3 animate-pulse sm:flex-row sm:items-center sm:justify-between"
      role="row"
      aria-label="Vault loading"
      aria-busy="true"
    >
      <div className="flex flex-col gap-1.5" role="cell">
        <div className="h-3.5 w-32 rounded bg-slate-700" />
        <div className="h-3 w-20 rounded bg-slate-800" />
      </div>
      <div className="grid grid-cols-3 gap-2 sm:flex sm:items-center sm:gap-4" role="cell">
        <div className="h-3 w-16 rounded bg-slate-700" />
        <div className="h-3 w-10 rounded bg-slate-700" />
        <div className="h-4 w-14 rounded-full bg-slate-800" />
      </div>
    </div>
  );
};

/** Renders `count` stacked `VaultCardSkeleton` rows for an initial list load. */
export const VaultListSkeleton: React.FC<{ count?: number }> = ({ count = 5 }) => (
  <div
    className="overflow-hidden rounded-xl border border-slate-800"
    role="list"
    aria-label="Vault list loading"
    aria-busy="true"
  >
    <div className="flex flex-col gap-2 p-2">
      {Array.from({ length: count }).map((_, i) => (
        <VaultCardSkeleton key={i} />
      ))}
    </div>
  </div>
);
