/**
 * config.ts
 *
 * Single source of truth for app-level contact / support links.
 *
 * Resolves WEB-53 (#186): contact/support links were hardcoded, so
 * white-label managers could not customize support URLs. All consumer-facing
 * links now flow through this module and are sourced from NEXT_PUBLIC_*
 * environment variables with Perigee defaults.
 *
 * White-label deployments: set these variables per deployment (Vercel env
 * vars, .env.local for local dev) to rebrand every support touchpoint at
 * once — no code changes required:
 *
 *   NEXT_PUBLIC_SUPPORT_URL    help center / support portal
 *   NEXT_PUBLIC_SUPPORT_EMAIL  contact e-mail address
 *   NEXT_PUBLIC_DOCS_URL       documentation site
 *   NEXT_PUBLIC_STATUS_URL     system status page
 *
 * Usage:
 *   import { supportLinks, supportMailto } from "@/lib/config";
 *   const href = supportLinks.supportUrl;
 *   <a href={supportMailto("Simulation failed")}>Contact support</a>
 */

export interface SupportLinks {
  /** Help center / support portal (shown on errors and outages). */
  supportUrl: string;
  /** Contact e-mail address (rendered as a mailto: link). */
  contactEmail: string;
  /** Documentation site (shown next to developer-facing errors). */
  docsUrl: string;
  /** System status page (shown during backend/RPC outages). */
  statusUrl: string;
}

// ---------------------------------------------------------------------------
// Read from environment variables
// ---------------------------------------------------------------------------

function env(key: string): string | undefined {
  return process.env[key]?.trim() || undefined;
}

/**
 * Resolve an optional env override against a mandatory default.
 * Empty/whitespace values fall back to the default so a blank var in a
 * white-label deploy never produces a dead link.
 */
export function resolveLink(override: string | undefined, fallback: string): string {
  return override ?? fallback;
}

const DEFAULT_SUPPORT_URL = "https://perigee.app/support";
const DEFAULT_SUPPORT_EMAIL = "support@perigee.app";
const DEFAULT_DOCS_URL = "https://perigee.app/docs";
const DEFAULT_STATUS_URL = "https://status.perigee.app";

/**
 * Contact/support links resolved from NEXT_PUBLIC_* env vars.
 * White-label managers customize them by setting the env vars above.
 */
export const supportLinks: Readonly<SupportLinks> = Object.freeze({
  supportUrl: resolveLink(env("NEXT_PUBLIC_SUPPORT_URL"), DEFAULT_SUPPORT_URL),
  contactEmail: resolveLink(env("NEXT_PUBLIC_SUPPORT_EMAIL"), DEFAULT_SUPPORT_EMAIL),
  docsUrl: resolveLink(env("NEXT_PUBLIC_DOCS_URL"), DEFAULT_DOCS_URL),
  statusUrl: resolveLink(env("NEXT_PUBLIC_STATUS_URL"), DEFAULT_STATUS_URL),
});

export interface PrivacyNoticeConfig {
  enabled: boolean;
  privacyPolicyUrl: string;
  telemetryDocsUrl: string;
  legalNote: string;
}

function envFlag(key: string, defaultValue: boolean): boolean {
  const value = env(key)?.toLowerCase();
  if (value === "true" || value === "1") return true;
  if (value === "false" || value === "0") return false;
  return defaultValue;
}

export const privacyNotice: Readonly<PrivacyNoticeConfig> = Object.freeze({
  enabled: envFlag("NEXT_PUBLIC_PRIVACY_NOTICE_ENABLED", true),
  privacyPolicyUrl: resolveLink(
    env("NEXT_PUBLIC_PRIVACY_POLICY_URL"),
    `${DEFAULT_DOCS_URL}/privacy`,
  ),
  telemetryDocsUrl: resolveLink(
    env("NEXT_PUBLIC_TELEMETRY_DOCS_URL"),
    `${DEFAULT_DOCS_URL}/telemetry`,
  ),
  legalNote: env("NEXT_PUBLIC_PRIVACY_LEGAL_NOTE") ?? "",
});

/** Escape a value for safe inclusion in a mailto: URI query component. */
function encodeMailtoPart(value: string): string {
  return encodeURIComponent(value);
}

/**
 * Build a prefilled mailto: link to the configured contact address.
 *
 * @param subject optional subject line (URI-encoded here)
 * @param body    optional body text (URI-encoded here)
 */
export function supportMailto(subject?: string, body?: string): string {
  const params: string[] = [];
  if (subject) params.push(`subject=${encodeMailtoPart(subject)}`);
  if (body) params.push(`body=${encodeMailtoPart(body)}`);
  const query = params.length > 0 ? `?${params.join("&")}` : "";
  return `mailto:${supportLinks.contactEmail}${query}`;
}
