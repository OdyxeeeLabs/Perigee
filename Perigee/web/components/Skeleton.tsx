import React from 'react';

interface SkeletonProps {
  /** Extra Tailwind classes (width, height, rounded, etc.) */
  className?: string;
  /** Inline style overrides */
  style?: React.CSSProperties;
}

/**
 * Skeleton – a single shimmer block.
 *
 * All domain skeletons (VaultBalance, StrategyPhase, NAV…) are composed from
 * this primitive so the shimmer animation is defined in exactly one place.
 *
 * Usage:
 *   <Skeleton className="h-4 w-32 rounded" />
 */
export const Skeleton: React.FC<SkeletonProps> = ({ className = '', style }) => (
  <div
    aria-hidden="true"
    className={`relative overflow-hidden bg-[#1f2937] rounded ${className}`}
    style={style}
  >
    {/* Shimmer sweep */}
    <span
      className="absolute inset-0 -translate-x-full animate-[shimmer_1.6s_infinite]"
      style={{
        background:
          'linear-gradient(90deg, transparent 0%, rgba(255,255,255,0.06) 50%, transparent 100%)',
      }}
    />
  </div>
);
