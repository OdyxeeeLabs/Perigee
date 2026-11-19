import { VITAL_THRESHOLDS, type VitalPayload } from "./webVitals";

/** Nearest-rank percentile of `values` (0 < p <= 100); 0 for an empty list. */
export function percentile(values: number[], p: number): number {
  if (values.length === 0) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  const rank = Math.ceil((p / 100) * sorted.length);
  return sorted[Math.min(sorted.length, Math.max(1, rank)) - 1];
}

export interface Distribution {
  count: number;
  p50: number;
  p75: number;
  p95: number;
}

export function distribution(values: number[]): Distribution {
  return {
    count: values.length,
    p50: percentile(values, 50),
    p75: percentile(values, 75),
    p95: percentile(values, 95),
  };
}

export interface VitalsSummary {
  overall: Record<string, Distribution>;
  byPage: Record<string, Record<string, Distribution>>;
  byDevice: Record<string, Record<string, Distribution>>;
  byConnection: Record<string, Record<string, Distribution>>;
  /** Metrics whose p75 exceeds the "good" threshold. */
  alerts: Array<{ metric: string; p75: number; threshold: number; severity: "warning" | "critical" }>;
}

function groupBy(
  samples: VitalPayload[],
  key: (s: VitalPayload) => string,
): Record<string, Record<string, Distribution>> {
  const buckets: Record<string, Record<string, number[]>> = {};
  for (const s of samples) {
    const group = (buckets[key(s)] ??= {});
    (group[s.name] ??= []).push(s.value);
  }
  const out: Record<string, Record<string, Distribution>> = {};
  for (const [g, metrics] of Object.entries(buckets)) {
    out[g] = Object.fromEntries(Object.entries(metrics).map(([m, v]) => [m, distribution(v)]));
  }
  return out;
}

export function summarize(samples: VitalPayload[]): VitalsSummary {
  const overallBuckets: Record<string, number[]> = {};
  for (const s of samples) (overallBuckets[s.name] ??= []).push(s.value);
  const overall = Object.fromEntries(
    Object.entries(overallBuckets).map(([m, v]) => [m, distribution(v)]),
  );

  const alerts: VitalsSummary["alerts"] = [];
  for (const [metric, dist] of Object.entries(overall)) {
    const t = VITAL_THRESHOLDS[metric as keyof typeof VITAL_THRESHOLDS];
    if (!t || dist.count === 0) continue;
    if (dist.p75 > t[1]) alerts.push({ metric, p75: dist.p75, threshold: t[1], severity: "critical" });
    else if (dist.p75 > t[0]) alerts.push({ metric, p75: dist.p75, threshold: t[0], severity: "warning" });
  }

  return {
    overall,
    byPage: groupBy(samples, (s) => s.page),
    byDevice: groupBy(samples, (s) => s.device),
    byConnection: groupBy(samples, (s) => s.connection),
    alerts,
  };
}
