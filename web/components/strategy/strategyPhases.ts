export type StrategyRiskLevel = "low" | "medium" | "high";

export interface StrategyPhaseRisk {
  level: StrategyRiskLevel;
  /** What can go wrong during this published phase. */
  summary: string;
  /** The rule-based control that limits the exposure. */
  mitigation: string;
}

export interface StrategyPhase {
  id: string;
  phase: string;
  marketCondition: string;
  action: string;
  description: string;
  risk: StrategyPhaseRisk;
}

export const strategyPhases: StrategyPhase[] = [
  {
    id: "accumulation",
    phase: "Accumulation",
    marketCondition: "Bullish Reversal",
    action: "Increase exposure",
    description:
      "Gradually accumulate assets during early bullish confirmation.",
    risk: {
      level: "medium",
      summary: "Early confirmation can fail, so a reversal may continue downwards.",
      mitigation:
        "Exposure is added gradually and bounded by the published maximum basket weight.",
    },
  },
  {
    id: "markup",
    phase: "Markup",
    marketCondition: "Bull Market",
    action: "Hold / Scale In",
    description:
      "Ride the upward trend while managing risk.",
    risk: {
      level: "medium",
      summary: "A prolonged rally can end abruptly, reducing unrealised gains.",
      mitigation:
        "Holding is rules-based; drawdown thresholds trigger rotation rather than discretionary selling.",
    },
  },
  {
    id: "distribution",
    phase: "Distribution",
    marketCondition: "Bearish Reversal",
    action: "Reduce exposure",
    description:
      "Take profits as momentum weakens.",
    risk: {
      level: "high",
      summary: "Reducing too early misses upside, while reducing too late exposes gains to a full drawdown.",
      mitigation:
        "A drift deadband prevents continuous rebalancing and keeps changes inside the declared bands.",
    },
  },
  {
    id: "markdown",
    phase: "Markdown",
    marketCondition: "Bear Market",
    action: "Preserve capital",
    description:
      "Limit exposure and protect capital during sustained declines.",
    risk: {
      level: "low",
      summary: "Stable liquidity can still lose purchasing power if the reference asset depegs.",
      mitigation:
        "Positions rotate into the auto-rebalanced stable position with per-asset weight caps.",
    },
  },
];
