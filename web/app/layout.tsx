import type { Metadata } from 'next';
import '../styles/globals.css';
import { Inter } from 'next/font/google';
import { LocaleSync } from '@/components/LocaleSync';
import { NetworkStatusBanner } from '@/components/NetworkStatusBanner';
import { API_URL } from '@/lib/api';
import { ErrorBoundary } from '@/components/ErrorBoundary';
import { WalletProvider } from '@/context/WalletContext';
// WEB-15 (#469): mirror the Pages Router `MotionProvider` so the App Router
// routes also honour `prefers-reduced-motion` once migration completes.
import { MotionProvider } from '../components/MotionProvider';
// WEB-52 (#185): command palette. The App Router still serves no real pages
// (pages/ router is active), but keeping the launcher mounted here means the
// palette survives the migration without a second wiring pass.
import { CommandPaletteLauncher } from '../components/CommandPaletteLauncher';
import { SkipLink } from '../components/common/SkipLink/SkipLink';
import { AnnouncerProvider } from '../components/AnnouncerProvider';

// WEB-54 (#187): self-host the Inter typeface through `next/font` instead of
// loading it from an external stylesheet. Fonts are downloaded and preloaded
// at build time (no FOIT, no layout shift) and exposed as the `--font-inter`
// CSS variable consumed by the Tailwind `font-sans` stack.
const inter = Inter({
  subsets: ['latin'],
  variable: '--font-inter',
  display: 'swap',
});

export const metadata: Metadata = {
  title: 'Perigee',
  description: 'Autonomous portfolio agents for Stellar',
};

export default function RootLayout({ children }: { children: React.ReactNode }) {
  return (
    <html lang="en" className={inter.variable}>
      <body>
        <ErrorBoundary
          variant="root"
          context={{
            route: '/',
            component: 'RootLayout',
          }}
          title="The app encountered a critical error"
          description="The application crashed while rendering. You can retry or reload the page to recover."
        >
          <MotionProvider>
            <WalletProvider>
              <AnnouncerProvider>
                <SkipLink />
                <CommandPaletteLauncher />
                <NetworkStatusBanner apiUrl={API_URL} />
                <main id="main-content" tabIndex={-1}>
                  <LocaleSync>{children}</LocaleSync>
                </main>
              </AnnouncerProvider>
            </WalletProvider>
          </MotionProvider>
        </ErrorBoundary>
      </body>
    </html>
  );
}
