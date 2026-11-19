import { useState, useMemo, useCallback, useEffect } from "react";
import Head from "next/head";
import Link from "next/link";
import { useRouter } from "next/router";
import {
  BarChart2,
  TrendingUp,
  TrendingDown,
  Wallet,
  Lock,
  Archive,
  Clock,
  ArrowUpRight,
} from "lucide-react";

import { SEO } from "@/components/SEO";
import { useDataRefresh } from "@/hooks/useDataRefresh";
import { useRouteChangePause } from "@/hooks/useRouteChangePause";
import { useLocale } from "@/components/LocaleSync";
import { FormattedAmount, FormattedPercent } from "@/components/ui/FormattedAmount";
import {
  formatNumber,
  formatPercent,
  formatRawNumber,
  pluralize,
} from "@/lib/locale/format";
import { Vault, VaultStatus } from "@/types/vault";

// ---------------------------------------------------------------------------
// Demo data — replace with real API call once the endpoint is available
// ---------------------------------------------------------------------------

const MOCK_VAULTS: Vault[] = Array.from({ length: 12 }, (_, i) => ({
  id: `vault-${i + 1}`,
  name: `Vault ${i + 1}`,
  owner: "GAXI...B7QK",
  balance: Math.round(Math.random() * 500_000 + 1_000),
  asset: ["XLM", "USDC", "BTC", "ETH"][i % 4],
  apy: Math.random() * 18 + 1,
  status: (["ACTIVE", "ACTIVE", "ACTIVE", "LOCKED", "PENDING", "ARCHIVED"] as VaultStatus[])[
    i % 6
  ],
  createdAt: new Date(Date.now() - i * 86_400_000 * 10).toISOString(),
  updatedAt: new Date().toISOString(),
}));

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function isVault(value: unknown): value is Vault {
  if (!value || typeof value !== "object") return false;
  const vault = value as Partial<Vault>;
  return (
    typeof vault.id === "string" &&
    typeof vault.name === "string" &&
    typeof vault.owner === "string" &&
    typeof vault.balance === "number" &&
    typeof vault.asset === "string" &&
    typeof vault.status === "string"
  );
}

async function loadVaults(): Promise<Vault[]> {
  try {
    const response = await fetch("/api/vaults", { cache: "no-store" });
    if (!response.ok) return MOCK_VAULTS;
    const data: unknown = await response.json();
    return Array.isArray(data) && data.every(isVault) ? data : MOCK_VAULTS;
  } catch {
    return MOCK_VAULTS;
  }
}

type AssetBreakdown = { asset: string; balance: number; count: number };

function getAssetBreakdown(vaults: Vault[]): AssetBreakdown[] {
  const map = new Map<string, AssetBreakdown>();
  for (const v of vaults) {
    const existing = map.get(v.asset) ?? { asset: v.asset, balance: 0, count: 0 };
    map.set(v.asset, {
      asset: v.asset,
      balance: existing.balance + v.balance,
      count: existing.count + 1,
    });
  }
  return Array.from(map.values()).sort((a, b) => b.balance - a.balance);
}

const STATUS_META: Record<
  VaultStatus,
  { label: string; color: string; icon: React.ReactNode }
> = {
  ACTIVE: {
    label: "Active",
    color: "text-emerald-400 bg-emerald-950 border-emerald-800",
    icon: <TrendingUp size={12} />,
  },
  LOCKED: {
    label: "Locked",
    color: "text-amber-400 bg-amber-950 border-amber-800",
    icon: <Lock size={12} />,
  },
  PENDING: {
    label: "Pending",
    color: "text-sky-400 bg-sky-950 border-sky-800",
    icon: <Clock size={12} />,
  },
  ARCHIVED: {
    label: "Archived",
    color: "text-slate-400 bg-slate-800 border-slate-700",
    icon: <Archive size={12} />,
  },
};

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

function StatCard({
  label,
  value,
  sub,
  icon,
  trend,
}: {
  label: string;
  value: React.ReactNode;
  sub?: string;
  icon: React.ReactNode;
  trend?: "up" | "down" | "neutral";
}) {
  return (
    <div className="rounded-xl border border-slate-800 bg-slate-900/60 p-5 backdrop-blur">
      <div className="flex items-center justify-between">
        <p className="text-xs font-medium text-slate-400">{label}</p>
        <span className="flex h-8 w-8 items-center justify-center rounded-lg bg-slate-800 text-slate-400">
          {icon}
        </span>
      </div>
      <p className="mt-3 text-2xl font-bold text-white">{value}</p>
      {sub && (
        <p
          className={`mt-1 flex items-center gap-1 text-xs ${
            trend === "up"
              ? "text-emerald-400"
              : trend === "down"
              ? "text-red-400"
              : "text-slate-500"
          }`}
        >
          {trend === "up" && <TrendingUp size={11} />}
          {trend === "down" && <TrendingDown size={11} />}
          {sub}
        </p>
      )}
    </div>
  );
}

function AssetBar({
  breakdown,
  total,
  locale,
}: {
  breakdown: AssetBreakdown[];
  total: number;
  locale: string;
}) {
  const COLORS = [
    "bg-sky-500",
    "bg-violet-500",
    "bg-amber-500",
    "bg-emerald-500",
    "bg-rose-500",
  ];
  const locale = useLocale();

  if (breakdown.length === 0 || total <= 0) {
    return <p className="py-6 text-center text-xs text-slate-500">No assets to display yet.</p>;
  }

  return (
    <div>
      {/* Stacked bar */}
      <div className="flex h-3 w-full overflow-hidden rounded-full">
        {breakdown.map((item, i) => (
          <div
            key={item.asset}
            className={`${COLORS[i % COLORS.length]} transition-all`}
            style={{ width: `${(item.balance / total) * 100}%` }}
            title={`${item.asset}: ${formatPercent((item.balance / total) * 100, locale, 1)} (${formatRawNumber((item.balance / total) * 100, locale)}%)`}
          />
        ))}
      </div>

      {/* Legend */}
      <ul className="mt-4 space-y-2">
        {breakdown.map((item, i) => (
          <li key={item.asset} className="flex items-center justify-between text-sm">
            <span className="flex items-center gap-2">
              <span
                className={`h-2.5 w-2.5 rounded-full ${COLORS[i % COLORS.length]}`}
              />
              <span className="text-slate-300">{item.asset}</span>
              <span className="text-xs text-slate-500">
                ({pluralize(item.count, { one: "{count} vault", other: "{count} vaults" }, locale)})
              </span>
            </span>
            <span className="font-medium text-slate-200">
              <FormattedAmount value={item.balance} locale={locale} />{" "}
              <span className="text-xs text-slate-500">
                (<FormattedPercent value={(item.balance / total) * 100} locale={locale} digits={1} />)
              </span>
            </span>
          </li>
        ))}
      </ul>
    </div>
  );
}

function StatusBreakdown({ vaults }: { vaults: Vault[] }) {
  const locale = useLocale();
  const counts = useMemo(() => {
    const m: Partial<Record<VaultStatus, number>> = {};
    for (const v of vaults) m[v.status] = (m[v.status] ?? 0) + 1;
    return m;
  }, [vaults]);

  return (
    <div className="grid grid-cols-2 gap-3">
      {(Object.keys(STATUS_META) as VaultStatus[]).map((s) => {
        const meta = STATUS_META[s];
        const count = counts[s] ?? 0;
        return (
          <div
            key={s}
            className={`flex items-center gap-2.5 rounded-lg border px-3 py-2.5 ${meta.color}`}
          >
            {meta.icon}
            <span className="text-sm font-medium">{meta.label}</span>
            <span className="ms-auto text-lg font-bold">
              {formatNumber(count, locale, { maximumFractionDigits: 0 })}
            </span>
          </div>
        );
      })}
    </div>
  );
}

function TopVaultRow({ vault }: { vault: Vault }) {
  const locale = useLocale();

  return (
    <Link
      href={`/vault/${vault.id}`}
      className="
        flex
        items-center
        justify-between
        rounded-lg
        border
        border-slate-800
        bg-slate-900/40
        px-4
        py-3
        transition-colors
        hover:border-slate-700
        hover:bg-slate-800/60
      "
    >
      <div className="min-w-0">
        <p className="truncate text-sm font-medium text-slate-200">{vault.name}</p>
        <p className="mt-0.5 text-xs text-slate-500">{vault.asset}</p>
      </div>

      <div className="flex items-center gap-4">
        <div className="text-end">
          <p className="text-sm font-semibold text-white">
            <FormattedAmount value={vault.balance} locale={locale} />
          </p>
          <p className="text-xs text-emerald-400">
            <FormattedPercent value={vault.apy ?? 0} locale={locale} digits={2} /> APY
          </p>
        </div>
        <ArrowUpRight size={14} className="text-slate-500" />
      </div>
    </Link>
  );
}

// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------

/**
 * Portfolio Overview page.
 *
 * Shows:
 * - Total balance, vault count, average APY, best-performing vault
 * - Asset allocation breakdown (stacked bar + legend)
 * - Status breakdown (Active / Locked / Pending / Archived)
 * - Top 5 vaults by balance
 *
 * Resolves #683 — Build portfolio overview.
 */
export default function PortfolioOverviewPage() {
  const [vaults, setVaults] = useState<Vault[]>(MOCK_VAULTS);
  const [lastRefreshedAt, setLastRefreshedAt] = useState<number | null>(null);
  const routeChanging = useRouteChangePause();

  const refreshVaults = useCallback(async () => {
    const next = await loadVaults();
    setVaults(next);
    setLastRefreshedAt(Date.now());
  }, []);

  const { intervalMs: refreshIntervalMs } = useDataRefresh(() => {
    void refreshVaults();
  }, { enabled: !routeChanging });

  useEffect(() => {
    void refreshVaults();
  }, [refreshVaults]);
  // In production wire this to a real data-fetching hook / SWR call.
  const [vaults] = useState<Vault[]>(MOCK_VAULTS);
  const locale = useLocale();

  const totalBalance = useMemo(
    () => vaults.reduce((s, v) => s + v.balance, 0),
    [vaults],
  );

  const avgApy = useMemo(() => {
    const active = vaults.filter((v) => v.apy !== undefined);
    if (active.length === 0) return 0;
    return active.reduce((s, v) => s + (v.apy ?? 0), 0) / active.length;
  }, [vaults]);

  const topVaults = useMemo(
    () => [...vaults].sort((a, b) => b.balance - a.balance).slice(0, 5),
    [vaults],
  );

  const assetBreakdown = useMemo(() => getAssetBreakdown(vaults), [vaults]);
  const hasVaults = vaults.length > 0;

  const createdInRange = useMemo(() => {
    if (!from) return vaults.length;
    const start = from.getTime();
    const end = to.getTime();
    return vaults.filter((vault) => {
      const created = Date.parse(vault.createdAt);
      return Number.isFinite(created) && created >= start && created <= end;
    }).length;
  }, [vaults, from, to]);

  return (
    <>
      <Head>
        <link rel="icon" href="/favicon.ico" />
      </Head>
      <SEO
        title="Portfolio Overview"
        description="View your vault portfolio breakdown — balances, assets, APY, and status."
        path="/portfolio"
      />

      <main className="min-h-screen bg-slate-950 text-slate-100">
        {/* Header */}
        <header className="sticky top-0 z-40 border-b border-slate-800 bg-slate-950/90 backdrop-blur">
          <div className="mx-auto flex max-w-6xl flex-wrap items-center justify-between gap-2 px-4 py-4 sm:px-6 lg:px-8">
            <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
              <Link href="/" className="text-2xl font-bold text-cyan-400 hover:text-cyan-300">
                Perigee
              </Link>
              <span className="ms-3 text-xs uppercase tracking-wider text-slate-500">
                Portfolio Overview
              </span>
            </div>
            <span className="hidden text-xs text-slate-500 md:inline">
              Auto-refresh: {refreshIntervalMs / 1000}s
              {lastRefreshedAt !== null &&
                ` · updated ${new Date(lastRefreshedAt).toLocaleTimeString()}`}
            </span>
            <Link
              href="/"
              className="text-sm font-medium text-slate-400 hover:text-cyan-400 transition-colors"
            >
              &larr; Dashboard
            </Link>
          </div>
        </header>

        <div className="mx-auto max-w-6xl px-4 py-8 sm:px-6 lg:px-8">
          {/* Page title */}
          <div className="mb-8">
            <h1 className="flex items-center gap-3 text-3xl font-bold text-white">
              <BarChart2 className="text-cyan-400" size={28} />
              Portfolio Overview
            </h1>
            <p className="mt-1 text-sm text-slate-400">
              All vaults &mdash; balances, allocations, and performance at a glance.
            </p>
            <div className="mt-4 flex flex-col gap-2 sm:flex-row sm:items-center sm:gap-4">
              <label
                htmlFor="comparison-range"
                className="text-xs font-medium text-slate-400"
              >
                Comparison period
              </label>
              <select
                id="comparison-range"
                data-testid="comparison-range"
                value={range}
                onChange={(event) => setRange(event.target.value as DateRangeId)}
                className="rounded-lg border border-slate-700 bg-slate-900 px-3 py-1.5 text-sm text-slate-200 focus:border-cyan-500 focus:outline-none focus:ring-1 focus:ring-cyan-500"
              >
                {DATE_RANGE_PRESETS.map((preset) => (
                  <option key={preset.id} value={preset.id}>
                    {preset.label}
                  </option>
                ))}
              </select>
              <span className="text-xs text-slate-500" data-testid="range-label">
                {rangeLabel}
              </span>
            </div>
          </div>

          {/* KPI cards */}
          <div className="mb-8 grid grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-4">
            <StatCard
              label="Total Balance"
              value={<FormattedAmount value={totalBalance} locale={locale} />}
              sub="Across all assets"
              icon={<Wallet size={16} />}
              trend="neutral"
            />
            <StatCard
              label="Total Vaults"
              value={formatNumber(vaults.length, locale, { maximumFractionDigits: 0 })}
              sub={`${formatNumber(vaults.filter((v) => v.status === "ACTIVE").length, locale, { maximumFractionDigits: 0 })} active`}
              icon={<BarChart2 size={16} />}
              trend="up"
            />
            <StatCard
              label="Average APY"
              value={<FormattedPercent value={avgApy} locale={locale} digits={2} />}
              sub="Across active vaults"
              icon={<TrendingUp size={16} />}
              trend="up"
            />
            <StatCard
              label="Best APY"
              value={
                <FormattedPercent
                  value={Math.max(...vaults.map((v) => v.apy ?? 0))}
                  locale={locale}
                  digits={2}
                />
              }
              sub={topVaults[0]?.name ?? "—"}
              icon={<TrendingUp size={16} />}
              trend="up"
            />
            <StatCard
              label="New Vaults (Period)"
              value={formatNumber(createdInRange, locale)}
              sub={rangeLabel}
              icon={<Clock size={16} />}
              trend="neutral"
            />
          </div>

          {/* Two-column layout */}
          <div className="grid grid-cols-1 gap-6 lg:grid-cols-2">
            {/* Asset allocation */}
            <section
              aria-labelledby="allocation-heading"
              className="rounded-xl border border-slate-800 bg-slate-900/40 p-6"
            >
              <h2
                id="allocation-heading"
                className="mb-4 text-base font-semibold text-white"
              >
                Asset Allocation
              </h2>
              <AssetBar breakdown={assetBreakdown} total={totalBalance} locale={locale} />
            </section>

            {/* Vault status */}
            <section
              aria-labelledby="status-heading"
              className="rounded-xl border border-slate-800 bg-slate-900/40 p-6"
            >
              <h2
                id="status-heading"
                className="mb-4 text-base font-semibold text-white"
              >
                Vault Status
              </h2>
              <StatusBreakdown vaults={vaults} />
            </section>
          </div>

          {/* Top vaults */}
          <section
            aria-labelledby="top-vaults-heading"
            className="mt-6 rounded-xl border border-slate-800 bg-slate-900/40 p-6"
          >
            <h2
              id="top-vaults-heading"
              className="mb-4 text-base font-semibold text-white"
            >
              Top Vaults by Balance
            </h2>
            <div className="space-y-2">
              {topVaults.map((vault) => (
                <TopVaultRow key={vault.id} vault={vault} locale={locale} />
              ))}
            </div>
            {hasVaults ? (
              <div className="space-y-2">
                {topVaults.map((vault) => (
                  <TopVaultRow key={vault.id} vault={vault} />
                ))}
              </div>
            ) : (
              <p role="status" className="py-6 text-center text-xs text-slate-500">
                No vaults to analyse yet.
              </p>
            )}
          </section>
        </div>
      </main>
    </>
  );
}
