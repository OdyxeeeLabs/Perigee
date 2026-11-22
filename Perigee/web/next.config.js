/** @type {import("next").NextConfig} */
const devProxy = require("./dev.proxy.config.js");
const devProxyConfig = require("./dev.proxy.json");

const nextConfig = {
  reactStrictMode: true,
  transpilePackages: ["@creit.tech/stellar-wallets-kit"],
  turbopack: {},

  images: {
    remotePatterns: [
      {
        protocol: "https",
        hostname: "stellar.creit.tech",
      },
    ],
  },
};

module.exports = devProxy(nextConfig, devProxyConfig);
