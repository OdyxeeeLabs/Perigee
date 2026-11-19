# Webhook delivery

`core/src/webhooks.rs` delivers events to customer endpoints.

## Request

```http
POST /your/endpoint
Content-Type: application/json
X-Perigee-Event-Id: evt_123
X-Perigee-Delivery-Id: dlv_45
X-Perigee-Signature: t=1760000000,v1=5257a869...

{"id": "evt_123", "type": "vault.created", "created_at": 1760000000, "data": { ... }}
```

## Verifying the signature

`v1` is the hex HMAC-SHA256, keyed with your endpoint secret, of `"<t>.<raw body>"`.
Recompute it over the **raw** body, compare in constant time, and reject timestamps older
than five minutes. `verify_signature(secret, header, body, now, 300)` does exactly this.

## Retries and idempotency

A delivery succeeds on any `2xx`. Anything else (including timeouts and connection errors)
is retried after **1 minute, 5 minutes, 15 minutes, 1 hour, 6 hours and 24 hours**; after
the last retry the delivery is dead-lettered. `X-Perigee-Event-Id` is the same on every
retry, so deduplicate on it. Each retry is re-signed with a fresh timestamp.

## Endpoints

Register with `register_endpoint(Endpoint::new(id, url, secret))`. Options: custom
`header`s, an `events` filter (empty means all event types), `set_enabled`, and a
`transform` closure that reshapes `data` per endpoint. URLs must be public `https`;
localhost, private ranges and link-local addresses are refused. Generate a secret with
`generate_secret()`.

## Operations

`deliveries(endpoint, status)` is the delivery log (every attempt with time, status code
and error). `replay(delivery_id, now)` starts a fresh retry cycle for any delivery,
typically a dead-lettered one. Implement `Transport` over your HTTP client; it must
refuse private addresses at connect time as well, since a hostname can resolve to one.
