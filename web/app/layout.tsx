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

// TODO: Move these from pages/_app.tsx during migration
// import { ErrorBoundary } from '@/components/ErrorBoundary';
// import { WalletProvider } from '@/context/WalletContext';
// import '@/styles/globals.css';

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
    <html lang="en">
      <body>
        {/* TODO: Wrap with providers from pages/_app.tsx during migration:
          <ErrorBoundary>
            <WalletProvider>
              {children}
            </WalletProvider>
          </ErrorBoundary>
        */}
        {children}
      </body>
    </html>
  );
}
