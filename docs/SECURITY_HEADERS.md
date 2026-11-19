# Security Headers

Closes [#624](https://github.com/OdyxeeeLabs/Perigee/issues/624).

The header policy lives in one place — [`web/lib/securityHeaders.ts`](../web/lib/securityHeaders.ts) —
and is applied by [`web/middleware.ts`](../web/middleware.ts) on every response.
[`web/vercel.json`](../web/vercel.json) mirrors the non-dynamic half so the
platform edge and the application agree.

## What changed

| Header | Before | After | Why |
| --- | --- | --- | --- |
| `Content-Security-Policy` | `script-src 'self' 'unsafe-inline'` | `script-src 'self' 'nonce-{per-request}' 'strict-dynamic' https:` | `'unsafe-inline'` means any injected markup can inject a script. The nonce removes that. |
| `frame-ancestors` | `'self'` | `'none'` | The vault dashboard must never be framed. |
| `X-Frame-Options` | `SAMEORIGIN` | `DENY` | Defence for browsers that do not implement `frame-ancestors`. |
| `Cross-Origin-Opener-Policy` | — | `same-origin` | Cross-origin isolation for the wasm workers. |
| `Cross-Origin-Embedder-Policy` | — | `require-corp` | Required by COOP for cross-origin isolation. |
| `Cross-Origin-Resource-Policy` | — | `same-origin` | Stops other origins loading our resources. |
| `Permissions-Policy` | camera/mic/geo | + `payment`, `usb`, `magnetometer`, `interest-cohort` | The app uses none of them; deny by default. |
| `connect-src` | `https://*` | named RPC/indexer origins | A supply-chain compromise of an unrelated HTTPS host could otherwise exfiltrate vault data. |
| `X-Powered-By` | emitted | deleted | Do not advertise the framework. |

Also added: `object-src 'none'`, `base-uri 'self'`, `manifest-src 'self'`,
`worker-src 'self' blob:`.

## The nonce, and why the app still works

Next.js reads a per-request nonce off the incoming `x-nonce` **request** header
and stamps it onto the framework `<script>` tags it emits. The middleware
generates a fresh 128-bit nonce per request, sets it on the request headers, and
then uses the same value to build the CSP on the response. No configuration
change is needed elsewhere and no build-time value is baked in.

The middleware also deletes `x-forwarded-nonce` from the incoming request before
setting its own. Without that, a proxy could inject a chosen nonce and have it
reflected straight back into the policy.

### Two deliberate relaxations

- **`style-src 'unsafe-inline'`** stays. Next inlines the small amount of
  critical CSS that must not wait for a stylesheet. A hash allowlist is not
  viable because the content differs per route. `style-src` does not permit
  script execution, so this is materially less dangerous than the `script-src`
  relaxation it replaced.
- **`script-src ... https:`** stays alongside `'strict-dynamic'`. Modern
  browsers that understand `strict-dynamic` ignore the host list; older ones
  need it. `object-src 'none'` limits what a bypassed script can do next.

## Verifying

Automated:

```bash
cd web && yarn test          # runs web/lib/securityHeaders.test.cjs
yarn typecheck
```

`securityHeaders.test.cjs` asserts the policy *and* reads `vercel.json` to prove
the edge config carries the same static headers. If someone changes one and not
the other, the test fails with the differing header named.

Against a running deployment:

```bash
curl -sSI https://<host>/ | grep -iE 'content-security|strict-transport|x-frame|permissions-policy|cross-origin|referrer'
# and/or
curl -sSI https://<host>/ | https://securityheaders.com  # or paste the headers into the UI
```

`securityheaders.com` should report green on every item except HSTS preload
submission, which requires the domain to be registered with the browser preload
list — an operational step, not a code change.

## Operational follow-ups

- Register the apex and `www` hosts with the HSTS preload list once this is on.
  `preload` in the header is only a request; the browser ignores it until the
  domain is submitted.
- Review the CSP report-only rollout: deploy
  `Content-Security-Policy-Report-Only` with a reporting endpoint for a week
  before switching to enforcing, so any third-party script the app still loads
  surfaces as a report rather than a breakage.