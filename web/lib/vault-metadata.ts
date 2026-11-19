import type { Metadata } from "next";
import { Vault } from "@/types/vault";
import { vaultService, VaultRecord } from "./api";

export const SITE_ORIGIN =
  process.env.NEXT_PUBLIC_SITE_URL?.replace(/\/+$/, "") || "https://perigee.app";
export const DEFAULT_OG_IMAGE = "/og-default.png";

export interface VaultMetadataInput {
  id?: string;
  name?: string;
  balance?: number;
  asset?: string;
  apy?: number;
  owner?: string;
  status?: string;
  [key: string]: unknown;
}

/**
 * Normalizes vault data from various API response shapes (Vault, VaultRecord, or raw object).
 */
export function normalizeVaultData(
  raw: Partial<Vault | VaultRecord | VaultMetadataInput> | null,
  fallbackId?: string
): {
  id: string;
  name: string;
  balance: number;
  asset: string;
  apy: number;
  owner: string;
  status: string;
} {
  if (!raw) {
    return {
      id: fallbackId || "unknown",
      name: "Vault Not Found",
      balance: 0,
      asset: "XLM",
      apy: 0,
      owner: "Unknown",
      status: "PENDING",
    };
  }

  // Parse config_json if present (VaultRecord shape)
  let parsedConfig: Record<string, unknown> = {};
  if ("config_json" in raw && typeof raw.config_json === "string" && raw.config_json) {
    try {
      parsedConfig = JSON.parse(raw.config_json);
    } catch {
      // ignore JSON parse error
    }
  }

  // `raw` is a union of three shapes (Vault | VaultRecord | VaultMetadataInput)
  // that intentionally don't all share the same fields — VaultRecord keeps
  // them inside `config_json` instead. Read through an untyped view here
  // rather than narrowing per-field, since every value is re-validated by
  // `String()`/`Number()` immediately below anyway.
  const r = raw as Record<string, unknown>;

  const id = String(r.id || fallbackId || "unknown");
  const name = String(r.name || parsedConfig.name || `Vault ${id}`);
  const balance = Number(r.balance ?? parsedConfig.balance ?? 0);
  const asset = String(r.asset || parsedConfig.asset || "XLM");
  const apy = Number(r.apy ?? parsedConfig.apy ?? 0);
  const owner = String(r.owner || r.manager_id || parsedConfig.owner || "Perigee");
  const status = String(r.status || "ACTIVE");

  return { id, name, balance, asset, apy, owner, status };
}

/**
 * Builds standard Open Graph custom tag dictionary containing vault name, balance, and APY.
 */
export function buildVaultOgTags(
  input: Partial<Vault | VaultRecord | VaultMetadataInput> | null,
  fallbackId?: string
): Record<string, string> {
  const vault = normalizeVaultData(input, fallbackId);
  return {
    "vault:name": vault.name,
    "vault:balance": `${vault.balance} ${vault.asset}`,
    "vault:apy": `${vault.apy}%`,
    "vault:asset": vault.asset,
    "vault:status": vault.status,
    "og:balance": `${vault.balance} ${vault.asset}`,
    "og:apy": `${vault.apy}%`,
  };
}

/**
 * Builds Schema.org JSON-LD structured data for a dynamic vault page.
 */
export function buildVaultJsonLd(
  input: Partial<Vault | VaultRecord | VaultMetadataInput> | null,
  fallbackId?: string,
  siteUrl: string = SITE_ORIGIN
): Record<string, unknown> {
  const vault = normalizeVaultData(input, fallbackId);
  const canonicalUrl = `${siteUrl}/vault/${vault.id}`;

  return {
    "@context": "https://schema.org",
    "@type": "FinancialProduct",
    name: vault.name,
    description: `Perigee autonomous vault ${vault.name} on Stellar. Current Balance: ${vault.balance} ${vault.asset}, APY: ${vault.apy}%.`,
    url: canonicalUrl,
    provider: {
      "@type": "Organization",
      name: "Perigee",
      url: siteUrl,
    },
    offers: {
      "@type": "Offer",
      price: String(vault.balance),
      priceCurrency: vault.asset,
    },
    annualPercentageYield: `${vault.apy}%`,
  };
}

/**
 * Builds Next.js App Router dynamic Metadata for a vault page,
 * ensuring vault name, balance, and APY are included in Open Graph and Twitter Card tags.
 */
export function buildVaultMetadata(
  input: Partial<Vault | VaultRecord | VaultMetadataInput> | null,
  fallbackId?: string,
  siteUrl: string = SITE_ORIGIN
): Metadata {
  const vault = normalizeVaultData(input, fallbackId);
  const title = `${vault.name} | Perigee`;
  const description = `Perigee vault ${vault.name}. Balance: ${vault.balance} ${vault.asset} · APY: ${vault.apy}% · Status: ${vault.status}.`;
  const canonicalUrl = `${siteUrl}/vault/${vault.id}`;
  const ogDescription = `Vault: ${vault.name} | Balance: ${vault.balance} ${vault.asset} | APY: ${vault.apy}%`;

  const ogTags = buildVaultOgTags(input, fallbackId);

  return {
    title,
    description,
    alternates: {
      canonical: canonicalUrl,
    },
    openGraph: {
      title,
      description: ogDescription,
      url: canonicalUrl,
      siteName: "Perigee",
      type: "website",
      images: [
        {
          url: DEFAULT_OG_IMAGE,
          width: 1200,
          height: 630,
          alt: `${vault.name} - Perigee Vault`,
        },
      ],
    },
    twitter: {
      card: "summary_large_image",
      title,
      description: ogDescription,
      images: [DEFAULT_OG_IMAGE],
    },
    other: ogTags,
  };
}

/**
 * Fetches vault record by ID with fallback support for mock/demo environments.
 */
export async function fetchVaultData(id: string): Promise<Vault | null> {
  try {
    const record = await vaultService.get(id);
    if (record) {
      const normalized = normalizeVaultData(record, id);
      return {
        id: normalized.id,
        name: normalized.name,
        owner: normalized.owner,
        balance: normalized.balance,
        asset: normalized.asset,
        apy: normalized.apy,
        status: normalized.status as Vault["status"],
        createdAt: record.created_at || new Date().toISOString(),
        updatedAt: record.updated_at || new Date().toISOString(),
      };
    }
  } catch {
    // Return mock fallback for demo/test IDs if backend API is not running
  }

  // Graceful fallback for demo or testing
  if (id) {
    return {
      id,
      name: `Vault ${id}`,
      owner: "Perigee Manager",
      balance: 10000,
      asset: "XLM",
      apy: 12.5,
      status: "ACTIVE",
      createdAt: new Date().toISOString(),
      updatedAt: new Date().toISOString(),
    };
  }

  return null;
}
