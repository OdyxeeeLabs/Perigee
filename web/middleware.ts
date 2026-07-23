import { NextResponse } from "next/server";
import type { NextRequest } from "next/server";

const csp = [
  "default-src 'self'",
  "script-src 'self' 'unsafe-inline'",
  "style-src 'self' 'unsafe-inline'",
  "img-src 'self' data:",
  "font-src 'self' data:",
  "connect-src 'self' https://*",
  "frame-ancestors 'self'",
  "form-action 'self'",
].join("; ");

const securityHeaders: Record<string, string> = {
  "Content-Security-Policy": csp,
  "X-Content-Type-Options": "nosniff",
  "Strict-Transport-Security": "max-age=63072000; includeSubDomains; preload",
  "X-Frame-Options": "SAMEORIGIN",
  "Referrer-Policy": "strict-origin-when-cross-origin",
  "X-DNS-Prefetch-Control": "off",
  "Permissions-Policy": "camera=(), microphone=(), geolocation=()",
};

export function middleware(_req: NextRequest) {
  return NextResponse.next({
    headers: securityHeaders,
  });
}

export const config = {
  matcher: ["/:path*"],
};
