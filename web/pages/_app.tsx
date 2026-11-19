import type { AppProps } from "next/app";
import "../styles/globals.css";
import "../styles/print.css";
import { Inter } from "next/font/google";
import { WalletProvider } from "../context/WalletContext";
import { ToastProvider } from "../context/ToastContext";
import { ErrorBoundary } from "../components/ErrorBoundary";
import { Analytics } from "../components/Analytics";
import { ServiceWorkerManager } from "../components/ServiceWorkerManager";
import { NetworkStatusBanner } from "../components/NetworkStatusBanner";
import { RpcFallbackBanner } from "../components/RpcFallbackBanner";
import { NextIntlClientProvider } from "next-intl";
import defaultMessages from "../messages/en.json";
import { API_URL } from "../lib/api";
import { useRouter } from "next/router";
import { useEffect } from "react";
import { FeatureFlagProvider } from "../features/feature-flags";
// WEB-15 (#469): enable `reducedMotion="user"` for Framer Motion across the
// whole Pages Router tree. Every `motion.*` animation now respects the OS
// `prefers-reduced-motion` setting, in addition to the per-component
// `useReducedMotion()` guards already in place.
import { MotionProvider } from "../components/MotionProvider";
// WEB (#700): locale/locale persistence. `next-intl` resolves the message
// catalogue; `LocaleSync` owns the document-level `lang`/`dir` attributes,
// which CSS logical properties and assistive technology both depend on.
import { LocaleSync } from "../components/LocaleSync";
// WEB-52 (#185): global command palette — ⌘K / Ctrl+K / `/` opens a searchable
// launcher for navigation, vaults, settings and quick actions.
import { CommandPaletteLauncher } from "../components/CommandPaletteLauncher";
// WCAG 2.4.1 (#696/#698): the skip link is the first focusable element and
// moves focus into <main id="main-content">.
import { SkipLink } from "../components/common/SkipLink/SkipLink";
// Global live-region announcer with a persisted verbosity setting (#697).
import { AnnouncerProvider } from "../components/AnnouncerProvider";

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
 *
 * Issue #705: **`@axe-core/react` is not in package.json or yarn.lock.** The
 * `import()` below therefore rejects on every dev boot, and the `.catch`
 * swallowed it — so the integration was silently dead code and looked like
 * working tooling. That is worse than having none: it made the codebase appear
 * to have automated a11y coverage that did not exist.
 *
 * The failure is now reported instead of ignored, once per session. The
 * dependency itself is a package.json + yarn.lock change, which is out of
 * scope here; `yarn add -D @axe-core/react` is the fix, and the
 * `declared-tooling` job in .github/workflows/a11y-audit.yml fails the build
 * if the import and the manifest disagree again.
 */
let axeWarningShown = false;

async function initAxe() {
  if (typeof window === "undefined" || process.env.NODE_ENV !== "development") return;

  try {
    const axe = await import("@axe-core/react");
    const React = await import("react");
    const ReactDOM = await import("react-dom");
    // 1000 ms debounce — avoids flooding the console on rapid re-renders
    await axe.default(React.default, ReactDOM.default, 1000);
  } catch (error) {
    if (axeWarningShown) return;
    axeWarningShown = true;
    console.warn(
      "[a11y] @axe-core/react is not installed, so the browser console audit " +
        "did not run. This is not a runtime problem: install it with " +
        "`yarn add -D @axe-core/react` for dev-time violations. " +
        "Dependency-free checks run `node scripts/a11y/static-audit.mjs`.",
      error,
    );
  }
}

export default function App({ Component, pageProps }: AppProps) {
  useEffect(() => {
    // `initAxe` reports its own failure once; a second silent catch here would
    // hide the very thing #705 exists to surface.
    initAxe().catch(() => {});
  }, []);

  const router = useRouter();
  const route = router.asPath || "/";

  return (
    <div className={inter.className}>
      <MotionProvider>
        <LocaleSync>
          <FeatureFlagProvider>
            <NextIntlClientProvider
              locale={router?.locale ?? "en"}
              messages={pageProps.messages ?? defaultMessages}
              timeZone="UTC"
            >
              <ErrorBoundary
                variant="root"
                context={{
                  route,
                  component: "AppShell",
                }}
                title="We hit an unexpected error"
                description="The application encountered a critical rendering issue. You can retry without losing your session."
              >
                <WalletProvider>
                  <ToastProvider>
                    <AnnouncerProvider>
                      <SkipLink />
                      {/* Network status and API availability (#109) */}
                      <NetworkStatusBanner apiUrl={API_URL} />

                      {/*
                       * Graceful RPC fallback — shown when the backend is unreachable (#115).
                       * Wraps ErrorBoundary so children can read `useRpcFallback()` to display
                       * stale-data badges on individual views.
                       */}
                      <RpcFallbackBanner apiUrl={API_URL}>
                        <ErrorBoundary
                          variant="route"
                          context={{
                            route,
                            component: Component.displayName ?? "RouteBoundary",
                          }}
                          title="This page failed to load"
                          description="This page hit an unexpected rendering error. Try again or reload to recover the page state."
                        >
                          {/* tabIndex={-1} so the skip link can move focus here (#696). */}
                          <main id="main-content" tabIndex={-1}>
                            <Component {...pageProps} />
                          </main>
                          <CommandPaletteLauncher />
                          <Analytics />
                          <ServiceWorkerManager />
                        </ErrorBoundary>
                      </RpcFallbackBanner>
                    </AnnouncerProvider>
                  </ToastProvider>
                </WalletProvider>
              </ErrorBoundary>
            </NextIntlClientProvider>
          </FeatureFlagProvider>
        </LocaleSync>
      </MotionProvider>
    </div>
  );
}

// Core Web Vitals (#693): Next.js invokes this for every reported metric.
export { reportWebVitals } from "../lib/webVitals";
