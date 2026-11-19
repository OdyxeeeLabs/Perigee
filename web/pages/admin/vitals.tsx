import { useEffect, useState } from "react";
import type { Distribution, VitalsSummary } from "../../lib/vitalsStats";

type Response = VitalsSummary & { sampleCount: number };

function DistributionTable({
  title,
  groups,
}: {
  title: string;
  groups: Record<string, Record<string, Distribution>>;
}) {
  const rows = Object.entries(groups).flatMap(([group, metrics]) =>
    Object.entries(metrics).map(([metric, d]) => ({ group, metric, ...d })),
  );
  if (rows.length === 0) return null;
  return (
    <section className="mb-8">
      <h2 className="mb-2 text-lg font-semibold text-white">{title}</h2>
      <table className="w-full text-left text-xs text-slate-300">
        <thead className="text-slate-500">
          <tr>
            <th className="py-1">Group</th>
            <th>Metric</th>
            <th>n</th>
            <th>p50</th>
            <th>p75</th>
            <th>p95</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r) => (
            <tr key={`${r.group}-${r.metric}`} className="border-t border-slate-800">
              <td className="py-1 font-mono">{r.group}</td>
              <td>{r.metric}</td>
              <td>{r.count}</td>
              <td>{r.p50}</td>
              <td>{r.p75}</td>
              <td>{r.p95}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </section>
  );
}

export default function VitalsDashboard() {
  const [data, setData] = useState<Response | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    fetch("/api/vitals")
      .then((res) => (res.ok ? res.json() : Promise.reject(new Error(`HTTP ${res.status}`))))
      .then(setData)
      .catch((err: Error) => setError(err.message));
  }, []);

  return (
    <main className="mx-auto max-w-4xl p-6 text-slate-200">
      <h1 className="mb-1 text-2xl font-bold text-white">Web Vitals</h1>
      <p className="mb-6 text-sm text-slate-400">
        Percentile distributions of consented real-user measurements (ms; CLS is unitless).
      </p>

      {error && <p role="alert">Could not load vitals: {error}</p>}
      {!data && !error && <p>Loading…</p>}

      {data && (
        <>
          <p className="mb-4 text-sm">Samples: {data.sampleCount}</p>
          {data.alerts.length > 0 && (
            <ul role="alert" className="mb-6 space-y-1 text-sm">
              {data.alerts.map((a) => (
                <li key={a.metric} className={a.severity === "critical" ? "text-red-400" : "text-amber-400"}>
                  {a.metric} p75 {a.p75} exceeds {a.severity === "critical" ? "poor" : "good"} threshold{" "}
                  {a.threshold}
                </li>
              ))}
            </ul>
          )}
          <DistributionTable title="Overall" groups={{ all: data.overall }} />
          <DistributionTable title="By page" groups={data.byPage} />
          <DistributionTable title="By device" groups={data.byDevice} />
          <DistributionTable title="By connection" groups={data.byConnection} />
        </>
      )}
    </main>
  );
}
