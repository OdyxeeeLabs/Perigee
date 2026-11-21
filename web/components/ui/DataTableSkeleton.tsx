import React from "react";

export interface DataTableSkeletonProps {
  /** Number of placeholder columns. Should match the real `columns.length`. */
  columns?: number;
  /** Number of placeholder rows. Defaults to `pageSize` used by `DataTable` elsewhere (10). */
  rows?: number;
  /** Whether to render the title/subtitle header bar placeholder. */
  showHeader?: boolean;
  /** Whether to render the filter input placeholder. */
  showFilter?: boolean;
}

/**
 * Skeleton for `components/ui/DataTable.tsx` — issue #685.
 *
 * Mirrors `DataTable`'s structure (optional title/subtitle bar, optional
 * filter input, header row, body rows) at the same column count so swapping
 * it for the real table causes no layout shift.
 */
export const DataTableSkeleton: React.FC<DataTableSkeletonProps> = ({
  columns = 4,
  rows = 6,
  showHeader = true,
  showFilter = true,
}) => {
  return (
    <div
      className="overflow-hidden rounded-lg border border-slate-800 animate-pulse"
      role="table"
      aria-label="Table loading"
      aria-busy="true"
    >
      {showHeader && (
        <div className="flex items-center justify-between border-b border-slate-800 bg-slate-900 px-4 py-3">
          <div className="flex flex-col gap-1.5">
            <div className="h-4 w-40 rounded bg-slate-700" />
            <div className="h-2.5 w-24 rounded bg-slate-800" />
          </div>
          {showFilter && <div className="h-8 w-48 rounded bg-slate-800" />}
        </div>
      )}

      <div className="grid gap-2 bg-slate-900 px-4 py-2" style={{ gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))` }} role="row">
        {Array.from({ length: columns }).map((_, i) => (
          <div key={i} className="h-2.5 rounded bg-slate-700" role="columnheader" />
        ))}
      </div>

      <div className="divide-y divide-slate-800">
        {Array.from({ length: rows }).map((_, rowIndex) => (
          <div
            key={rowIndex}
            className="grid gap-2 px-4 py-3"
            style={{ gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))` }}
            role="row"
          >
            {Array.from({ length: columns }).map((_, colIndex) => (
              <div key={colIndex} className="h-3 rounded bg-slate-800" role="cell" />
            ))}
          </div>
        ))}
      </div>
    </div>
  );
};
