import { strategyPhases, type StrategyRiskLevel } from "./strategyPhases";

const levelLabels: Record<StrategyRiskLevel, string> = {
  low: "Lower risk",
  medium: "Moderate risk",
  high: "Higher risk",
};

/**
 * Plain-language risk explanations shown next to the published cycle phases
 * (WEB-18 / #818).
 *
 * The explanations are descriptive, not calculated: they explain the risk of
 * each published rule and the bound the rule already applies. The textual level
 * label is always rendered, so colour or an icon is never the only signal.
 */
export function StrategyRiskExplanations() {
  return (
    <section
      aria-labelledby="strategy-risk-heading"
      className="mt-6 rounded-xl border border-slate-800 bg-slate-900/50 p-5"
      data-testid="strategy-risk-explanations"
    >
      <h3 id="strategy-risk-heading" className="mb-4 text-lg font-semibold text-cyan-300">
        Why this phase carries risk
      </h3>
      <ol className="space-y-4">
        {strategyPhases.map((phase) => (
          <li key={phase.id} className="border-l-2 border-slate-700 pl-4">
            <p className="text-sm font-medium text-slate-200">
              {phase.phase} — <span>{levelLabels[phase.risk.level]}</span>
            </p>
            <p className="mt-1 text-sm text-slate-400">{phase.risk.summary}</p>
            <p className="mt-1 text-xs text-slate-500">
              Bound: {phase.risk.mitigation}
            </p>
          </li>
        ))}
      </ol>
    </section>
  );
}

export default StrategyRiskExplanations;
