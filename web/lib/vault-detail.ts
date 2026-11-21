/**
 * Vault detail fetching — issue #684.
 *
 * Loaded on demand (only when the detail modal/drawer actually opens), not
 * as part of the list/row fetch. There is no backend endpoint yet that
 * returns strategy/performance/risk data (`vaultService.get` only returns
 * `VaultRecord` — id/name/status/config_json), so this generates
 * deterministic, seeded mock data from the vault id. That keeps the UI
 * demoable and testable (same id always produces the same numbers) without
 * inventing a fake "looks real" backend contract that would need to be
 * un-mocked later.
 *
 * When `config_json` on the real record carries strategy/risk fields in the
 * future, `buildVaultDetail` is the single seam to start reading them instead
 * of generating them.
 */

import type {
  AuditStatus,
  Vault,
  VaultDetail,
  VaultHistoryPoint,
} from "@/types/vault";
import { fetchVaultData } from "./vault-metadata";

/** Deterministic string -> 32-bit seed (djb2). */
function hashSeed(seed: string): number {
  let hash = 5381;
  for (let i = 0; i < seed.length; i++) {
    hash = (hash * 33) ^ seed.charCodeAt(i);
  }
  return hash >>> 0;
}

/** Deterministic PRNG (mulberry32) seeded from a vault id — same id, same data. */
function mulberry32(seed: number): () => number {
  let a = seed;
  return function random() {
    a |= 0;
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const PROTOCOLS = ["Delta-neutral LP rotation", "Stable yield aggregation", "Covered call overlay"];
const AUDIT_STATUSES: AuditStatus[] = ["audited", "pending", "unaudited"];

/** Builds a deterministic historical performance series for the sparkline. */
export function buildMockVaultHistory(seed: string, points = 30): VaultHistoryPoint[] {
  const random = mulberry32(hashSeed(seed));
  const history: VaultHistoryPoint[] = [];
  let apy = 4 + random() * 8;
  let tvl = 200_000 + random() * 800_000;
  // Anchored to the start of the current UTC day (not the exact millisecond)
  // so repeated calls for the same id within the same day produce byte-for-
  // byte identical output, not just identical apy/tvl values.
  const now = Math.floor(Date.now() / 86_400_000) * 86_400_000;
  const dayMs = 24 * 60 * 60 * 1000;

  for (let i = points - 1; i >= 0; i--) {
    apy = Math.max(0, apy + (random() - 0.5) * 1.5);
    tvl = Math.max(0, tvl * (1 + (random() - 0.48) * 0.04));
    history.push({
      timestamp: new Date(now - i * dayMs).toISOString(),
      apy: Number(apy.toFixed(2)),
      tvl: Math.round(tvl),
    });
  }

  return history;
}

/** Builds a deterministic `VaultDetail` from a base `Vault` (same id -> same data). */
export function buildMockVaultDetail(base: Vault): VaultDetail {
  const random = mulberry32(hashSeed(base.id));
  const history = buildMockVaultHistory(base.id);
  const lastHistory = history[history.length - 1];

  return {
    ...base,
    strategy: {
      protocol: PROTOCOLS[Math.floor(random() * PROTOCOLS.length)],
      parameters: {
        rebalanceThresholdPct: Number((1 + random() * 4).toFixed(2)),
        targetUtilizationPct: Number((60 + random() * 30).toFixed(1)),
        maxLeverage: Number((1 + random() * 2).toFixed(2)),
      },
      riskScore: Math.round(random() * 100),
    },
    performance: {
      apy: lastHistory?.apy ?? base.apy ?? 0,
      tvl: lastHistory?.tvl ?? base.balance,
      sharpeRatio: Number((0.5 + random() * 2.5).toFixed(2)),
      maxDrawdown: Number((-(random() * 20)).toFixed(2)),
    },
    risk: {
      liquidationThreshold: Number((110 + random() * 40).toFixed(1)),
      oracleDeviationTolerance: Number((1 + random() * 4).toFixed(2)),
      auditStatus: AUDIT_STATUSES[Math.floor(random() * AUDIT_STATUSES.length)],
    },
    tokenInfo: {
      underlyingAsset: base.asset,
      underlyingAddress: mockContractAddress(`${base.id}-underlying`),
      shareToken: `p${base.asset}`,
      shareTokenAddress: mockContractAddress(`${base.id}-share`),
      decimals: 7,
    },
    history,
  };
}

/** Deterministic, Strkey-shaped (but non-cryptographic) mock contract address. */
function mockContractAddress(seed: string): string {
  const random = mulberry32(hashSeed(seed));
  const alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
  let out = "C";
  for (let i = 0; i < 55; i++) {
    out += alphabet[Math.floor(random() * alphabet.length)];
  }
  return out;
}

/**
 * Fetches full vault detail on demand. Falls back to deterministic mock data
 * when the base vault record itself falls back (demo/offline mode) — see
 * `fetchVaultData` in `lib/vault-metadata.ts`.
 */
export async function fetchVaultDetail(id: string): Promise<VaultDetail> {
  const base = await fetchVaultData(id);
  if (!base) {
    throw new Error(`Vault ${id} not found`);
  }
  return buildMockVaultDetail(base);
}
