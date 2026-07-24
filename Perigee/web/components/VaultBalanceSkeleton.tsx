import React from 'react';
import { Skeleton } from './Skeleton';

/**
 * VaultBalanceSkeleton
 *
 * Shown while vault balance data (BTC/ETH basket values and total vault value)
 * is being fetched from the backend. Mirrors the layout of the real vault
 * balance card so the page doesn't jump once data arrives.
 */
export const VaultBalanceSkeleton: React.FC = () => (
  <div
    aria-label="Loading vault balances…"
    className="rounded-xl border border-[#30363d] bg-[#161b22] p-6"
  >
    {/* Card header */}
    <div className="mb-5 flex items-center justify-between">
      <Skeleton className="h-5 w-36" />
      {/* Status pill */}
      <Skeleton className="h-6 w-20 rounded-full" />
    </div>

    {/* Total vault value – big number */}
    <div className="mb-6">
      <Skeleton className="mb-2 h-3 w-24" />
      <Skeleton className="h-9 w-48" />
    </div>

    {/* Asset rows: BTC and ETH */}
    {[0, 1].map((i) => (
      <div
        key={i}
        className="flex items-center justify-between py-3 [&:not(:last-child)]:border-b [&:not(:last-child)]:border-[#30363d]"
      >
        <div className="flex items-center gap-3">
          {/* Asset icon */}
          <Skeleton className="h-8 w-8 rounded-full" />
          <div>
            <Skeleton className="mb-1.5 h-4 w-10" />
            <Skeleton className="h-3 w-20" />
          </div>
        </div>
        <div className="text-right">
          <Skeleton className="mb-1.5 ml-auto h-4 w-24" />
          <Skeleton className="ml-auto h-3 w-16" />
        </div>
      </div>
    ))}

    {/* Footer: last updated */}
    <div className="mt-4 flex items-center gap-1.5">
      <Skeleton className="h-3 w-3 rounded-full" />
      <Skeleton className="h-3 w-28" />
    </div>
  </div>
);
