# Security Audit Checklist — Perigee Backend and Contracts

This checklist is mandatory before a production release. A reviewer must verify the current deployment, not only the source tree, and record accepted exceptions in the release ticket.

## Scope and ownership

- [ ] The release commit, backend image digest, frontend commit, contract WASM hashes, and deployment manifest are recorded.
- [ ] The security reviewer is independent of the feature implementer.
- [ ] At least two authorized operators can perform emergency pause, secret rotation, rollback, and audit-log verification.
- [ ] Public, partner, and administrative network paths are documented and separated at the ingress layer.

## Entry-point inventory

| Entry point | Authentication | Authorization | Sensitive effect | Required control |
|---|---|---|---|---|
| `/`, `/swagger-ui`, `/api-docs/openapi.json` | Public | None | Reveals service and API metadata | Restrict or disable at ingress in production |
| `/health` | Public | None | Reveals liveness only | No configuration, dependency, or secret details |
| `/ready` | Public/internal | None | Reveals dependency health | Restrict at ingress; return a generic unavailable response |
| `/metrics` | Public/internal | None | Reveals operational telemetry | Restrict to monitoring networks; never expose high-cardinality secrets |
| `/auth/challenge`, `/auth/verify` | Public | None | Starts SEP-10 authentication | Rate limits, bounded payload sizes, replay-safe nonce and time bounds |
| `/auth/refresh`, `/auth/revoke` | Public | Refresh-token possession | Rotates or revokes a token family | Store only token hashes; detect reuse; remain available during emergency pause for recovery |
| `/auth/jwks` | Public | None | Publishes verification keys | Publish only current and unexpired overlap keys |
| `/fees/*` | Public | None | Reads market estimates | Integer arithmetic, bounded query ranges, no internal errors |
| `/managers/register`, `/managers/status/:address` | Public | None | Creates or reveals onboarding state | Abuse controls and minimal response data |
| `/managers*` reads and decisions | JWT | Admin | PII and KYC state | Authenticated router and server-side admin check |
| `/v1/*` and root analysis routes | JWT | Scope and tenant/vault checks | Executes analysis and spends resources | Required scope, tenant scope, rate limit, body limit |
| `/v1/*` and root vault routes | JWT | Tenant/vault roles | Reads or mutates vault configuration | Optimistic locking, ownership checks, audit events |
| `/v2/vaults*` | JWT | Tenant/vault roles | Feature-gated vault aliases | `enable_vault_v2`; same authorization and audit controls as root vault routes |
| `/fees/v2/recommend` | Public | None | Feature-gated fee estimate | `enable_new_fee_model`; generic not-found response while disabled |
| `/auth/scoped-token` | JWT | Approved manager/admin | Mints delegated authority | Cannot exceed issuer authority; non-admin wildcard and cross-tenant scopes are rejected |
| `/auth/emergency-pause` | JWT | Admin | Disables authentication verification | Protected route, admin check, audit chain entry; refresh remains available for recovery |
| `/reconcile*` | JWT | Authenticated operator | Starts jobs and exposes financial reports | Authenticated router and least-privilege ingress |
| `/ws/jobs/:job_id` | Capability URL | Possession of unguessable job ID | Streams job output | Non-guessable IDs, short retention, no sensitive payloads |

- [ ] Every new route is added to this inventory; stable routes are also added to OpenAPI with their authentication requirement, while disabled gated aliases may remain unpublished.
- [ ] New state-changing routes are placed in the authenticated router before deployment.
- [ ] Public 404 and error responses do not disclose route, stack, SQL, filesystem, or provider internals.

## Trust boundaries and data flows

- [ ] Internet or partner input is validated for size, type, range, encoding, and allowed characters before business logic.
- [ ] JWT claims are signature-, algorithm-, issuer-, expiry-, key-id-, subject-, and scope-validated.
- [ ] Vault identifiers in path, body, and claims are checked against the authenticated user's scopes.
- [ ] Provider URLs and peer URLs are allowlisted or validated; credentials are never sent to discovered peers.
- [ ] Database queries are parameterized, use bounded limits, and preserve tenant ownership predicates.
- [ ] Redis keys and cached values include tenant context and expire; cache failures do not bypass authorization.
- [ ] Filesystem paths for WASM, cache, strategy persistence, and migrations cannot escape approved directories.
- [ ] Stellar network passphrase validation succeeds before signing state is initialized.
- [ ] Production error responses are redacted while full diagnostics remain in access-controlled logs.

## Authentication and authorization

- [ ] Production refuses to start with an ephemeral JWT signing key.
- [ ] `JWT_KEY_RING` or `JWT_PRIVATE_KEY` is supplied by a secret manager, not committed configuration.
- [ ] Exactly one RSA signing key of at least 2048 bits has a unique `kid`.
- [ ] Previous public keys remain published only for the configured overlap, which is at least the access-token lifetime.
- [ ] Emergency pause cannot be called without a valid administrator token.
- [ ] Manager enumeration, reads, approval, and rejection cannot be called without a valid administrator token.
- [ ] Login clients cannot self-assign roles or vault scopes; vault, scoped-token, and delegated-role checks are enforced server-side for every object.
- [ ] Refresh-token reuse revokes the complete token family.
- [ ] Rate limits are applied by authenticated tenant and return `429` without revealing account state.

## Secrets and rotation

- [ ] Database, Redis, RPC, audit-chain, JWT, CI, and provider credentials are injected at runtime.
- [ ] Secrets are absent from source, images, CI output, metrics, audit events, panic messages, and application configuration logs.
- [ ] JWT rotation is rolling: first deploy key-ring support with the old signer and the future new public key, then switch signers while retaining the old public key, wait at least `JWT_KEY_OVERLAP_SECS`, and remove old keys.
- [ ] API keys and other opaque secrets use the typed `SecretKeyring` pattern with one current value and expiring verification values.
- [ ] Rotation and retirement are audited with actor, key identifier, timestamp, and outcome; secret values are never logged.
- [ ] Emergency access to the secret manager is tested and available to at least two operators.

`JWT_KEY_RING` is a JSON array. The current entry has `signing: true`, a unique `kid`, and `private_key_pem`; previous entries normally contain only `public_key_pem`. An optional Unix `verification_not_after` value shortens a key's acceptance window. `JWT_CURRENT_KEY_ID` must match the signing entry. PEM newlines must be JSON-escaped when the value comes from an environment variable.

## Database and service boundaries

- [ ] `DATABASE_MAX_CONNECTIONS`, `DATABASE_MIN_CONNECTIONS`, and `DATABASE_ACQUIRE_TIMEOUT_SECS` match database and deployment capacity.
- [ ] `database_pool_connections{state="active|idle|waiting"}`, pool maximum, utilization, alerting state, and alert count are scrapeable.
- [ ] `DATABASE_POOL_ALERT_THRESHOLD_PERCENT` is approved and alerts route to an owned on-call destination.
- [ ] Readiness fails when the database or all configured RPC providers are unavailable.
- [ ] Graceful shutdown stops new work and returns shared pool resources without exposing credentials.
- [ ] Redis and provider clients have bounded timeouts and do not create an unbounded pool per request.

## Audit and logging

- [ ] Production provides `AUDIT_LOG_SIGNING_KEY` as at least 32 random bytes encoded as hex.
- [ ] Administrative, authentication, authorization-denial, rotation, and emergency events are structured and correlated by request ID.
- [ ] The HMAC audit chain is exported and independently verified after deployment.
- [ ] Refresh tokens, access tokens, private keys, API keys, database URLs with credentials, and raw authorization headers are redacted.
- [ ] Log retention, access, alerting, and clock synchronization are documented and monitored.

## Feature flags and production configuration

- [ ] Backend flags are declared in the typed `FeatureFlag` enum and default to disabled unless explicitly approved.
- [ ] `FEATURE_FLAGS` and individual flag environment variables contain valid booleans; invalid or unknown flags stop startup.
- [ ] Disabled experimental routes return a generic not-found response and perform no side effects.
- [ ] `APP_ENV=production` requires explicit CORS origins, persistent JWT material, an audit signing key, and at least one configured administrator.
- [ ] Rollback disables new flags without requiring a code rollback.

## Contracts and dependencies

- [ ] Every user-fund contract integrates `EmergencyGuard` and has tested granular pause controls.
- [ ] Privileged contract operations require M-of-N multisig approval with N at least 3 and M at least 2.
- [ ] Relayer keys are separate from admin keys and cannot pause, withdraw, or administer contracts.
- [ ] Deployed contract IDs, WASM hashes, constructor arguments, and admin addresses match the reviewed manifest.
- [ ] `Cargo.lock` is committed and security-sensitive dependencies do not use wildcard versions.
- [ ] Contract code contains no `unsafe` blocks.
- [ ] Dependency advisories are reviewed and either fixed or covered by a time-bound exception.

## Automated release gates

| Gate | Command | Required result |
|---|---|---|
| Custom security lint | `python scripts/core_security_lint.py` | Exit 0 |
| Clippy | `cargo clippy --locked -p Perigee-core --all-targets -- -D warnings` | Exit 0 |
| Dependency audit | `cargo audit --file Cargo.lock` | No unaccepted advisory |
| CI orchestration | `.github/workflows/core-security.yml` | All three jobs pass on the pull request |

- [ ] CI results are linked from the release ticket.
- [ ] Any waiver names the finding, owner, expiry date, compensating control, and approving reviewer.
- [ ] Local commands are not a substitute for passing CI.

## Incident response

- [ ] On-call coverage and escalation paths are active before launch.
- [ ] Emergency pause and contract guard procedures are reachable without repository access.
- [ ] Key compromise can revoke the affected key without invalidating unrelated credentials.
- [ ] A post-incident review includes audit-chain verification, credential rotation, and regression coverage.

## Sign-off

| Role | Name | Date | Evidence |
|---|---|---|---|
| Lead Engineer |  |  |  |
| Security Reviewer |  |  |  |
| Operations Owner |  |  |  |
| Multisig Key Holder |  |  |  |
