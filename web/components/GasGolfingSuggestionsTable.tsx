'use client';

import React from 'react';
import clsx from 'clsx';
import { DataTable, type ColumnDef } from './ui/DataTable';
import type {
  GasGolfingSuggestion,
} from '../lib/gasGolfingSort';

// ─────────────────────────────────────────────────────────────────────────────
// Severity chip — kept as a local helper since it's UI-specific rendering
// ─────────────────────────────────────────────────────────────────────────────

function SeverityChip({ severity }: { severity: string }) {
  const normalized = severity.toLowerCase();
  const style =
    normalized === 'high'
      ? 'border-red-500/50 bg-red-500/10 text-red-200'
      : normalized === 'medium'
        ? 'border-yellow-500/50 bg-yellow-500/10 text-yellow-200'
        : normalized === 'low'
          ? 'border-emerald-500/50 bg-emerald-500/10 text-emerald-200'
          : 'border-slate-500/50 bg-slate-500/10 text-slate-200';

  return (
    <span
      className={clsx(
        'inline-flex items-center rounded-full border px-2 py-0.5 text-[11px] font-semibold',
        style,
      )}
    >
      {severity.toUpperCase()}
    </span>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// Column definitions
// ─────────────────────────────────────────────────────────────────────────────

const columns: ColumnDef<GasGolfingSuggestion>[] = [
  {
    key: 'title',
    header: 'Suggestion',
    cell: (row) => (
      <div>
        <div className="font-medium text-[#c9d1d9]">{row.title}</div>
        {row.description ? (
          <div className="mt-0.5 text-xs text-[#8b949e]">{row.description}</div>
        ) : null}
      </div>
    ),
  },
  {
    key: 'severity',
    header: 'Severity',
    sortable: true,
    cell: (row) => <SeverityChip severity={String(row.severity)} />,
  },
  {
    key: 'gas_saved_estimate',
    header: 'Gas Saved',
    sortable: true,
    align: 'right',
    className: 'font-mono text-xs',
    cell: (row) => (
      <span>{row.gas_saved_estimate ?? '—'}</span>
    ),
  },
];

// ─────────────────────────────────────────────────────────────────────────────
// Component
// ─────────────────────────────────────────────────────────────────────────────

export function GasGolfingSuggestionsTable({
  suggestions,
}: {
  suggestions: GasGolfingSuggestion[];
}) {
  if (!suggestions.length) {
    return (
      <div className="rounded-lg border border-[#30363d] bg-[#0d1117] p-4 text-sm text-[#8b949e]">
        No gas golfing suggestions found.
      </div>
    );
  }

  return (
    <DataTable<GasGolfingSuggestion>
      columns={columns}
      data={suggestions}
      rowKey={(row, idx) => `${row.title}-${idx}`}
      title="Gas Golfing Suggestions"
      subtitle="Click a column header to sort."
      filterPlaceholder="Filter suggestions…"
      emptyMessage="No matching suggestions."
      data-testid="gas-golfing-suggestions-table"
    />
  );
}
