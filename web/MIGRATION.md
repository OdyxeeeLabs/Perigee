# App Router Migration Plan

## Status: Foundation Established — In Progress

## Why We're Migrating

Next.js 16 ships with App Router as the default. Continuing with Pages Router will eventually cause:
- Version lag (future major versions may drop Pages Router support)
- Missed performance improvements (React Server Components, streaming, parallel routes)
- Maintenance burden (two routers in one codebase)

### Current State

- **Router**: Pages Router (`pages/` directory)
- **Next.js version**: `16.1.6` (pinned — see `package.json`)
- **Migration started**: July 2026
- **Coexistence**: Pages Router and App Router will run simultaneously during migration

## Migration Strategy

**Incremental migration** — Pages Router and App Router can coexist during transition. This allows deploying changes incrementally without a risky "big bang" cutover.

### Key Principles

1. **One page at a time** — Migrate pages to `app/` without touching the rest
2. **No breaking changes** — Keep Pages Router routes working until App Router replacement is ready
3. **Test each phase** — Verify functionality after each page migration
4. **Document as you go** — Update this file as pages migrate

## Phase 1 — Foundation ✅ COMPLETE

- [x] Pin Next.js version (`16.1.6`) to prevent accidental upgrades
- [x] Create `app/` directory with root layout
- [x] Create `app/page.tsx` placeholder
- [x] Document migration plan in `MIGRATION.md`
- [x] Update `next.config.js` with migration status comments

## Phase 2 — Simple Pages (No Data Fetching)

Migrate pages with no `getServerSideProps` or `getStaticProps`:

- [ ] **pages/index.tsx** → `app/page.tsx`
  - Uses `useState`, `useEffect`, and client-side API calls
  - Uses `next/head` for metadata — convert to `metadata` export in `app/layout.tsx`
  - Wrapped in `WalletProvider` and `ErrorBoundary` in `_app.tsx`
  - **Steps**:
    1. Copy content from `pages/index.tsx` to `app/page.tsx`
    2. Remove `Head` import and `<Head>` component usage
    3. Add metadata to `app/layout.tsx` (already templated)
    4. Test in dev environment
    5. Delete `pages/index.tsx` only after confirmed working

## Phase 3 — Providers & Layout

Convert `pages/_app.tsx` providers to `app/layout.tsx`:

- [ ] **Move global CSS**
  - Move `import '../styles/globals.css'` from `pages/_app.tsx` to `app/layout.tsx`
- [ ] **Move ErrorBoundary**
  - `ErrorBoundary` (class component) → needs conversion to error boundary pattern
  - Option A: Use `error.tsx` file (App Router native)
  - Option B: Keep as wrapper component in layout if simpler
- [ ] **Move WalletProvider**
  - Already has `"use client"` directive (future-proof)
  - Wrap `{children}` with `<WalletProvider>` in layout
- [ ] **Remove pages/_app.tsx**
  - Delete after all providers migrated to layout

## Phase 4 — Cleanup

- [ ] Delete `pages/` directory (only after all routes migrated)
- [ ] Remove `useFileSystemPublicRoutes: true` from `next.config.js`
- [ ] Update Next.js to latest version
- [ ] Remove this `MIGRATION.md` file or update to "Completed"

## Key Differences to Handle

### Metadata: `next/head` → `metadata` export

```typescript
// BEFORE (Pages Router)
import Head from 'next/head';

export default function Page() {
  return (
    <>
      <Head>
        <title>My Page</title>
        <meta name="description" content="..." />
      </Head>
      <div>Content</div>
    </>
  );
}

// AFTER (App Router)
export const metadata = {
  title: 'My Page',
  description: '...',
};

export default function Page() {
  return <div>Content</div>;
}
```

### Data Fetching: `getServerSideProps` → async components

```typescript
// BEFORE (Pages Router)
export async function getServerSideProps() {
  const data = await fetchData();
  return { props: { data } };
}

export default function Page({ data }) {
  return <div>{data}</div>;
}

// AFTER (App Router)
export default async function Page() {
  const data = await fetchData();
  return <div>{data}</div>;
}
```

### Client Components: `"use client"` directive

```typescript
// BEFORE: Implicit client context in pages/
import { useState } from 'react';

export default function Page() {
  const [count, setCount] = useState(0);
  return <button onClick={() => setCount(count + 1)}>{count}</button>;
}

// AFTER: Explicit "use client" for client-side interactivity
'use client';

import { useState } from 'react';

export default function Page() {
  const [count, setCount] = useState(0);
  return <button onClick={() => setCount(count + 1)}>{count}</button>;
}
```

### API Routes: `pages/api/` → `app/api/` with Route Handlers

```typescript
// BEFORE: pages/api/users.ts
export default function handler(req, res) {
  if (req.method === 'GET') {
    res.json({ users: [] });
  }
}

// AFTER: app/api/users/route.ts
export async function GET() {
  return Response.json({ users: [] });
}
```

## Pages Inventory

All pages currently in the project (from Part 1 analysis):

| Page | File | Data Fetching | Complexity | Status |
|------|------|---------------|-----------|--------|
| Home | `pages/index.tsx` | Client-side API calls | Low | Pending |

**Total: 1 page** — Simple migration, no API routes to migrate.

## Providers & Context Used

The project uses these providers in `pages/_app.tsx`:

1. **ErrorBoundary** — Class component that catches render errors
   - Currently wraps entire app
   - Must be converted to App Router error handling (error.tsx)
   - Already has custom fallback UI

2. **WalletProvider** — Custom context for Stellar wallet connection
   - Uses Zustand-like store pattern
   - Already has `"use client"` directive (future-proof)
   - Handles wallet kit initialization and persistence
   - No API data fetching — safe to move as-is

## Global Resources

- **CSS**: `styles/globals.css` (Tailwind imports + custom body styles)
- **Config**: `next.config.js`, `tsconfig.json`, `tailwind.config.js`
- **Environment**: `.env.example` (one var: `NEXT_PUBLIC_API_URL`)

## Migration Checklist

### Before You Start

- [ ] Read this file completely
- [ ] Understand the differences between Pages Router and App Router
- [ ] Have a test environment ready

### Phase 2 Checklist (Simple Pages)

- [ ] Backup current code or create a migration branch
- [ ] Copy `pages/index.tsx` to `app/page.tsx`
- [ ] Remove `next/head` usage
- [ ] Test the app in dev environment
- [ ] Verify wallet connection still works
- [ ] Verify contract analysis features work
- [ ] Commit changes with message: "chore: migrate pages/index.tsx to app/page.tsx"

### Phase 3 Checklist (Providers)

- [ ] Move `ErrorBoundary` and `WalletProvider` setup to `app/layout.tsx`
- [ ] Import global CSS in layout
- [ ] Test all functionality in dev environment
- [ ] Delete `pages/_app.tsx`
- [ ] Commit changes with message: "chore: migrate _app.tsx providers to app/layout.tsx"

### Phase 4 Checklist (Cleanup)

- [ ] Delete `pages/` directory
- [ ] Remove `useFileSystemPublicRoutes: true` from `next.config.js`
- [ ] Run `npm run build` to verify production build
- [ ] Update Next.js to latest version (if desired)
- [ ] Commit changes with message: "chore: complete App Router migration"

## Testing Strategy

After each phase, verify:

```bash
# Development server
npm run dev

# Check for errors in console
# Test wallet connection (if applicable)
# Test contract analysis features
# Test page navigation

# Production build
npm run build
npm run start
```

## Rollback Plan

If migration goes wrong:

```bash
# Restore from git
git checkout -- pages/ app/

# Or reset to pre-migration commit
git reset --hard <commit-before-migration>
```

## Resources

- [Next.js App Router Migration Guide](https://nextjs.org/docs/app/building-your-application/upgrading/app-router-migration)
- [Incremental Adoption Strategy](https://nextjs.org/docs/app/building-your-application/upgrading/app-router-migration#migrating-from-pages-to-app)
- [React Server Components](https://nextjs.org/docs/app/building-your-application/rendering/server-components)
- [Metadata API](https://nextjs.org/docs/app/building-your-application/optimizing/metadata)
- [Error Handling](https://nextjs.org/docs/app/building-your-application/routing/error-handling)
- [File Conventions](https://nextjs.org/docs/app/api-reference/file-conventions)

## Questions & Notes

- **Q: Can I use both pages/ and app/ at the same time?**
  - **A**: Yes! Next.js prioritizes `app/` routes over `pages/` routes when both exist. This enables incremental migration.

- **Q: Will my middleware.ts still work?**
  - **A**: Yes! `middleware.ts` in the root works with both Pages Router and App Router.

- **Q: Do I need to convert the Stellar wallet kit to Server Components?**
  - **A**: No. The `WalletProvider` is already marked `"use client"` and works fine in App Router layouts.

- **Q: What about TypeScript path aliases?**
  - **A**: The project currently uses relative imports (no aliases). This doesn't change during migration, but aliases can be added to `tsconfig.json` if desired: `"@/*": ["./*"]`

---

**Last Updated**: July 2026  
**Next Review**: After Phase 2 completion
