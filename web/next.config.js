/** @type {import('next').NextConfig} */
const nextConfig = {
  reactStrictMode: true,
  transpilePackages: ["@creit.tech/stellar-wallets-kit"],
  turbopack: {},

  // MIGRATION STATUS: Pages Router active
  // Explicit configuration to maintain Pages Router stability during migration.
  // Both pages/ and app/ directories can coexist during incremental migration.
  // Remove this comment and any Pages Router-specific config after full migration to App Router.
  // See MIGRATION.md for incremental migration plan.
  useFileSystemPublicRoutes: true,
};

module.exports = nextConfig;
