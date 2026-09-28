'use client';

/**
 * DataTable — reusable table component with sorting, client-side filtering,
 * and pagination. (WEB-33)
 *
 * Usage:
 *   const columns: ColumnDef<MyRow>[] = [
 *     { key: 'name', header: 'Name', sortable: true },
 *     { key: 'value', header: 'Value', sortable: true, align: 'right' },
 *   ];
 *   <DataTable columns={columns} data={rows} pageSize={10} />
 */

import React, { useMemo, useState, useId } from 'react';
import { cn } from '../../lib/utils';

// ─────────────────────────────────────────────────────────────────────────────
// Public types
// ─────────────────────────────────────────────────────────────────────────────

export type SortDirection = 'asc' | 'desc';

/** Column definition — T is the row type. */
export interface ColumnDef<T> {
  /** Unique key matching a field in T (or a synthetic label for accessor cols). */
  key: keyof T | string;
  /** Header label shown in <th>. */
  header: React.ReactNode;
  /** Whether this column is sortable. Defaults to false. */
  sortable?: boolean;
  /**
   * Optional cell renderer. Receives the row and returns a React node.
   * Falls back to `String(row[key])` when omitted.
   */
  cell?: (row: T) => React.ReactNode;
  /** Text alignment for both <th> and <td>. Defaults to 'left'. */
  align?: 'left' | 'center' | 'right';
  /** Additional className applied to every <td> in this column. */
  className?: string;
}

export interface DataTableProps<T> {
  /** Column definitions. */
  columns: ColumnDef<T>[];
  /** Row data. */
  data: T[];
  /**
   * Optional row key extractor. Defaults to the row index.
   * Providing a stable key (e.g. a unique `id` field) avoids reconciliation
   * issues when sorting / filtering.
   */
  rowKey?: (row: T, index: number) => string | number;
  /** Number of rows per page. Pass 0 or undefined to disable pagination. */
  pageSize?: number;
  /**
   * Keys of columns searched by the filter input.
   * When omitted, all string/number cell values are included in the search.
   */
  filterKeys?: (keyof T | string)[];
  /** Placeholder text for the filter input. */
  filterPlaceholder?: string;
  /** Message shown when the filtered result set is empty. */
  emptyMessage?: React.ReactNode;
  /** Optional title rendered in the table header bar. */
  title?: React.ReactNode;
  /** Optional subtitle rendered below the title. */
  subtitle?: React.ReactNode;
  /** Additional className for the outermost container. */
  className?: string;
  /** data-testid applied to the root element (WEB-34 compatibility). */
  'data-testid'?: string;
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal helpers
// ─────────────────────────────────────────────────────────────────────────────

function alignClass(align: ColumnDef<unknown>['align']) {
  if (align === 'center') return 'text-center';
  if (align === 'right') return 'text-right';
  return 'text-left';
}

function cellValue<T>(row: T, col: ColumnDef<T>): React.ReactNode {
  if (col.cell) return col.cell(row);
  const raw = (row as Record<string, unknown>)[col.key as string];
  if (raw === null || raw === undefined) return '—';
  return String(raw);
}

/** Returns a plain string suitable for filter comparison. */
function cellText<T>(row: T, col: ColumnDef<T>): string {
  if (col.cell) {
    // For custom cells fall back to the raw value for filtering.
    const raw = (row as Record<string, unknown>)[col.key as string];
    return raw === null || raw === undefined ? '' : String(raw);
  }
  const raw = (row as Record<string, unknown>)[col.key as string];
  return raw === null || raw === undefined ? '' : String(raw);
}

function sortRows<T>(
  rows: T[],
  columns: ColumnDef<T>[],
  sortKey: string,
  direction: SortDirection,
): T[] {
  const col = columns.find((c) => c.key === sortKey);
  return [...rows].sort((a, b) => {
    const aVal = (a as Record<string, unknown>)[sortKey];
    const bVal = (b as Record<string, unknown>)[sortKey];

    // Null / undefined always goes last.
    if (aVal == null && bVal == null) return 0;
    if (aVal == null) return 1;
    if (bVal == null) return -1;

    let cmp = 0;
    if (typeof aVal === 'number' && typeof bVal === 'number') {
      cmp = aVal - bVal;
    } else {
      cmp = String(aVal).localeCompare(String(bVal));
    }

    return direction === 'asc' ? cmp : -cmp;
  });
}

// ─────────────────────────────────────────────────────────────────────────────
// Sub-components
// ─────────────────────────────────────────────────────────────────────────────

function SortIcon({
  active,
  direction,
}: {
  active: boolean;
  direction: SortDirection;
}) {
  if (!active) {
    return (
      <span className="ml-1 inline-block select-none text-[10px] text-[#6e7681]" aria-hidden>
        ↕
      </span>
    );
  }
  return (
    <span className="ml-1 inline-block select-none text-[10px] text-[#00d9ff]" aria-hidden>
      {direction === 'asc' ? '↑' : '↓'}
    </span>
  );
}

function PaginationBar({
  page,
  pageCount,
  pageSize,
  total,
  onPageChange,
}: {
  page: number;
  pageCount: number;
  pageSize: number;
  total: number;
  onPageChange: (p: number) => void;
}) {
  const from = total === 0 ? 0 : (page - 1) * pageSize + 1;
  const to = Math.min(page * pageSize, total);

  return (
    <div className="flex items-center justify-between border-t border-[#30363d] px-4 py-2 text-xs text-[#8b949e]">
      <span>
        {total === 0 ? 'No results' : `${from}–${to} of ${total}`}
      </span>

      <div className="flex items-center gap-1">
        <button
          type="button"
          onClick={() => onPageChange(1)}
          disabled={page === 1}
          aria-label="First page"
          className="rounded px-1.5 py-0.5 hover:bg-[#21262d] disabled:cursor-not-allowed disabled:opacity-40"
        >
          «
        </button>
        <button
          type="button"
          onClick={() => onPageChange(page - 1)}
          disabled={page === 1}
          aria-label="Previous page"
          className="rounded px-1.5 py-0.5 hover:bg-[#21262d] disabled:cursor-not-allowed disabled:opacity-40"
        >
          ‹
        </button>

        <span className="px-2 tabular-nums">
          {page} / {pageCount || 1}
        </span>

        <button
          type="button"
          onClick={() => onPageChange(page + 1)}
          disabled={page >= pageCount}
          aria-label="Next page"
          className="rounded px-1.5 py-0.5 hover:bg-[#21262d] disabled:cursor-not-allowed disabled:opacity-40"
        >
          ›
        </button>
        <button
          type="button"
          onClick={() => onPageChange(pageCount)}
          disabled={page >= pageCount}
          aria-label="Last page"
          className="rounded px-1.5 py-0.5 hover:bg-[#21262d] disabled:cursor-not-allowed disabled:opacity-40"
        >
          »
        </button>
      </div>
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// Main component
// ─────────────────────────────────────────────────────────────────────────────

export function DataTable<T>({
  columns,
  data,
  rowKey,
  pageSize = 0,
  filterKeys,
  filterPlaceholder = 'Filter…',
  emptyMessage = 'No results found.',
  title,
  subtitle,
  className,
  'data-testid': dataTestId,
}: DataTableProps<T>) {
  const filterId = useId();

  // ── Sort state ─────────────────────────────────────────────────────────────
  const firstSortable = columns.find((c) => c.sortable);
  const [sortKey, setSortKey] = useState<string>(
    firstSortable ? String(firstSortable.key) : '',
  );
  const [sortDir, setSortDir] = useState<SortDirection>('asc');

  // ── Filter state ───────────────────────────────────────────────────────────
  const [filter, setFilter] = useState('');

  // ── Pagination state ───────────────────────────────────────────────────────
  const [page, setPage] = useState(1);

  // ── Derived data ───────────────────────────────────────────────────────────
  const filtered = useMemo(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return data;

    const searchCols = filterKeys
      ? columns.filter((c) => (filterKeys as string[]).includes(c.key as string))
      : columns;

    return data.filter((row) =>
      searchCols.some((col) => cellText(row, col).toLowerCase().includes(q)),
    );
  }, [data, filter, filterKeys, columns]);

  const sorted = useMemo(() => {
    if (!sortKey) return filtered;
    return sortRows(filtered, columns, sortKey, sortDir);
  }, [filtered, columns, sortKey, sortDir]);

  const paginationEnabled = pageSize > 0;
  const pageCount = paginationEnabled ? Math.ceil(sorted.length / pageSize) : 1;

  // Reset to page 1 whenever filter changes.
  const handleFilterChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    setFilter(e.target.value);
    setPage(1);
  };

  const paginated = useMemo(() => {
    if (!paginationEnabled) return sorted;
    const start = (page - 1) * pageSize;
    return sorted.slice(start, start + pageSize);
  }, [sorted, paginationEnabled, page, pageSize]);

  // ── Sort toggle ────────────────────────────────────────────────────────────
  const handleSort = (key: string) => {
    if (key === sortKey) {
      setSortDir((d) => (d === 'asc' ? 'desc' : 'asc'));
    } else {
      setSortKey(key);
      setSortDir('asc');
    }
    setPage(1);
  };

  // ─────────────────────────────────────────────────────────────────────────
  // Render
  // ─────────────────────────────────────────────────────────────────────────
  return (
    <div
      className={cn('rounded-lg border border-[#30363d] bg-[#0d1117]', className)}
      data-testid={dataTestId}
    >
      {/* ── Header bar ─────────────────────────────────────────────────────── */}
      <div className="flex flex-col gap-3 border-b border-[#30363d] px-4 py-3 sm:flex-row sm:items-center sm:justify-between">
        <div className="min-w-0">
          {title && (
            <h3 className="truncate text-sm font-semibold text-[#c9d1d9]">
              {title}
            </h3>
          )}
          {subtitle && (
            <p className="mt-0.5 text-xs text-[#8b949e]">{subtitle}</p>
          )}
        </div>

        <label htmlFor={filterId} className="sr-only">
          {filterPlaceholder}
        </label>
        <input
          id={filterId}
          type="search"
          value={filter}
          onChange={handleFilterChange}
          placeholder={filterPlaceholder}
          aria-label={filterPlaceholder}
          data-testid={dataTestId ? `${dataTestId}-filter` : undefined}
          className={cn(
            'w-full rounded-md border border-[#30363d] bg-[#161b22]',
            'px-3 py-1.5 text-xs text-[#c9d1d9] placeholder:text-[#8b949e]',
            'focus:border-[#00d9ff] focus:outline-none focus:ring-1 focus:ring-[#00d9ff]',
            'sm:w-56',
          )}
        />
      </div>

      {/* ── Table ──────────────────────────────────────────────────────────── */}
      <div className="overflow-x-auto">
        <table
          className="min-w-full text-left text-sm"
          data-testid={dataTestId ? `${dataTestId}-table` : undefined}
        >
          <thead className="bg-[#161b22] text-xs text-[#8b949e]">
            <tr>
              {columns.map((col) => {
                const key = String(col.key);
                const isSorted = sortKey === key;
                return (
                  <th
                    key={key}
                    scope="col"
                    className={cn(
                      'px-4 py-3 font-medium',
                      alignClass(col.align),
                      col.sortable && 'cursor-pointer select-none',
                    )}
                    aria-sort={
                      isSorted
                        ? sortDir === 'asc'
                          ? 'ascending'
                          : 'descending'
                        : undefined
                    }
                  >
                    {col.sortable ? (
                      <button
                        type="button"
                        onClick={() => handleSort(key)}
                        className="inline-flex items-center hover:text-[#c9d1d9]"
                        aria-label={`Sort by ${typeof col.header === 'string' ? col.header : key}`}
                      >
                        {col.header}
                        <SortIcon active={isSorted} direction={sortDir} />
                      </button>
                    ) : (
                      col.header
                    )}
                  </th>
                );
              })}
            </tr>
          </thead>

          <tbody className="divide-y divide-[#30363d]">
            {paginated.length === 0 ? (
              <tr>
                <td
                  colSpan={columns.length}
                  className="px-4 py-8 text-center text-[#8b949e]"
                >
                  {emptyMessage}
                </td>
              </tr>
            ) : (
              paginated.map((row, idx) => {
                const key = rowKey
                  ? rowKey(row, (page - 1) * (pageSize || 0) + idx)
                  : (page - 1) * (pageSize || 1) + idx;
                return (
                  <tr
                    key={key}
                    className="hover:bg-[#0f1621] transition-colors"
                    data-testid={dataTestId ? `${dataTestId}-row` : undefined}
                  >
                    {columns.map((col) => (
                      <td
                        key={String(col.key)}
                        className={cn(
                          'px-4 py-3 text-[#c9d1d9]',
                          alignClass(col.align),
                          col.className,
                        )}
                      >
                        {cellValue(row, col)}
                      </td>
                    ))}
                  </tr>
                );
              })
            )}
          </tbody>
        </table>
      </div>

      {/* ── Pagination ─────────────────────────────────────────────────────── */}
      {paginationEnabled && (
        <PaginationBar
          page={page}
          pageCount={pageCount}
          pageSize={pageSize}
          total={sorted.length}
          onPageChange={setPage}
        />
      )}
    </div>
  );
}
