import React from 'react';
import { Skeleton } from './Skeleton';

/**
 * NAVSkeleton
 *
 * Shown while NAV (Net Asset Value), high-water mark, and performance fee
 * accrual data are being fetched from the backend. Mirrors the real NAV /
 * performance metrics card layout.
 */
export const NAVSkeleton: React.FC = () => (
  <div
    aria-label="Loading NAV and performance metrics…"
    className="rounded-xl border border-[#30363d] bg-[#161b22] p-6"
  >
    {/* Card header */}
    <div className="mb-5 flex items-center justify-between">
      <Skeleton className="h-5 w-48" />
      {/* Period selector */}
      <Skeleton className="h-7 w-24 rounded-md" />
    </div>

    {/* NAV – primary big number */}
    <div className="mb-6">
      <Skeleton className="mb-2 h-3 w-32" />
      <div className="flex items-end gap-3">
        <Skeleton className="h-10 w-40" />
        {/* Change badge */}
        <Skeleton className="mb-1 h-5 w-16 rounded-full" />
      </div>
    </div>

    {/* Metrics grid: High-Water Mark, Accrued Fee, Return */}
    <div className="mb-6 grid grid-cols-3 gap-4">
      {[0, 1, 2].map((i) => (
        <div key={i} className="rounded-lg border border-[#30363d] bg-[#0d1117] p-3">
          <Skeleton className="mb-2 h-3 w-full" />
          <Skeleton className="h-5 w-3/4" />
        </div>
      ))}
    </div>

    {/* Sparkline / mini chart placeholder */}
    <div className="overflow-hidden rounded-lg border border-[#30363d] bg-[#0d1117] p-3">
      <div className="mb-2 flex justify-between">
        <Skeleton className="h-3 w-20" />
        <Skeleton className="h-3 w-12" />
      </div>
      {/* Simulated bar chart */}
      <div className="flex h-16 items-end gap-1">
        {[55, 70, 45, 80, 60, 90, 65, 75, 50, 85, 70, 95].map((h, i) => (
          <Skeleton
            key={i}
            className="flex-1 rounded-sm"
            style={{ height: `${h}%` }}
          />
        ))}
      </div>
    </div>

    {/* Footer: fee accrual note */}
    <div className="mt-4 flex items-center gap-2">
      <Skeleton className="h-3 w-3 rounded-full" />
      <Skeleton className="h-3 w-56" />
    </div>
  </div>
);
