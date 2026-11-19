import Mermaid from "@/components/Mermaid";

import { StrategyPhaseTable } from "./StrategyPhaseTable";
import { StrategyRiskExplanations } from "./StrategyRiskExplanations";

export function StrategyPhaseDiagram() {
  return (
    <section
      aria-labelledby="strategy-phases-heading"
    >
      <h2 id="strategy-phases-heading">
        Strategy Phases
      </h2>

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
          Visual representation of the
          market strategy lifecycle.
        </figcaption>
      </figure>

      <div className="mt-8">
        <h3>Accessible Alternative</h3>

        <p className="mb-4">
          The following table presents the
          same information contained in the
          strategy diagram.
        </p>

        <StrategyPhaseTable />
        <StrategyRiskExplanations />
      </div>
    </section>
  );
}