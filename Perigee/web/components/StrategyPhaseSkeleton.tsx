import React from 'react';
import { Skeleton } from './Skeleton';

/**
 * StrategyPhaseSkeleton
 *
 * Shown while the current strategy phase (Bull / Bear) and rotation trigger
 * status are being fetched from the backend. Mirrors the real strategy phase
 * indicator card's layout.
 */
export const StrategyPhaseSkeleton: React.FC = () => (
  <div
    aria-label="Loading strategy phase…"
    className="rounded-xl border border-[#30363d] bg-[#161b22] p-6"
  >
    {/* Card header */}
    <div className="mb-5 flex items-center justify-between">
      <Skeleton className="h-5 w-32" />
      {/* Agent status badge */}
      <Skeleton className="h-5 w-16 rounded-full" />
    </div>

    {/* Phase pill – large central indicator */}
    <div className="mb-6 flex items-center gap-4">
      <Skeleton className="h-12 w-12 rounded-full" />
      <div>
        <Skeleton className="mb-2 h-7 w-28" />
        <Skeleton className="h-3 w-44" />
      </div>
    </div>

    {/* Trigger conditions list */}
    <div className="space-y-3">
      {[0, 1, 2].map((i) => (
        <div key={i} className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Skeleton className="h-4 w-4 rounded-full" />
            <Skeleton className="h-3 w-32" />
          </div>
          <Skeleton className="h-4 w-16 rounded-full" />
        </div>
      ))}
    </div>

    {/* Rotation progress bar */}
    <div className="mt-5">
      <div className="mb-1.5 flex justify-between">
        <Skeleton className="h-3 w-24" />
        <Skeleton className="h-3 w-10" />
      </div>
      <div className="h-2 w-full overflow-hidden rounded-full bg-[#0d1117]">
        <Skeleton className="h-full w-2/5 rounded-full" />
      </div>
    </div>
  </div>
);
