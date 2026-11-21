# Notification service

`core/src/notifications.rs` delivers messages over **email**, **push**, **in-app** and
**SMS**.

## Flow

`send()` renders the template, applies the user's preferences and either queues the
message or, with `send_at` in the future, schedules it. A worker calls `process(now,
limit)`, which releases due scheduled messages and hands each queued one to its channel's
`ChannelProvider`. Bind `DeliveryQueue` to the job system so delivery runs as background
jobs; `InMemoryQueue` is provided for tests.

Provider adapters implement `ChannelProvider` for the real backends: SendGrid or SES
(email), FCM or APNs (push), WebSocket or SSE fan-out (in-app), Twilio (SMS). Retryable
errors are requeued up to `max_attempts` (default 3).

## Templates

* Versioned: register `welcome` v1, v2, ...; a request uses the latest unless it pins
  `template_version`.
* Localised: a request's locale (or the user's saved locale) resolves `fr-CA`, then `fr`,
  then `en`.
* `{{name}}` substitutes an HTML-escaped value on the email channel and plain text
  elsewhere; `{{{name}}}` is always raw. A missing variable is an error.
* MJML is compiled to HTML before registration; the registered email body is the HTML.

## Preferences and unsubscribe

* Per user: disable a channel, set a locale.
* Opt-outs are per channel and category (`ALL_CATEGORIES` covers the channel). Messages
  flagged `transactional` (password resets, security alerts) ignore opt-outs.
* `unsubscribe_token()` returns an HMAC-signed token for the unsubscribe link;
  `unsubscribe(token)` verifies it and records the opt-out without a login.

## Per-user preferences API

Authenticated users read and change **their own** preferences; the user id is
always the caller's wallet address, so one user can never see or edit
another's.

| Method | Path | Effect |
| --- | --- | --- |
| `GET` | `/notifications/preferences` | Effective preferences, with defaults filled in. |
| `PUT` | `/notifications/preferences` | Partial update, validated in full and applied atomically. |

```json
{
  "channels_enabled": { "push": false },
  "categories": { "vault_activity": { "sms": false }, "product_updates": { "push": true } }
}
```

Channels are `email`, `push`, `in_app` and `sms`. Unknown channels, unknown
fields, unsupported categories, mandatory categories and empty updates are
rejected with `422`, and a rejected update changes nothing.

### Categories

| Category | Controllable | Default |
| --- | --- | --- |
| `vault_activity` | yes | on for every channel |
| `strategy_alerts` | yes | on for every channel |
| `agent_health` | yes | on for every channel |
| `reports` | yes | on for every channel |
| `product_updates` | yes | email and in-app only (push and SMS off) |
| `security` | **no, mandatory** | always delivered |
| `account` | **no, mandatory** | always delivered |

### Precedence

1. `transactional` requests and the mandatory `security` / `account`
   categories are **always delivered**. Channel switches, unsubscribes and
   category choices never suppress them.
2. Otherwise a disabled channel (`channels_enabled`) suppresses everything on
   that channel.
3. Then an unsubscribe for the channel (all categories or this one).
4. Then the user's explicit category/channel choice, or the default above.

Free-form categories outside this table (legacy callers) are only affected by
rules 2 and 3.

## Tracking

Every message has a `DeliveryRecord`: `scheduled` / `queued` -> `sent` -> `delivered` |
`bounced` | `failed`, or `suppressed` when preferences blocked it. Provider callbacks call
`record_event`.

## Test mode

`with_test_mode(true)` renders and records messages in an outbox (`test_outbox()`) and
never calls a provider, for development and CI.
