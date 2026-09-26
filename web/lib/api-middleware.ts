/**
 * api-middleware.ts
 *
 * Pages-router handler wrappers that every `pages/api/*` route MUST compose
 * into its default export to satisfy #WEB-14 / issue #100:
 *   • `withRateLimit`  — per-client token-bucket throttling (default 100 req / 60 s)
 *   • `validateBody`   — type-guard based body validation that returns 400 on
 *                        injection-shaped payloads before any handler runs
 *
 * These helpers wrap a NextApiHandler and return a new NextApiHandler — they
 * never mutate the original — so a route is composed like:
 *
 *   // pages/api/example.ts
 *   import { withRateLimit, validateBody } from "@/lib/api-middleware";
 *   import type { NextApiRequest, NextApiResponse } from "next";
 *
 *   type CreateVaultBody = { name: string };
 *   const isCreateVaultBody = (b: unknown): b is CreateVaultBody =>
 *     !!b && typeof b === "object" &&
 *     "name" in (b as Record<string, unknown>) &&
 *     typeof (b as Record<string, unknown>).name === "string" &&
 *     (b as Record<string, unknown>).name !== "";
 *
 *   async function handler(req: NextApiRequest, res: NextApiResponse) {
 *     // body is now guaranteed shape-safe
 *     res.status(200).json({ ok: true });
 *   }
 *   export default withRateLimit(validateBody(handler, isCreateVaultBody));
 *
 * The store is in-process (Map) — sufficient for the per-key throttle used
 * here. For multi-instance horizontal scaling, swap for a Redis-backed
 * implementation; the API surface stays the same.
 */
import type { NextApiHandler, NextApiRequest, NextApiResponse } from "next";

/* ── Rate limiter ──────────────────────────────────────────────────────── */

interface RateLimitOptions {
  /** Maximum number of requests within `windowMs`. Default 100. */
  max?: number;
  /** Sliding window size in milliseconds. Default 60_000 (1 min). */
  windowMs?: number;
  /**
   * Strategy for deriving the rate-limit bucket key. Defaults to the client
   * IP. Switch to a user-id key (after authentication) to make the limit
   * per-user instead of per-IP.
   */
  keyFn?: (req: NextApiRequest) => string;
}

interface RateLimitBucket {
  count: number;
  /** Wall-clock time (ms) when the bucket will reset. */
  resetAt: number;
}

const rateLimitBuckets = new Map<string, RateLimitBucket>();

function defaultKeyFn(req: NextApiRequest): string {
  // x-forwarded-for is set by Vercel / most reverse proxies.
  const xff = req.headers["x-forwarded-for"];
  if (typeof xff === "string" && xff.length > 0) {
    return xff.split(",")[0]!.trim();
  }
  if (Array.isArray(xff) && xff.length > 0 && typeof xff[0] === "string") {
    return xff[0].split(",")[0]!.trim();
  }
  return req.socket?.remoteAddress ?? "unknown";
}

/**
 * Wrap a Pages-Router handler with an in-memory token-bucket rate limiter
 * keyed by client IP (override with `options.keyFn` for a per-user key).
 *
 * Returns HTTP 429 with a `Retry-After` header once the bucket is exhausted.
 */
export function withRateLimit(
  handler: NextApiHandler,
  options: RateLimitOptions = {},
): NextApiHandler {
  const max = options.max ?? 100;
  const windowMs = options.windowMs ?? 60_000;
  const keyFn = options.keyFn ?? defaultKeyFn;

  return async function rateLimitedHandler(
    req: NextApiRequest,
    res: NextApiResponse,
  ) {
    const key = keyFn(req);
    const now = Date.now();

    const bucket = rateLimitBuckets.get(key);
    if (!bucket || bucket.resetAt <= now) {
      rateLimitBuckets.set(key, { count: 1, resetAt: now + windowMs });
      return handler(req, res);
    }

    if (bucket.count >= max) {
      const retryAfter = Math.max(1, Math.ceil((bucket.resetAt - now) / 1000));
      res.setHeader("Retry-After", String(retryAfter));
      res.setHeader("X-RateLimit-Limit", String(max));
      res.setHeader("X-RateLimit-Remaining", "0");
      res.setHeader("X-RateLimit-Reset", String(Math.ceil(bucket.resetAt / 1000)));
      return res.status(429).json({
        error: "TOO_MANY_REQUESTS",
        message: `Rate limit exceeded. Try again in ${retryAfter}s.`,
      });
    }

    bucket.count += 1;
    res.setHeader("X-RateLimit-Limit", String(max));
    res.setHeader(
      "X-RateLimit-Remaining",
      String(Math.max(0, max - bucket.count)),
    );
    return handler(req, res);
  };
}

/* ── Body validator ────────────────────────────────────────────────────── */

/**
 * TypeScript user-defined type guard. Returning `true` here also narrows the
 * payload to `T` inside the wrapped handler — this is the only place
 * `unknown` is asserted into a concrete shape.
 */
export type BodyValidator<T> = (body: unknown) => body is T;

/** Default rejection for objects that are not plain JSON. */
const isPlainJsonObject = (b: unknown): b is Record<string, unknown> =>
  typeof b === "object" && b !== null && !Array.isArray(b);

/**
 * Wrap a handler so POST / PUT / PATCH bodies are validated against a
 * user-supplied type guard before reaching the handler.

 * Returns 400 for:
 *   • non-object bodies (`null`, arrays, primitives)
 *   • bodies that fail the supplied `BodyValidator`
 *
 * GET / DELETE / HEAD are forwarded unchanged — these helpers are
 * specifically about hardening bodies, not adding auth.
 */
export function validateBody<T>(
  handler: NextApiHandler,
  isValid: BodyValidator<T>,
): NextApiHandler {
  return async function validatedHandler(
    req: NextApiRequest,
    res: NextApiResponse,
  ) {
    if (
      req.method !== "POST" &&
      req.method !== "PUT" &&
      req.method !== "PATCH"
    ) {
      return handler(req, res);
    }

    if (!isPlainJsonObject(req.body)) {
      return res.status(400).json({
        error: "BAD_REQUEST",
        message: "Request body must be a JSON object.",
      });
    }

    if (!isValid(req.body)) {
      return res.status(400).json({
        error: "BAD_REQUEST",
        message: "Request body failed validation.",
      });
    }

    return handler(req, res);
  };
}

/** Test-only: clear the in-memory rate-limit bucket map. */
export function __resetRateLimitBucketsForTests(): void {
  rateLimitBuckets.clear();
}
