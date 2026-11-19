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

## Tracking

Every message has a `DeliveryRecord`: `scheduled` / `queued` -> `sent` -> `delivered` |
`bounced` | `failed`, or `suppressed` when preferences blocked it. Provider callbacks call
`record_event`.

## Test mode

`with_test_mode(true)` renders and records messages in an outbox (`test_outbox()`) and
never calls a provider, for development and CI.
