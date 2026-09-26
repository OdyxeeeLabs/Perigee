import type { AppProps } from "next/app";
import "../styles/globals.css";
import "../styles/print.css";
import { Inter } from "next/font/google";
import { WalletProvider } from "../context/WalletContext";
import { ErrorBoundary } from "../components/ErrorBoundary";
import { Analytics } from "../components/Analytics";
import { NetworkStatusBanner } from "../components/NetworkStatusBanner";
import { RpcFallbackBanner } from "../components/RpcFallbackBanner";
import { NextIntlClientProvider } from "next-intl";
import { API_URL } from "../lib/api";
import { useRouter } from "next/router";
import { useEffect } from "react";
import { FeatureFlagProvider } from "../features/feature-flags";
// WEB-15 (#469): enable `reducedMotion="user"` for Framer Motion across the
// whole Pages Router tree. Every `motion.*` animation now respects the OS
// `prefers-reduced-motion` setting, in addition to the per-component
// `useReducedMotion()` guards already in place.
import { MotionProvider } from "../components/MotionProvider";

// WEB-54 (#187): self-host the Inter typeface through `next/font`. Font files
// are downloaded and preloaded at build time, eliminating FOIT and render
// blocking from an external font stylesheet. `inter.className` applies the
// generated font-family to every Pages Router route.
const inter = Inter({
  subsets: ["latin"],
  display: "swap",
});

/**
 * Initialize @axe-core/react in development so accessibility violations
 * are reported to the browser console during local development.
 *
 * The import is dynamic so axe-core is never bundled into production builds.
 * Resolves WEB-24 (#110): missing accessibility audit tooling.
 */
async function initAxe() {
  if (typeof window !== "undefined" && process.env.NODE_ENV === "development") {
    const axe = await import("@axe-core/react");
    const React = await import("react");
    const ReactDOM = await import("react-dom");
    // 1000 ms debounce — avoids flooding the console on rapid re-renders
    await axe.default(React.default, ReactDOM.default, 1000);
  }
}

export default function App({ Component, pageProps }: AppProps) {
  useEffect(() => {
    initAxe().catch(() => {
      // axe-core is a dev-only optional dependency; swallow import errors
      // in environments where it is not installed.
    });
  }, []);

  const router = useRouter();

  return (
    <div className={inter.className}>
      <MotionProvider>
        <FeatureFlagProvider>
          <NextIntlClientProvider
            locale={router?.locale ?? "en"}
            messages={pageProps.messages ?? {}}
            timeZone="UTC"
          >
            <WalletProvider>
              {/* Network status and API availability (#109) */}
              <NetworkStatusBanner apiUrl={API_URL} />
              {/*
               * Graceful RPC fallback — shown when the backend is unreachable (#115).
               * Wraps ErrorBoundary so children can read `useRpcFallback()` to display
               * stale-data badges on individual views.
               */}
              <RpcFallbackBanner apiUrl={API_URL}>
                <ErrorBoundary>
                  <Component {...pageProps} />
                  <Analytics />
                </ErrorBoundary>
              </RpcFallbackBanner>
            </WalletProvider>
          </NextIntlClientProvider>
        </FeatureFlagProvider>
      </MotionProvider>
    </div>
  );
}
