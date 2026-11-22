# API keys for integrations

Server-to-server integrations that cannot hold an interactive wallet session
authenticate with a scoped API key. Implemented in `core/src/api_keys.rs`.

## Lifecycle

Managing keys requires a wallet session (`Authorization: Bearer <JWT>`). An API
key can never create, rotate or revoke keys.

| Method | Path | Effect |
| --- | --- | --- |
| `POST` | `/api-keys` | Create a key. **The response is the only time the secret is shown.** |
| `GET` | `/api-keys` | List your keys: metadata only (id, name, scopes, status, timestamps). |
| `POST` | `/api-keys/:key_id/rotate` | Issue a new secret. The previous secret stops working immediately. |
| `DELETE` | `/api-keys/:key_id` | Revoke permanently. Revoked keys cannot be rotated back to life. |

Create request:

```json
{ "name": "nightly-reporting", "scopes": ["vaults:read", "reports:read"], "expires_in_days": 90 }
```

* `name`: 1–64 printable characters.
* `scopes`: at least one of the scopes below. Unknown scopes are rejected.
* `expires_in_days`: 1–365, default 90.
* At most 25 active keys per manager.

Create and rotate return `{"key": {...metadata...}, "secret": "pgk_<key id>.<secret>"}`
with `Cache-Control: no-store`.

### Scopes

| Scope | Allows |
| --- | --- |
| `vaults:read` | Reading vault records. |
| `vaults:write` | Changing vault records. Only callers whose own role can write vaults may grant it. |
| `simulations:run` | Running read-only simulations and analyses. |
| `reports:read` | Reading reports. |

No scope grants key management, manager approval or administrator access.
Grant the smallest set the integration needs, and use separate keys for
separate integrations so each can be rotated or revoked on its own.

### Tenant separation

Keys belong to the manager that created them. Listing shows only your keys,
and rotating or revoking another manager's key returns `404`, the same as a key
that does not exist.

### Storage and audit

The secret is 256 bits from the OS random generator. Only a non-reversible
SHA-256 verifier (over the key id and the secret) is stored and compared in
constant time; the secret cannot be recovered from the server, logs or
listings. Creation, rotation, revocation and rejected authentications are
written to the audit log with the key id, never the secret.

Keys are held in process memory in this release: they do not survive a restart
and are not shared between instances.

## Using a key

Send it in the `X-API-Key` header over HTTPS:

```http
GET /v1/integrations/api-key HTTP/1.1
Host: api.perigee.example
X-API-Key: pgk_3f9c1a7b2e4d6f80.5yJ…
```

`GET /integrations/api-key` returns the key id and scopes, which is a safe way
for an integration to confirm its configuration. A missing, malformed, wrong,
rotated, revoked or expired key gets `401` with `WWW-Authenticate: ApiKey`; a
valid key without the required scope gets `403`.

## Safe credential handling

```bash
# 1. Create the key with a wallet session and capture the secret straight
#    into a secret manager; never echo it or write it to a file in the repo.
curl -sS -X POST https://api.perigee.example/v1/api-keys \
  -H "Authorization: Bearer $PERIGEE_SESSION_JWT" \
  -H "Content-Type: application/json" \
  -d '{"name":"nightly-reporting","scopes":["vaults:read"]}' \
  | jq -r .secret \
  | aws secretsmanager put-secret-value --secret-id perigee/reporting-key --secret-string file:///dev/stdin

# 2. The integration reads it from the secret manager / environment at runtime.
curl -sS https://api.perigee.example/v1/integrations/api-key \
  -H "X-API-Key: $PERIGEE_API_KEY"
```

* Treat the key like a password: keep it in a secret manager or an injected
  environment variable, never in source control, URLs, query strings, client-side
  code or logs.
* Rotate on a schedule and immediately after any suspected exposure: call
  `POST /api-keys/:key_id/rotate`, deploy the new secret, and the old one is
  already dead.
* Revoke keys for decommissioned integrations with `DELETE /api-keys/:key_id`.
* Keys start with `pgk_`, so secret scanners can be configured to flag them.
