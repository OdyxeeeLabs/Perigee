# Secrets Rotation

Closes [#625](https://github.com/OdyxeeeLabs/Perigee/issues/625).

Every long-lived secret Perigee holds is declared in
[`config/security/secrets-rotation.json`](../config/security/secrets-rotation.json)
with an owner, a source, a rotation procedure, an emergency procedure and an
audit flag. That file is validated in CI by
[`scripts/security/validate_secrets_rotation.py`](../scripts/security/validate_secrets_rotation.py),
so a secret cannot be added without a rotation policy.

## Schedule

| Policy | Max age | Grace | Applies to |
| --- | --- | --- | --- |
| `critical` | 90 days | 14 days | Database passwords, CI registry tokens |
| `standard` | 90 days | 7 days | Redis, RPC keys, Sentry DSN |
| `encryption_key` | 365 days | 30 days | JWT signing key, vault DEK KEK |

The validator pins `max_age_days` to these values, so changing a schedule
requires changing this table in the same commit.

## Zero-downtime rotation

Every rotation follows the same four-step sequence. The point of the *dual
secret window* is that step 2 and step 4 are separated by a verification window
long enough for in-flight work to pick up the new value, so no request is ever
rejected because it used the old one.

1. **Generate.** Mint the replacement in the owning secrets manager. Never in a
   terminal, never in a ticket, never in a commit.
2. **Deploy both.** Publish the new secret alongside the old; consumers accept
   either. Applications verify against the new secret and *sign* with the old
   during the window, so existing sessions and signed URLs stay valid.
3. **Verify.** Watch the verification metric for at least the grace period:
   - new-secret verifications trending up and old trending to zero;
   - auth failure rate flat;
   - for the JWT keyring, no signature errors at all.
4. **Revoke.** Remove the old secret. Record the rotation in the audit trail:
   who, when, which secret, why, and the verification evidence.

Skipping step 3 and revoking early is the single most common way this goes
wrong, and it takes the service down rather than the attacker.

## Emergency rotation

For a suspected compromise, the grace period is skipped and the order is
reversed:

1. Revoke the old secret **first**. Accept the availability hit.
2. Generate and deploy the new one.
3. Verify.
4. Record, and open an incident rather than closing the rotation ticket.

The rule is that availability is the thing you sacrifice, not confidentiality.
If a rotation was triggered by a suspected compromise rather than by schedule,
open an incident rather than closing the rotation ticket.

## Auditing

Every access to a secret — not just rotation — is logged with actor, timestamp,
secret reference and reason. Key access additionally requires approval and is
audited in KMS, so "who decrypted a customer's PII" has an answer. The
`audit: true` flag in the config is enforced by the validator; a secret without
it fails CI.

## Adding a secret

1. Add an entry to `config/security/secrets-rotation.json`: `name`, `policy`,
   `owner`, `source`, `consumers`, `rotation_procedure`,
   `emergency_procedure`, `audit: true`.
2. Run the validator. It checks that every declared consumer is a file that
   actually exists, so a renamed module cannot silently drop out of rotation.
3. Update this table if the policy is new.

## Checking

```bash
python3 scripts/security/validate_secrets_rotation.py
```

The validator fails on a value pasted into the config instead of a reference
(`=` , `ghp_`, `sk-`, `xox`, `AKIA`, `-----BEGIN`), on a policy with the wrong
max age, on a missing owner/source/procedure, on `audit` not being `true`, and
on a consumer path that no longer exists.

The same check runs as the `policy` job in
[`.github/workflows/security-scan.yml`](../.github/workflows/security-scan.yml).