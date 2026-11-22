import { NextResponse } from "next/server";
import type { NextRequest } from "next/server";

import {
  buildSecurityHeaders,
  generateNonce,
  NONCE_REQUEST_HEADER,
} from "./lib/securityHeaders";

/**
 * Attach a per-request CSP nonce and the full security header set.
 *
 * The nonce is generated here, echoed onto the *request* headers as `x-nonce`
 * (which is how Next.js learns to stamp it onto the framework scripts it
 * emits), and then used to build the `Content-Security-Policy` on the
 * *response*.
 *
 * Policy lives in `web/lib/securityHeaders.ts`; this file only wires it up.
 */
export function middleware(req: NextRequest) {
  const nonce = generateNonce();

  // Next.js reads the nonce off the request to apply it to its own <script>
  // tags. Setting it on the request headers is what makes strict-dynamic
  // workable without breaking the app.
  const requestHeaders = new Headers(req.headers);
  requestHeaders.set(NONCE_REQUEST_HEADER, nonce);
  // Strip anything a proxy might have injected, so a spoofed nonce from
  // upstream cannot be reflected back into the policy.
  requestHeaders.delete("x-forwarded-nonce");

  const response = NextResponse.next({
    request: { headers: requestHeaders },
    headers: buildSecurityHeaders(nonce),
  });

  // Do not advertise the framework.
  response.headers.delete("x-powered-by");

  return response;
}

export const config = {
  matcher: ["/:path*"],
};