import { ErrorBoundary } from "@/components/ErrorBoundary";
import Mermaid from "@/components/Mermaid";

import { strategyPhases } from "./strategyPhases";
import { StrategyPhaseTable } from "./StrategyPhaseTable";
import { StrategyRiskExplanations } from "./StrategyRiskExplanations";

export function StrategyPhaseDiagram() {
  const phaseSummary = strategyPhases
    .map(
      (phase, index) =>
        `Phase ${index + 1} of ${strategyPhases.length}: ${phase.phase}, market ${phase.marketCondition}, action ${phase.action}. ${phase.description}`,
    )
    .join(" ");

  return (
    <section aria-labelledby="strategy-phases-heading">
      <h2 id="strategy-phases-heading">Strategy Phases</h2>

      <figure>
        <ErrorBoundary
          variant="component"
          context={{ component: "StrategyPhaseDiagram" }}
          title="The strategy diagram could not be displayed"
          description="The table and phase summary below contain the same information."
        >
          <Mermaid />
        </ErrorBoundary>
      <figure className="min-w-0">
        <div
          className="responsive-scroll-region"
          role="region"
          aria-label="Strategy phase diagram — scroll horizontally for the full diagram"
          tabIndex={0}
        >
          <Mermaid />
        </div>

        <figcaption>
          Visual representation of the market strategy lifecycle. Time range:
          one repeating market cycle from Accumulation through Markup,
          Distribution and Markdown; phases have no fixed duration.
          <span className="sr-only" data-testid="strategy-phase-state-summary">
            {phaseSummary}
          </span>
        </figcaption>
      </figure>

      <div className="mt-8">
        <h3>Accessible Alternative</h3>

        <p className="mb-4">
          The following table presents the same information contained in the
          strategy diagram.
        </p>

        <StrategyPhaseTable />
        <StrategyRiskExplanations />
      </div>
    </section>
  );
}
