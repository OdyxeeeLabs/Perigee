# Response compression (#695)

## What the app does

`next.config.js` sets `compress: true`, so the Next.js server gzips
compressible responses (HTML, JS, CSS, JSON, SVG, fonts). Next's built-in
middleware does **not** support Brotli, and it already skips responses that are
not compressible.

## Brotli at the edge

Brotli (q=11 for static assets, a lower level for dynamic responses) and gzip
(q=6) belong on the reverse proxy / CDN in front of Next. Example nginx:

```nginx
brotli on;
brotli_comp_level 11;          # static assets; use 5-6 for proxied dynamic responses
brotli_types text/html text/css application/javascript application/json
             image/svg+xml font/ttf font/otf application/xml;

gzip on;
gzip_comp_level 6;
gzip_vary on;
gzip_types text/html text/css application/javascript application/json
           image/svg+xml font/ttf font/otf application/xml;
# Images, video, woff/woff2 and wasm are already compressed: they are not listed.
```

If a CDN (Vercel, Cloudflare) is used, it negotiates Brotli automatically; set
`compress: false` only when the proxy compresses and you want to avoid double work.

## Verifying

```bash
node scripts/check-compression.mjs http://localhost:3000 / /api/vitals
```

The script requests each path with `Accept-Encoding: br, gzip`, fails when a
compressible response is not encoded (or an already-compressed type is), and
prints the compression ratio so it can be tracked over time.
