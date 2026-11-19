import React from "react";

/**
 * Skeleton for `VaultDetailModal` / `VaultDetailContent` — issue #685.
 *
 * Built alongside #684 so the vault detail modal ships with a matching
 * skeleton from the start, instead of a spinner that would need to be
 * replaced later. Mirrors the section layout of `VaultDetailContent`
 * (token info, performance metrics, chart, strategy, risk, actions) so the
 * loading state occupies the same space as the loaded content — no layout
 * shift when the fetch resolves.
 */
export const VaultDetailSkeleton: React.FC = () => {
  return (
    <div className="flex flex-col gap-6 animate-pulse" role="group" aria-label="Vault details loading" aria-busy="true">
      {/* Token info */}
      <section>
        <div className="h-3 w-20 bg-slate-800 rounded" />
        <div className="mt-2 grid grid-cols-2 gap-x-4 gap-y-2">
          {Array.from({ length: 5 }).map((_, i) => (
            <React.Fragment key={i}>
              <div className="h-3 w-24 bg-slate-800 rounded" />
              <div className="h-3 w-28 bg-slate-800 rounded justify-self-end" />
            </React.Fragment>
          ))}
        </div>
      </section>

      {/* Performance metrics */}
      <section>
        <div className="h-3 w-24 bg-slate-800 rounded" />
        <div className="mt-2 grid grid-cols-2 gap-3 sm:grid-cols-4">
          {Array.from({ length: 4 }).map((_, i) => (
            <div key={i} className="rounded-lg border border-slate-800 bg-slate-900/60 px-3 py-2">
              <div className="h-2 w-12 bg-slate-800 rounded" />
              <div className="mt-2 h-4 w-16 bg-slate-700 rounded" />
            </div>
          ))}
        </div>
      </section>

      {/* Chart */}
      <section>
        <div className="h-3 w-28 bg-slate-800 rounded" />
        <div className="mt-2 h-[100px] w-full rounded-lg border border-slate-800 bg-slate-900/60" />
      </section>

      {/* Strategy */}
      <section>
        <div className="h-3 w-36 bg-slate-800 rounded" />
        <div className="mt-2 rounded-lg border border-slate-800 bg-slate-900/60 p-3">
          <div className="h-4 w-40 bg-slate-700 rounded" />
          <div className="mt-2 h-1.5 w-full rounded-full bg-slate-800" />
          <div className="mt-3 grid grid-cols-2 gap-2">
            <div className="h-3 w-full bg-slate-800 rounded" />
            <div className="h-3 w-full bg-slate-800 rounded" />
          </div>
        </div>
      </section>

      {/* Risk parameters */}
      <section>
        <div className="h-3 w-28 bg-slate-800 rounded" />
        <div className="mt-2 flex gap-3">
          <div className="h-4 w-32 bg-slate-800 rounded" />
          <div className="h-4 w-40 bg-slate-800 rounded" />
          <div className="h-4 w-16 bg-slate-800 rounded-full" />
        </div>
      </section>

      {/* Actions */}
      <section className="flex gap-2 border-t border-slate-800 pt-4">
        <div className="h-9 w-24 bg-slate-800 rounded" />
        <div className="h-9 w-24 bg-slate-800 rounded" />
        <div className="h-9 w-20 bg-slate-800 rounded" />
      </section>
    </div>
  );
};
