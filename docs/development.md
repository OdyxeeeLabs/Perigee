# Development Guide

This guide covers the local setup and the commands a web contributor is expected
to know. The repository is a Cargo workspace plus a Yarn-managed Next.js app in
`web/`.

## Prerequisites

- **Rust** stable (with `rustup` and the `wasm32-unknown-unknown` target for
  contract work)
- **Node.js 24** — this is the version used by the component-docs CI job; the
  commit-lint job also runs on Node 22+
- **Yarn 4.9.1** via Corepack. The root `package.json` pins
  `packageManager: yarn@4.9.1`, so enable Corepack before installing:
  ```bash
  corepack enable
  ```
- **Soroban CLI** — only needed when deploying or invoking contracts locally

> Some CI jobs (for example `component-docs.yml`) install the web workspace with
> `npm ci` from `web/`. Use `yarn install --immutable` at the repository root for
> the canonical Yarn workspace install. Do not mix the two lockfiles when adding
> a dependency.

## Monorepo setup

1. Fork `OdyxeeeLabs/Perigee` and clone your fork.
2. Add the upstream remote and create a topic branch from `upstream/main`:
   ```bash
   git remote add upstream https://github.com/OdyxeeeLabs/Perigee.git
   git fetch upstream main
   git switch -c fix/my-change upstream/main
   ```
3. Install workspace dependencies from the repository root:
   ```bash
   yarn install --immutable
   ```
   For a web-only checkout you can instead run `cd web && npm ci`.
4. Build the Rust core:
   ```bash
   cargo build -p Perigee-core
   ```
5. Build the contracts when changing `contracts/`:
   ```bash
   rustup target add wasm32-unknown-unknown
   cargo build --target wasm32-unknown-unknown --release
   ```

## Essential web commands

Run these from `web/` unless the table says otherwise.

| Command | Purpose |
| --- | --- |
| `yarn dev` | Start the Next.js development server (`npm run dev` also works). |
| `yarn lint` | ESLint over `.ts`, `.tsx`, `.js` and `.jsx` files. |
| `yarn typecheck` | `tsc --noEmit`; run this before opening a PR. |
| `yarn test` | `node --test` over `web/__tests__`, `web/lib` and `web/components` `*.test.cjs` files. |
| `node --test ./__tests__/web-16-aria-live.test.cjs` | Run one focused test file. |
| `yarn build` | Production Next.js build. |
| `yarn docs:check` | Verify generated component docs are in sync. |

From the repository root, `yarn lint` and `yarn typecheck` delegate to the web
workspace.

### Web test conventions

- Tests are plain `node:test` + `node:assert/strict` CommonJS files. There is no
  React test renderer, so component behaviour is usually covered in two ways:
  - **Source-contract tests** read a component's source and assert that the
    required state branches, ARIA attributes or API calls are present.
  - **Pure-logic tests** mirror a TypeScript helper in a `.cjs` file so the
    mapping can be exercised by `node --test` without a build step.
- Keep helpers that need behavioural tests in `web/lib/` as pure functions with
  no React imports. This is what `lib/onboardingChecklist.ts` does for the
  manager onboarding checklist.
- Status announcements use `LiveStatus`/`LiveAlert` from
  `components/LiveRegion.tsx`. A visible status label must also be present;
  colour or an icon alone is never sufficient.
- All user-facing strings belong in `web/messages/en.json` under a stable
  `namespace.key` name. Other locales fall back to English until translated.

## Rust and contract commands

```bash
cargo fmt --all
cargo clippy --all-targets
cargo test -p Perigee-core --lib
cargo test -p perigee-policy-vault
bash scripts/coverage.sh   # see docs/COVERAGE.md
```

Contract tests live in `contracts/<crate>/src/test.rs` and use `Env::default()`,
`env.mock_all_auths()` and generated `try_*` clients to assert deterministic
errors. Any change to persistent contract state must document its storage keys
and upgrade-compatibility behaviour in the PR description.

Deployment probes and their restart versus traffic-routing roles are described
in [Health probes](./HEALTH_PROBES.md).

## Contribution workflow

- Work from your fork; never push to `OdyxeeeLabs/Perigee:main`.
- Keep one feature branch and one PR per issue group, with a conventional-commit
  title under 70 characters.
- Link every assigned issue that the PR closes, each `Closes #N` on its own line.
- Do not commit secrets, generated lockfiles from a different package manager,
  or unrelated formatting churn.
- State persistence rules are in
  [Frontend Persistence](./FRONTEND_PERSISTENCE.md); only allowlisted,
  non-sensitive form fields may be stored, and the draft must be cleared after a
  successful submission.
