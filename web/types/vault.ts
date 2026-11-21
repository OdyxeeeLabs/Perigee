export type VaultStatus =
  | "ACTIVE"
  | "LOCKED"
  | "PENDING"
  | "ARCHIVED";

export interface Vault {
  id: string;

  name: string;

  owner: string;

  balance: number;

  asset: string;

  apy?: number;

  status: VaultStatus;

  createdAt: string;

  updatedAt: string;
}

// ---------------------------------------------------------------------------
// Vault detail — issue #684 (expandable vault detail modal/drawer)
//
// The base `Vault` above is deliberately lean (list/row rendering). The
// detail modal needs richer, more expensive-to-compute data, so it is a
// separate type loaded on demand via `lib/vault-detail.ts` rather than
// fields bolted onto every list-row fetch.
// ---------------------------------------------------------------------------

export type AuditStatus = "audited" | "pending" | "unaudited";

export interface VaultStrategyConfig {
  /** Underlying protocol/strategy name, e.g. "Delta-neutral LP rotation". */
  protocol: string;
  /** Free-form strategy parameters (rebalance thresholds, target weights, …). */
  parameters: Record<string, string | number>;
  /** 0 (lowest risk) – 100 (highest risk). */
  riskScore: number;
}

export interface VaultPerformanceMetrics {
  apy: number;
  /** Total value locked, denominated in `Vault.asset`. */
  tvl: number;
  sharpeRatio: number;
  /** Largest peak-to-trough decline, as a negative percentage. */
  maxDrawdown: number;
}

export interface VaultRiskParameters {
  /** Collateralization ratio below which positions may be liquidated, percent. */
  liquidationThreshold: number;
  /** Maximum tolerated oracle price deviation before circuit breakers trip, percent. */
  oracleDeviationTolerance: number;
  auditStatus: AuditStatus;
}

export interface VaultTokenInfo {
  underlyingAsset: string;
  underlyingAddress: string;
  shareToken: string;
  shareTokenAddress: string;
  decimals: number;
}

export interface VaultHistoryPoint {
  /** ISO 8601 timestamp. */
  timestamp: string;
  apy: number;
  tvl: number;
}

export interface VaultDetail extends Vault {
  strategy: VaultStrategyConfig;
  performance: VaultPerformanceMetrics;
  risk: VaultRiskParameters;
  tokenInfo: VaultTokenInfo;
  /** Ascending by timestamp; used for the historical performance sparkline. */
  history: VaultHistoryPoint[];
}