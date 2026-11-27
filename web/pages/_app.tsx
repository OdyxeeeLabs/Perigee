import type { AppProps } from "next/app";
import "../styles/globals.css";
import "../styles/print.css";
import { WalletProvider } from "../context/WalletContext";
import { ToastProvider } from "../context/ToastContext";
import { ErrorBoundary } from "../components/ErrorBoundary";
import { Analytics } from "../components/Analytics";
import { ServiceWorkerManager } from "../components/ServiceWorkerManager";
import { NetworkStatusBanner } from "../components/NetworkStatusBanner";
import { RpcFallbackBanner } from "../components/RpcFallbackBanner";
import { NextIntlClientProvider } from "next-intl";
import enMessages from "../messages/en.json";

const defaultMessages: Record<string, string> =
  (enMessages as { default?: Record<string, string> }).default ??
  (enMessages as unknown as Record<string, string>);
import { API_URL } from "../lib/api";
import { useRouter } from "next/router";
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

export default function App({ Component, pageProps }: AppProps) {
  const router = useRouter();
  const route = router.asPath || "/";

  return (
    <div className="font-sans">
      <MotionProvider>
        <LocaleSync>
          <FeatureFlagProvider>
            <NextIntlClientProvider
              locale={router?.locale ?? "en"}
              messages={{ ...defaultMessages, ...(pageProps.messages ?? {}) }}
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
