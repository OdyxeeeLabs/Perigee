import { Fragment } from "react";
import type { VaultDetail } from "@/types/vault";
import { shortenAddress } from "@/lib/utils";
import { CopyButton } from "../ui/CopyButton";
import { HistoricalPerformanceChart } from "./HistoricalPerformanceChart";

const auditBadgeClasses: Record<VaultDetail["risk"]["auditStatus"], string> = {
  audited: "bg-emerald-950 text-emerald-400",
  pending: "bg-amber-950 text-amber-400",
  unaudited: "bg-rose-950 text-rose-400",
};

export interface VaultDetailContentProps {
  vault: VaultDetail;
  /** Deposit / withdraw / manage — wired to placeholder actions (see note below). */
  onDeposit?: (vault: VaultDetail) => void;
  onWithdraw?: (vault: VaultDetail) => void;
  onManage?: (vault: VaultDetail) => void;
}

/**
 * Vault detail sections — issue #684.
 *
 * Pure presentational component shared by `VaultDetailModal` (overlay) and
 * the `/vault/[id]` and `/vaults/[id]` page routes (full page), so the two
 * surfaces never drift out of sync.
 */
export function VaultDetailContent({ vault, onDeposit, onWithdraw, onManage }: VaultDetailContentProps) {
  return (
    <div className="flex flex-col gap-6">
      {/* Token info */}
      <section aria-labelledby="vault-token-info-heading">
        <h3 id="vault-token-info-heading" className="text-xs font-semibold uppercase tracking-wider text-slate-400">
          Token info
        </h3>
        <dl className="mt-2 grid grid-cols-2 gap-x-4 gap-y-2 text-sm">
          <dt className="text-slate-500">Underlying</dt>
          <dd className="text-right font-mono text-slate-200">{vault.tokenInfo.underlyingAsset}</dd>
          <dt className="text-slate-500">Underlying address</dt>
          <dd className="flex items-center justify-end gap-1 font-mono text-xs text-slate-400">
            <span title={vault.tokenInfo.underlyingAddress}>
              {shortenAddress(vault.tokenInfo.underlyingAddress)}
            </span>
            <CopyButton value={vault.tokenInfo.underlyingAddress} label="underlying token address" />
          </dd>
          <dt className="text-slate-500">Share token</dt>
          <dd className="text-right font-mono text-slate-200">{vault.tokenInfo.shareToken}</dd>
          <dt className="text-slate-500">Share token address</dt>
          <dd className="flex items-center justify-end gap-1 font-mono text-xs text-slate-400">
            <span title={vault.tokenInfo.shareTokenAddress}>
              {shortenAddress(vault.tokenInfo.shareTokenAddress)}
            </span>
            <CopyButton value={vault.tokenInfo.shareTokenAddress} label="share token address" />
          </dd>
          <dt className="text-slate-500">Decimals</dt>
          <dd className="text-right font-mono text-slate-200">{vault.tokenInfo.decimals}</dd>
        </dl>
      </section>

      {/* Performance metrics */}
      <section aria-labelledby="vault-performance-heading">
        <h3 id="vault-performance-heading" className="text-xs font-semibold uppercase tracking-wider text-slate-400">
          Performance
        </h3>
        <div className="mt-2 grid grid-cols-2 gap-3 sm:grid-cols-4">
          <Metric label="APY" value={`${vault.performance.apy}%`} tone="emerald" />
          <Metric label="TVL" value={`${vault.performance.tvl.toLocaleString()} ${vault.asset}`} />
          <Metric label="Sharpe ratio" value={vault.performance.sharpeRatio.toFixed(2)} />
          <Metric label="Max drawdown" value={`${vault.performance.maxDrawdown}%`} tone="rose" />
        </div>
      </section>

      {/* Historical performance chart */}
      <section aria-labelledby="vault-history-heading">
        <h3 id="vault-history-heading" className="text-xs font-semibold uppercase tracking-wider text-slate-400">
          Historical APY
        </h3>
        <div className="mt-2 rounded-lg border border-slate-800 bg-slate-900/60 p-3">
          <HistoricalPerformanceChart history={vault.history} metric="apy" height={100} />
        </div>
      </section>

      {/* Strategy configuration */}
      <section aria-labelledby="vault-strategy-heading">
        <h3 id="vault-strategy-heading" className="text-xs font-semibold uppercase tracking-wider text-slate-400">
          Strategy configuration
        </h3>
        <div className="mt-2 rounded-lg border border-slate-800 bg-slate-900/60 p-3 text-sm">
          <div className="flex items-center justify-between">
            <span className="text-slate-200">{vault.strategy.protocol}</span>
            <span className="text-xs text-slate-400">Risk score {vault.strategy.riskScore}/100</span>
          </div>
          <div className="mt-2 h-1.5 w-full overflow-hidden rounded-full bg-slate-800">
            <div
              className="h-full rounded-full bg-gradient-to-r from-emerald-400 via-amber-400 to-rose-400"
              style={{ width: `${vault.strategy.riskScore}%` }}
            />
          </div>
          <dl className="mt-3 grid grid-cols-2 gap-x-4 gap-y-1 text-xs">
            {Object.entries(vault.strategy.parameters).map(([key, value]) => (
              <Fragment key={key}>
                <dt className="text-slate-500">{key}</dt>
                <dd className="text-right font-mono text-slate-300">{value}</dd>
              </Fragment>
            ))}
          </dl>
        </div>
      </section>

      {/* Risk parameters */}
      <section aria-labelledby="vault-risk-heading">
        <h3 id="vault-risk-heading" className="text-xs font-semibold uppercase tracking-wider text-slate-400">
          Risk parameters
        </h3>
        <div className="mt-2 flex flex-wrap items-center gap-3 text-sm">
          <span className="text-slate-400">
            Liquidation threshold <span className="font-mono text-slate-200">{vault.risk.liquidationThreshold}%</span>
          </span>
          <span className="text-slate-400">
            Oracle deviation tolerance{" "}
            <span className="font-mono text-slate-200">{vault.risk.oracleDeviationTolerance}%</span>
          </span>
          <span className={`rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase ${auditBadgeClasses[vault.risk.auditStatus]}`}>
            {vault.risk.auditStatus}
          </span>
        </div>
      </section>

      {/* Actions */}
      <section aria-labelledby="vault-actions-heading" className="flex flex-wrap gap-2 border-t border-slate-800 pt-4">
        <h3 id="vault-actions-heading" className="sr-only">
          Actions
        </h3>
        <button
          type="button"
          onClick={() => onDeposit?.(vault)}
          className="rounded bg-cyan-500 px-4 py-2 text-sm font-medium text-white hover:bg-cyan-600"
        >
          Deposit
        </button>
        <button
          type="button"
          onClick={() => onWithdraw?.(vault)}
          className="rounded bg-slate-800 px-4 py-2 text-sm font-medium text-slate-200 hover:bg-slate-700"
        >
          Withdraw
        </button>
        <button
          type="button"
          onClick={() => onManage?.(vault)}
          className="rounded border border-slate-700 px-4 py-2 text-sm font-medium text-slate-300 hover:bg-slate-800"
        >
          Manage
        </button>
      </section>
    </div>
  );
}

function Metric({ label, value, tone }: { label: string; value: string; tone?: "emerald" | "rose" }) {
  const toneClass = tone === "emerald" ? "text-emerald-400" : tone === "rose" ? "text-rose-400" : "text-slate-200";
  return (
    <div className="rounded-lg border border-slate-800 bg-slate-900/60 px-3 py-2">
      <div className="text-[10px] uppercase tracking-wide text-slate-500">{label}</div>
      <div className={`font-mono text-sm font-semibold ${toneClass}`}>{value}</div>
    </div>
  );
}
