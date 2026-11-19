import { memo } from 'react';
import { DataTable, type ColumnDef } from '../ui/DataTable';
import { strategyPhases, type StrategyPhase } from './strategyPhases';

const columns: ColumnDef<StrategyPhase>[] = [
  {
    key: 'phase',
    header: 'Phase',
    sortable: true,
  },
  {
    key: 'marketCondition',
    header: 'Market',
    sortable: true,
  },
  {
    key: 'action',
    header: 'Recommended Action',
    sortable: true,
  },
  {
    key: 'description',
    header: 'Description',
  },
];

function StrategyPhaseTableImpl() {
  return (
    <DataTable<StrategyPhase>
      columns={columns}
      data={strategyPhases}
      rowKey={(row) => row.id}
      title="Strategy Phases"
      caption="Strategy phases: phase, market condition, recommended action and description. Same information as the strategy diagram."
      subtitle="Cycle-phase rotation rules."
      filterPlaceholder="Filter phases…"
      emptyMessage="No matching phases."
      data-testid="strategy-phase-table"
    />
  );
}

/** Memoised (#692): takes no props, so it never needs to re-render with its parent. */
export const StrategyPhaseTable = memo(StrategyPhaseTableImpl);
