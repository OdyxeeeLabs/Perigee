/**
 * Root App Router layout.
 *
 * MIGRATION STATUS: Foundation only — pages/ router still active.
 * This file establishes the App Router structure for future migration.
 *
 * To complete migration:
 * 1. Move pages/ files to app/ incrementally
 * 2. Convert _app.tsx providers to this layout
 * 3. Convert next/head usage to metadata exports
 * 4. Remove pages/ when all routes migrated
 *
 * See MIGRATION.md for detailed plan.
 */

import type { Metadata } from 'next';
import type { ReactNode } from 'react';
import { Inter } from 'next/font/google';
import { NetworkStatusBanner } from '@/components/NetworkStatusBanner';
import { API_URL } from '@/lib/api';
import { ErrorBoundary } from '@/components/ErrorBoundary';
import { WalletProvider } from '@/context/WalletContext';
import '@/styles/globals.css';
// WEB-15 (#469): mirror the Pages Router `MotionProvider` so the App Router
// routes also honour `prefers-reduced-motion` once migration completes.
import { MotionProvider } from '../components/MotionProvider';

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
  title: {
    template: '%s | Perigee',
    default: 'Perigee - Soroban Smart Contract Resource Analyzer',
  },
  description:
    'Explore, test, and analyze the CPU, RAM, and ledger footprint of Soroban smart contracts.',
};

export default function RootLayout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" className={inter.variable}>
      <body>
        <ErrorBoundary>
          <MotionProvider>
            <WalletProvider>
              <NetworkStatusBanner apiUrl={API_URL} />
              {children}
            </WalletProvider>
          </MotionProvider>
        </ErrorBoundary>
      </body>
    </html>
  );
}
