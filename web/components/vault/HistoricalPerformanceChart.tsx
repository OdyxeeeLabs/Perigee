import { useId } from "react";
import { formatNumber } from "@/lib/locale/format";
import type { VaultHistoryPoint } from "@/types/vault";

export interface HistoricalPerformanceChartProps {
  history: VaultHistoryPoint[];
  /** Which series to plot. Defaults to "apy". */
  metric?: "apy" | "tvl";
  width?: number;
  height?: number;
  className?: string;
}

const formatDate = (timestamp: string): string => {
  const date = new Date(timestamp);
  if (Number.isNaN(date.getTime())) return timestamp;
  return date.toISOString().slice(0, 10);
};

/**
 * Historical performance sparkline — issue #684.
 *
 * `package.json` has no charting library, and `next.config.js` already
 * splits large vendor chunks deliberately (`mermaid`, `framer-motion`,
 * `@tanstack`) to keep bundle size down (#686). Adding a chart dependency for
 * one sparkline would work against that, so this is a small inline SVG
 * polyline — no dependency, fully themeable, and cheap to render inside a
 * modal that may open/close frequently.
 */
export function HistoricalPerformanceChart({
  history,
  metric = "apy",
  width = 480,
  height = 120,
  className = "",
}: HistoricalPerformanceChartProps) {
  const id = useId();
  const descriptionId = `${id}-description`;

  if (history.length === 0) {
    return (
      <div
        className={`flex w-full items-center justify-center text-xs text-slate-500 ${className}`}
        style={{ height }}
      >
        No historical data available.
      </div>
    );
  }

  const values = history.map((p) => p[metric]);
  const min = Math.min(...values);
  const max = Math.max(...values);
  const range = max - min || 1;
  const padding = 8;

  const points = values.map((value, index) => {
    const x =
      history.length === 1
        ? padding
        : padding + (index / (history.length - 1)) * (width - padding * 2);
    const y = height - padding - ((value - min) / range) * (height - padding * 2);
    return [x, y] as const;
  });

  const polyline = points.map(([x, y]) => `${x.toFixed(1)},${y.toFixed(1)}`).join(" ");
  const areaPath = `M${points[0][0].toFixed(1)},${height} L${polyline
    .split(" ")
    .join(" L")} L${points[points.length - 1][0].toFixed(1)},${height} Z`;

  const first = values[0];
  const last = values[values.length - 1];
  const trendUp = last >= first;
  const metricLabel = metric === "apy" ? "APY" : "TVL";
  const startTime = formatDate(history[0].timestamp);
  const endTime = formatDate(history[history.length - 1].timestamp);
  const description =
    `Historical ${metricLabel} from ${startTime} to ${endTime}: ` +
    `${history.length} data points, trending ${trendUp ? "up" : "down"} ` +
    `from ${first.toLocaleString()} to ${last.toLocaleString()}. ` +
    `Lowest ${min.toLocaleString()}, highest ${max.toLocaleString()}.`;

  return (
    <figure className={className} style={{ width }} data-testid="historical-performance-chart">
    <div className={`w-full ${className}`}>
      <svg
        viewBox={`0 0 ${width} ${height}`}
        width="100%"
        height={height}
        role="img"
        aria-label={`Historical ${metricLabel} chart, ${history.length} data points, trending ${
          trendUp ? "up" : "down"
        } from ${first.toLocaleString()} to ${last.toLocaleString()}`}
        aria-describedby={descriptionId}
        data-testid="historical-performance-chart-svg"
        aria-label={`Historical ${metric === "apy" ? "APY" : "TVL"} chart, ${history.length} data points, trending ${
          trendUp ? "up" : "down"
        } from ${formatNumber(first)} to ${formatNumber(last)}`}
      >
        <path d={areaPath} fill={trendUp ? "rgba(52,211,153,0.12)" : "rgba(248,113,113,0.12)"} stroke="none" />
        <polyline
          points={polyline}
          fill="none"
          stroke={trendUp ? "#34d399" : "#f87171"}
          strokeWidth={2}
          strokeLinejoin="round"
          strokeLinecap="round"
        />
        <circle
          cx={points[points.length - 1][0]}
          cy={points[points.length - 1][1]}
          r={3}
          fill={trendUp ? "#34d399" : "#f87171"}
        />
      </svg>
      <figcaption id={descriptionId} className="sr-only" data-testid="historical-performance-chart-description">
        {description}
      </figcaption>
      <table className="sr-only" data-testid="historical-performance-chart-data">
        <caption>
          Historical {metricLabel} values from {startTime} to {endTime}
        </caption>
        <thead>
          <tr>
            <th scope="col">Date</th>
            <th scope="col">{metricLabel}</th>
          </tr>
        </thead>
        <tbody>
          {history.map((point, index) => (
            <tr key={`${point.timestamp}-${index}`}>
              <th scope="row">{formatDate(point.timestamp)}</th>
              <td>{point[metric].toLocaleString()}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </figure>
      <div className="mt-1 flex flex-wrap gap-x-3 gap-y-0.5 text-[11px] text-slate-400">
        <span>First: <strong className="font-semibold text-slate-200">{formatNumber(first)}</strong></span>
        <span>Latest: <strong className="font-semibold text-slate-200">{formatNumber(last)}</strong></span>
        <span>
          Trend:{" "}
          <strong className={trendUp ? "font-semibold text-emerald-400" : "font-semibold text-red-400"}>
            {trendUp ? "Up" : "Down"}
          </strong>
        </span>
      </div>
    </div>
  );
}
