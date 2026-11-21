# Idempotency keys

Mutating requests (`POST`, `PUT`, `PATCH`, `DELETE`) accept an `Idempotency-Key`
header so a client can safely retry after a timeout or dropped connection without
doing the work twice. Implemented in `core/src/idempotency.rs`.

## Using it

```http
POST /vaults
Idempotency-Key: 123e4567-e89b-12d3-a456-426614174000
Content-Type: application/json

{"name": "treasury", ...}
```

* The key must be a UUID (`8-4-4-4-12` hex). Anything else is `400`.
* Generate a new key per logical operation and reuse it for every retry of that
  operation.
* The first request runs. Its response (status, headers, body) is stored for
  **24 hours**; a repeat of the same request within that time returns the stored
  response without running again.

## Scope

A key is scoped to the **user and the endpoint** (method + path). The same UUID may
be used by two users, or on two endpoints, without interfering.

## Errors

| Status | When |
| --- | --- |
| `400` | The key is not a UUID. |
| `409` | The key was already used with a **different** request (different body, method or path). This is a client bug: use a fresh key. |
| `409` | A request with this key is **still running**. Wait and retry with the same key. |
| `503` | The idempotency store is unavailable. The request was not run. |

`5xx` responses are not stored, so a retry after a server error is a fresh attempt.
`4xx` responses are stored and replayed.

## Server integration

```rust
let service = IdempotencyService::new(RedisBackend::new(client));
match service.begin(&Scope::new(user_id, method, path), key, &fingerprint(method, path, body))? {
    Outcome::Replay(response) => return Ok(response),     // already handled
    Outcome::Proceed(claim) => {
        let response = handler().await;
        service.complete(claim, response.clone())?;        // or service.abandon(claim)
        Ok(response)
    }
}
```

`begin` claims the key atomically (`SET key value NX EX 86400` on Redis), so when
several identical requests arrive at once exactly one runs and the rest receive
`409` until it finishes, then the stored response. `InMemoryBackend` is for tests
and single-process development only.
