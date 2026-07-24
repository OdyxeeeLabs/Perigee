// migration.test.cjs — App Router migration validation tests
// Ensures:
// 1. Next.js version is pinned (no ^ or ~ prefix)
// 2. app/layout.tsx exists with required exports
// 3. Both routers can coexist without conflicts
// 4. MIGRATION.md is present and up to date
//
// Runs with: node --test ./__tests__/migration.test.cjs

'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const { readFileSync, existsSync } = require('fs');
const { join } = require('path');

// Get project root (one level up from web/)
const projectRoot = join(__dirname, '..');

// Helper to read package.json
function readPackageJson() {
  const pkgPath = join(projectRoot, 'package.json');
  const content = readFileSync(pkgPath, 'utf-8');
  return JSON.parse(content);
}

// Helper to check file exists
function fileExists(filePath) {
  return existsSync(join(projectRoot, filePath));
}

// Helper to read file content
function readFile(filePath) {
  const fullPath = join(projectRoot, filePath);
  return readFileSync(fullPath, 'utf-8');
}

// ── SUITE 1: Version Pinning ──────────────────────────────────────────────────

test('Next.js version pinned', async (t) => {
  const pkg = readPackageJson();
  const nextVersion = pkg.dependencies?.next ?? pkg.devDependencies?.next;

  await t.test('next version defined in package.json', () => {
    assert.ok(nextVersion, 'Next.js version not found in package.json');
  });

  await t.test('next version has no ^ prefix', () => {
    assert.ok(
      !nextVersion.startsWith('^'),
      `Next.js version must be pinned without ^ caret. Found: ${nextVersion}`
    );
  });

  await t.test('next version has no ~ prefix', () => {
    assert.ok(
      !nextVersion.startsWith('~'),
      `Next.js version must be pinned without ~ tilde. Found: ${nextVersion}`
    );
  });

  await t.test('next version is valid semver', () => {
    const semverRegex = /^\d+\.\d+\.\d+/;
    assert.match(
      nextVersion,
      semverRegex,
      `Next.js version must match semver X.Y.Z format. Found: ${nextVersion}`
    );
  });
});

// ── SUITE 2: App Router Foundation ────────────────────────────────────────────

test('App Router foundation files exist', async (t) => {
  await t.test('app/ directory exists', () => {
    assert.ok(fileExists('app'), 'app/ directory not found');
  });

  await t.test('app/layout.tsx exists', () => {
    assert.ok(fileExists('app/layout.tsx'), 'app/layout.tsx not found');
  });

  await t.test('app/page.tsx exists', () => {
    assert.ok(fileExists('app/page.tsx'), 'app/page.tsx not found');
  });

  await t.test('app/layout.tsx exports default RootLayout', () => {
    const content = readFile('app/layout.tsx');
    assert.ok(
      content.includes('export default function RootLayout'),
      'app/layout.tsx must export default function RootLayout'
    );
  });

  await t.test('app/layout.tsx exports metadata', () => {
    const content = readFile('app/layout.tsx');
    assert.ok(
      content.includes('export const metadata'),
      'app/layout.tsx must export const metadata'
    );
  });

  await t.test('app/layout.tsx imports Metadata type', () => {
    const content = readFile('app/layout.tsx');
    assert.ok(
      content.includes("import type { Metadata }"),
      'app/layout.tsx must import Metadata type from next'
    );
  });

  await t.test('app/page.tsx exists with placeholder', () => {
    const content = readFile('app/page.tsx');
    assert.ok(
      content.includes('export default function'),
      'app/page.tsx must have default export'
    );
  });
});

// ── SUITE 3: next.config.js Configuration ────────────────────────────────────

test('next.config.js migration setup', async (t) => {
  const content = readFile('next.config.js');

  await t.test('next.config.js contains useFileSystemPublicRoutes', () => {
    assert.ok(
      content.includes('useFileSystemPublicRoutes'),
      'next.config.js must include useFileSystemPublicRoutes for Pages Router coexistence'
    );
  });

  await t.test('next.config.js has migration status comments', () => {
    assert.ok(
      content.includes('MIGRATION STATUS') || content.includes('Pages Router'),
      'next.config.js should document migration status'
    );
  });
});

// ── SUITE 4: Migration Documentation ──────────────────────────────────────────

test('Migration documentation', async (t) => {
  await t.test('MIGRATION.md exists', () => {
    assert.ok(fileExists('MIGRATION.md'), 'MIGRATION.md not found at project root');
  });

  await t.test('MIGRATION.md contains Status section', () => {
    const content = readFile('MIGRATION.md');
    assert.ok(
      content.includes('Status:'),
      'MIGRATION.md must have Status section'
    );
  });

  await t.test('MIGRATION.md contains Phase 1', () => {
    const content = readFile('MIGRATION.md');
    assert.ok(
      content.includes('Phase 1'),
      'MIGRATION.md must document Phase 1'
    );
  });

  await t.test('MIGRATION.md contains Pages Inventory', () => {
    const content = readFile('MIGRATION.md');
    assert.ok(
      content.includes('Pages Inventory'),
      'MIGRATION.md must have Pages Inventory section'
    );
  });

  await t.test('MIGRATION.md documents key differences', () => {
    const content = readFile('MIGRATION.md');
    assert.ok(
      content.includes('Metadata') || content.includes('next/head'),
      'MIGRATION.md must document metadata differences'
    );
    assert.ok(
      content.includes('getServerSideProps') || content.includes('Data Fetching'),
      'MIGRATION.md must document data fetching changes'
    );
  });

  await t.test('MIGRATION.md has rollback plan', () => {
    const content = readFile('MIGRATION.md');
    assert.ok(
      content.includes('Rollback') || content.includes('git'),
      'MIGRATION.md should document rollback strategy'
    );
  });
});

// ── SUITE 5: Pages Router Backward Compatibility ──────────────────────────────

test('Pages Router backward compatibility', async (t) => {
  await t.test('pages/ directory exists', () => {
    assert.ok(fileExists('pages'), 'pages/ directory not found');
  });

  await t.test('pages/_app.tsx exists', () => {
    const tsxExists = fileExists('pages/_app.tsx');
    const jsExists = fileExists('pages/_app.js');
    assert.ok(
      tsxExists || jsExists,
      'pages/_app.tsx or pages/_app.js not found'
    );
  });

  await t.test('pages/index.tsx exists', () => {
    const tsxExists = fileExists('pages/index.tsx');
    const jsExists = fileExists('pages/index.js');
    assert.ok(
      tsxExists || jsExists,
      'pages/index.tsx or pages/index.js not found'
    );
  });

  await t.test('pages/_app.tsx contains providers', () => {
    const content = readFile('pages/_app.tsx');
    assert.ok(
      content.includes('Provider') || content.includes('export default'),
      'pages/_app.tsx should contain provider wrapping'
    );
  });
});

// ── SUITE 6: Router Coexistence ───────────────────────────────────────────────

test('Router coexistence validation', async (t) => {
  await t.test('both app/ and pages/ directories exist', () => {
    const appExists = fileExists('app');
    const pagesExists = fileExists('pages');
    assert.ok(
      appExists && pagesExists,
      'Both app/ and pages/ directories must exist for incremental migration'
    );
  });

  await t.test('app/layout.tsx and pages/_app.tsx both exist', () => {
    const layoutExists = fileExists('app/layout.tsx');
    const appExists = fileExists('pages/_app.tsx');
    assert.ok(
      layoutExists && appExists,
      'Both app/layout.tsx and pages/_app.tsx should coexist during migration'
    );
  });

  await t.test('middleware.ts is compatible with both routers', () => {
    const middlewareExists = fileExists('middleware.ts');
    if (middlewareExists) {
      const content = readFile('middleware.ts');
      assert.ok(
        content.includes('NextResponse'),
        'middleware.ts should use NextResponse (compatible with both routers)'
      );
    }
  });
});

// ── SUITE 7: Configuration Files ──────────────────────────────────────────────

test('Configuration files in place', async (t) => {
  await t.test('tsconfig.json exists', () => {
    assert.ok(fileExists('tsconfig.json'), 'tsconfig.json not found');
  });

  await t.test('tailwind.config.js exists', () => {
    assert.ok(fileExists('tailwind.config.js'), 'tailwind.config.js not found');
  });

  await t.test('postcss.config.js exists', () => {
    assert.ok(fileExists('postcss.config.js'), 'postcss.config.js not found');
  });

  await t.test('next.config.js exists', () => {
    assert.ok(fileExists('next.config.js'), 'next.config.js not found');
  });
});

// ── SUITE 8: Providers Ready for Migration ────────────────────────────────────

test('Providers ready for App Router migration', async (t) => {
  await t.test('WalletContext has use client directive', () => {
    const content = readFile('context/WalletContext.tsx');
    assert.ok(
      content.includes('"use client"'),
      'WalletContext should have "use client" directive for App Router'
    );
  });

  await t.test('ErrorBoundary component exists', () => {
    assert.ok(
      fileExists('components/ErrorBoundary.tsx'),
      'ErrorBoundary component not found'
    );
  });

  await t.test('Global styles exist', () => {
    assert.ok(
      fileExists('styles/globals.css'),
      'styles/globals.css not found'
    );
  });
});
