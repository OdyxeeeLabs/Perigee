/** @type {import('next').NextConfig} */
const createNextIntlPlugin = require('next-intl/plugin');
const withNextIntl = createNextIntlPlugin('./lib/i18n.ts');

const nextConfig = {
  reactStrictMode: true,
  // Gzip responses from the Next server (issue #695). Brotli is applied by the
  // reverse proxy / CDN, see docs/compression.md.
  compress: true,
  transpilePackages: ["@creit.tech/stellar-wallets-kit"],
  turbopack: {},

  // Locale routing (issue #700). Without this block `router.locale` is
  // always undefined, `next-intl` always loads `messages/en.json`, and the
  // language switcher is decoration. The list must stay identical to
  // SUPPORTED_LOCALES in lib/locale/locales.ts — lib/locale/locales.test.cjs
  // asserts that it is. `localeDetection: false` keeps the server from
  // redirecting on Accept-Language, so an Arabic URL is never rewritten to
  // English for a crawler or a shared link; the switcher is the only thing
  // that changes the locale, and it persists the choice.
  i18n: {
    locales: ["en", "es", "zh", "ar", "hi", "pt", "fr", "de"],
    defaultLocale: "en",
    localeDetection: false,
  },

  images: {
    // Modern formats first; Next negotiates AVIF/WebP from the Accept header.
    formats: ["image/avif", "image/webp"],
    deviceSizes: [640, 750, 828, 1080, 1200, 1920],
    imageSizes: [16, 20, 32, 48, 64, 96, 128, 256],
    minimumCacheTTL: 60 * 60 * 24,
    remotePatterns: [
      {
        protocol: "https",
        hostname: "stellar.creit.tech",
      },
      // IPFS gateways (issue #689)
      { protocol: "https", hostname: "ipfs.io", pathname: "/ipfs/**" },
      { protocol: "https", hostname: "cloudflare-ipfs.com", pathname: "/ipfs/**" },
      { protocol: "https", hostname: "gateway.pinata.cloud", pathname: "/ipfs/**" },
      // Optional CDN host, e.g. NEXT_PUBLIC_IMAGE_CDN_HOST=cdn.example.com
      ...(process.env.NEXT_PUBLIC_IMAGE_CDN_HOST
        ? [{ protocol: "https", hostname: process.env.NEXT_PUBLIC_IMAGE_CDN_HOST }]
        : []),
    ],
  },

  // MIGRATION STATUS: Pages Router active
  // Explicit configuration to maintain Pages Router stability during migration.
  // Both pages/ and app/ directories can coexist during incremental migration.
  // Remove this comment and any Pages Router-specific config after full migration to App Router.
  // See MIGRATION.md for incremental migration plan.
  useFileSystemPublicRoutes: true,

  // Pin to current stable behaviour
  // Remove experimental flag when migrating to App Router
  experimental: {
    // App Router will be enabled incrementally
    // See MIGRATION.md for migration phases

    /**
     * Optimise CSS — removes unused Tailwind classes at build time via
     * the Lightning CSS minifier bundled with Next.js.
     *
     * Resolves #686 — Optimize bundle size.
     */
    optimizeCss: true,
  },

  /**
   * Webpack customisations for bundle-size reduction.
   *
   * Resolves #686 — Optimize bundle size.
   *
   * Changes:
   *  1. `mermaid` is a large library (~1.5 MB) used only in the result viewer.
   *     We mark it as an async chunk so it is loaded on-demand rather than
   *     included in the initial JS bundle.
   *  2. Moment.js (pulled in transitively) includes all locale data by default.
   *     We use `IgnorePlugin` to strip unused locales (~300 KB savings).
   *  3. Third-party packages that are already shipped as ES modules (e.g.
   *     framer-motion, lucide-react) are placed in a dedicated vendor chunk so
   *     they are cached independently and don't bust the app chunk on every
   *     deploy.
   */
  webpack(config, { isServer }) {
    // 1. Split mermaid into its own async chunk
    config.optimization = config.optimization ?? {};
    config.optimization.splitChunks = {
      ...(config.optimization.splitChunks ?? {}),
      cacheGroups: {
        ...(config.optimization.splitChunks?.cacheGroups ?? {}),
        mermaid: {
          test: /[\\/]node_modules[\\/]mermaid[\\/]/,
          name: "vendor-mermaid",
          chunks: "async",
          priority: 30,
          enforce: true,
        },
        // 3. Stable vendor chunk for large UI / animation libraries
        uiVendors: {
          test: /[\\/]node_modules[\\/](framer-motion|lucide-react|@tanstack)[\\/]/,
          name: "vendor-ui",
          chunks: "all",
          priority: 20,
          enforce: true,
        },
      },
    };

    // 2. Drop unused moment.js locales (no-op if moment is not in the graph)
    if (!isServer) {
      const webpack = require("webpack");
      config.plugins = config.plugins ?? [];
      config.plugins.push(
        new webpack.IgnorePlugin({
          resourceRegExp: /^\.\/locale$/,
          contextRegExp: /moment$/,
        }),
      );
    }

    return config;
  },

  async headers() {
    return [
      {
        // The worker script must always revalidate so updates are detected.
        source: "/sw.js",
        headers: [
          { key: "Cache-Control", value: "no-cache, no-store, must-revalidate" },
          { key: "Service-Worker-Allowed", value: "/" },
        ],
      },
      {
        source: "/:path*.wasm",
        headers: [
          {
            key: "Cache-Control",
            value: "public, max-age=31536000, immutable",
          },
        ],
      },
      {
        source: "/api/wasm/:path*",
        headers: [
          {
            key: "Cache-Control",
            value: "no-cache, no-store, must-revalidate",
          },
        ],
      },
    ];
  },
};

module.exports = withNextIntl(nextConfig);
