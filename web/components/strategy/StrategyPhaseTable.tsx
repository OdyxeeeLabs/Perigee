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

export function StrategyPhaseTable() {
  return (
    <DataTable<StrategyPhase>
      columns={columns}
      data={strategyPhases}
      rowKey={(row) => row.id}
      title="Strategy Phases"
      subtitle="Cycle-phase rotation rules."
      filterPlaceholder="Filter phases…"
      emptyMessage="No matching phases."
      data-testid="strategy-phase-table"
    />
  );
}
