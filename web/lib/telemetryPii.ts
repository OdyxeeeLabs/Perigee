/**
 * Privacy-first PII scrubbing and redaction engine for Perigee telemetry.
 *
 * Reduces exposure to Personally Identifiable Information (PII) by:
 *  - Strips keys containing forbidden words (email, address, name, ip, phone, ssn, secret, etc.)
 *  - Redacts sensitive string values matching Stellar public keys (G...), Soroban contract IDs (C...),
 *    secret keys (S...), email patterns, IPv4/IPv6 addresses, and 64-character hex secrets.
 *  - Traverses nested objects and arrays recursively.
 *
 * Deployment-specific compliance claims are intentionally not made here.
 */

// Keys that must NEVER appear in telemetry payloads
const FORBIDDEN_KEY_SUBSTRINGS = [
  "email",
  "address",
  "name",
  "ip",
  "phone",
  "ssn",
  "password",
  "secret",
  "token",
  "credential",
  "auth",
  "cookie",
  "signature",
  "privkey",
  "pubkey",
  "account_id",
  "public_key",
];

// Regex patterns for sensitive values
const EMAIL_REGEX = /\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}\b/g;
const IPV4_REGEX = /\b(?:\d{1,3}\.){3}\d{1,3}\b/g;
const IPV6_REGEX = /\b(?:[A-Fa-f0-9]{1,4}:){7}[A-Fa-f0-9]{1,4}\b/g;
// Stellar public key: G followed by 55 alphanumeric characters
const STELLAR_ADDRESS_REGEX = /\bG[A-Z0-9]{55}\b/g;
// Soroban contract address: C followed by 55 alphanumeric characters
const CONTRACT_ADDRESS_REGEX = /\bC[A-Z0-9]{55}\b/g;
// Stellar secret seed: S followed by 55 alphanumeric characters
const STELLAR_SECRET_REGEX = /\bS[A-Z0-9]{55}\b/g;
// 64-character hex private key / hash
const HEX_KEY_REGEX = /\b[0-9a-fA-F]{64}\b/g;

/**
 * Checks whether a property key is forbidden due to PII potential.
 */
export function isForbiddenKey(key: string): boolean {
  const lower = key.toLowerCase();
  return FORBIDDEN_KEY_SUBSTRINGS.some((forbidden) => lower.includes(forbidden));
}

/**
 * Redacts known sensitive patterns (emails, IPs, Stellar addresses, secret keys)
 * from a string value.
 */
export function sanitizeStringValue(value: string): string {
  let sanitized = value;
  sanitized = sanitized.replace(EMAIL_REGEX, "[REDACTED_EMAIL]");
  sanitized = sanitized.replace(IPV4_REGEX, "[REDACTED_IP]");
  sanitized = sanitized.replace(IPV6_REGEX, "[REDACTED_IP]");
  sanitized = sanitized.replace(STELLAR_ADDRESS_REGEX, "[REDACTED_STELLAR_ADDRESS]");
  sanitized = sanitized.replace(CONTRACT_ADDRESS_REGEX, "[REDACTED_CONTRACT_ADDRESS]");
  sanitized = sanitized.replace(STELLAR_SECRET_REGEX, "[REDACTED_SECRET]");
  sanitized = sanitized.replace(HEX_KEY_REGEX, "[REDACTED_HEX_KEY]");
  return sanitized;
}

/**
 * Recursively scrubs and sanitizes properties of an event.
 * Strips disallowed keys and redacts sensitive patterns in string values.
 * Returns only safe primitive values (string, number, boolean).
 */
export function sanitizeTelemetryProperties(
  properties?: Record<string, unknown>,
): Record<string, string | number | boolean> {
  if (!properties || typeof properties !== "object") return {};

  const clean: Record<string, string | number | boolean> = {};

  for (const [key, val] of Object.entries(properties)) {
    // 1. Strip forbidden keys
    if (isForbiddenKey(key)) {
      continue;
    }

    // 2. Sanitize and retain valid primitive values
    if (typeof val === "string") {
      clean[key] = sanitizeStringValue(val);
    } else if (typeof val === "number") {
      if (Number.isFinite(val)) {
        clean[key] = val;
      }
    } else if (typeof val === "boolean") {
      clean[key] = val;
    } else if (val !== null && typeof val === "object") {
      // Flatten nested objects safely with dot notation
      const nested = sanitizeTelemetryProperties(val as Record<string, unknown>);
      for (const [nestedKey, nestedVal] of Object.entries(nested)) {
        clean[`${key}_${nestedKey}`] = nestedVal;
      }
    }
  }

  return clean;
}
