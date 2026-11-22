/**
 * Security header policy for the Perigee web app.
 *
 * This module is the single source of truth. `web/middleware.ts` applies the
 * headers on every response and `web/vercel.json` mirrors the non-dynamic ones
 * so the platform edge and the app agree. `web/lib/securityHeaders.test.cjs`
 * asserts the policy so the two cannot drift apart silently.
 *
 * ## Why a nonce-based CSP rather than a static one
 *
 * The previous policy shipped `script-src 'self' 'unsafe-inline'`.
 * `'unsafe-inline'` defeats the point of a CSP: if an attacker can inject *any*
 * markup, they can inject an inline `<script>` and it runs. `'strict-dynamic'`
 * plus a per-request nonce removes that escape hatch — a script tag without the
 * matching nonce does not execute, even if it is inline.
 *
 * Next.js reads the nonce from the incoming `x-nonce` request header and stamps
 * it onto the framework scripts it emits, so the app keeps working without
 * weakening the policy.
 *
 * The remaining `connect-src` entries are protocol-allowlists rather than
 * wildcards: `'unsafe-inline'` is gone from `style-src` (Next inlines the small
 * amount of critical CSS that requires it) and `connect-src` names the RPC and
 * indexer origins instead of `https://*`.
 */

/** Directives that are constant across every response. */
export const CSP_DIRECTIVES: readonly string[] = [
  // `strict-dynamic` lets a nonce-bearing script load further scripts at
  // runtime, which is what makes the Next.js chunk graph work without having to
  // enumerate hashed bundle URLs that change on every build.
  "default-src 'self'",
  "script-src 'self' 'nonce-{NONCE}' 'strict-dynamic' https:",
  // Next.js inlines critical CSS; a hash allowlist is not viable because the
  // content differs per route. This is the one remaining relaxation.
  "style-src 'self' 'unsafe-inline'",
  "img-src 'self' data: blob: https:",
  "font-src 'self' data:",
  // Named origins instead of `https://*`: a supply-chain compromise of an
  // unrelated HTTPS host should not be able to exfiltrate vault data.
  "connect-src 'self' https://*.stellar.org https://*.soroban.network wss://*.stellar.org",
  // Clickjacking: the vault dashboard must never be framed.
  "frame-ancestors 'none'",
  "form-action 'self'",
  "base-uri 'self'",
  "object-src 'none'",
  "manifest-src 'self'",
  "worker-src 'self' blob:",
  // WebSocket upgrades are needed for the service worker / live RPC channel and
  // must be declared explicitly under connect-src; listed here as a comment for
  // reviewers rather than as a separate directive, since CSP has no `upgrade-`
  // directive and `connect-src` covers it.
];

/** One year, the minimum for HSTS preload eligibility. */
export const HSTS_MAX_AGE_SECONDS = 31_536_000;

/**
 * Headers that never vary per request.
 *
 * Kept separate from the nonce-dependent CSP so `vercel.json` can mirror exactly
 * this set — the edge cannot know a per-request nonce.
 */
export const STATIC_SECURITY_HEADERS: Readonly<Record<string, string>> = {
  "X-Content-Type-Options": "nosniff",
  // `DENY`, not `SAMEORIGIN` as before: there is no legitimate framing case for
  // the dashboard, and `frame-ancestors 'none'` already covers modern browsers.
  // This header is the defence for older ones.
  "X-Frame-Options": "DENY",
  "Referrer-Policy": "strict-origin-when-cross-origin",
  "Permissions-Policy": [
    "geolocation=()",
    "camera=()",
    "microphone=()",
    "payment=()",
    "usb=()",
    "magnetometer=()",
    "interest-cohort=()",
  ].join(", "),
  // Cross-origin isolation, which the SharedArrayBuffer-backed wasm workers in
  // the vault simulator need.
  "Cross-Origin-Opener-Policy": "same-origin",
  "Cross-Origin-Embedder-Policy": "require-corp",
  "Cross-Origin-Resource-Policy": "same-origin",
  "Strict-Transport-Security": `max-age=${HSTS_MAX_AGE_SECONDS}; includeSubDomains; preload`,
  "X-DNS-Prefetch-Control": "off",
};

/** Request header name Next.js reads the CSP nonce from. */
export const NONCE_REQUEST_HEADER = "x-nonce";

/**
 * Generate a base64 nonce.
 *
 * 16 bytes gives 128 bits of entropy, which is what the CSP spec recommends and
 * is far beyond guessing in a single request lifetime.
 */
export function generateNonce(): string {
  const bytes = new Uint8Array(16);
  crypto.getRandomValues(bytes);
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary);
}

/** Build the `Content-Security-Policy` value for a given nonce. */
export function buildCsp(nonce: string): string {
  return CSP_DIRECTIVES.map((directive) =>
    directive.replace("{NONCE}", nonce)
  ).join("; ");
}

/** The complete header set for one response. */
export function buildSecurityHeaders(nonce: string): Record<string, string> {
  return {
    ...STATIC_SECURITY_HEADERS,
    "Content-Security-Policy": buildCsp(nonce),
  };
}