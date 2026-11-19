# Encryption at Rest

Closes [#627](https://github.com/OdyxeeeLabs/Perigee/issues/627).

Perigee's on-chain vault state is public by construction — that is the point.
What must be encrypted at rest is everything *off-chain*: the API's database of
manager and patient-linked records, the analytics events derived from them, the
backups of both, and the key material that protects them.

The deployed posture is declared in
[`config/security/encryption-at-rest.json`](../config/security/encryption-at-rest.json)
and asserted in CI by
[`scripts/security/verify_encryption_at_rest.py`](../scripts/security/verify_encryption_at_rest.py).

## Envelope encryption

Per-record data keys, one master key. Every sensitive field is encrypted with its
own DEK; the DEK itself is encrypted with a KEK that never leaves the KMS.

```text
root key ──(HSM)──> KEK ──wraps──> DEK ──encrypts──> ciphertext + tag
                     │
                     └── never exported, never in config, never logged
```

Why this shape rather than encrypting everything with one key:

- **Rotation is cheap.** Rotating the KEK re-wraps DEKs; it never re-encrypts
  record data. With one key, rotation means rewriting the entire dataset.
- **Blast radius is bounded.** One compromised DEK exposes one record, not one
  table.
- **The trust boundary is small.** Only the KEK depends on the KMS; DEKs are
  ordinary ciphertext next to the data.

### Parameters

| Parameter | Value | Note |
| --- | --- | --- |
| Algorithm | AES-256-GCM | authenticated encryption; a modified ciphertext fails to decrypt rather than yielding garbage |
| Nonce | 96-bit random per encryption | **never reused with the same DEK** — this is the one mistake that breaks GCM completely |
| Tag length | 128 bits | full-length; truncating buys nothing and weakens forgery resistance |
| AAD | `record_type`, `tenant_id` | binds the ciphertext to its context |
| DEK rotation | 365 days | re-encrypt records under a fresh DEK |
| KEK rotation | 365 days | re-wrap DEKs only |

### Why AAD matters

Without additional authenticated data, a ciphertext copied from tenant A's row
into tenant B's table decrypts cleanly — the key is the same, so the
authentication tag passes. Binding `tenant_id` and `record_type` into the AAD
means the tag no longer verifies in the wrong context, so the tampering is
detected instead of silently succeeding. This is the control that turns a
cross-tenant row copy from a disclosure into a loud failure.

## Field coverage

| Class | Fields | Treatment |
| --- | --- | --- |
| PII | `email`, `phone`, `full_name`, `wallet_address_label` | envelope-encrypted per record |
| Secrets | API keys, JWT material | secrets manager only; never in the database at all |
| Keys | DEKs | wrapped by the KEK |
| Transaction records | amounts, addresses, timestamps | not encrypted — needed for reconciliation and already public on-chain |
| Logs, analytics, error reports | — | carry no PII at all |

The last row matters as much as the first: encryption at rest does nothing for a
PII value that was already copied into a log pipeline. Redaction happens before
serialisation, not at rest.

## Key access

- **Logged**: every KMS decrypt call is audited with actor, timestamp, key ARN
  and request ID. "Who read a customer's record" has an answer.
- **Approved**: `Decrypt` is not in the default key policy for application roles;
  it requires an explicit grant, so a compromised service role cannot read
  plaintext on its own.
- **Root disabled**: the KMS key policy denies the account root user, so the
  escape hatch that normally defeats KMS-only designs is closed.

## Backups

Backups are encrypted with the same envelope scheme and a **separate** KEK, and
restore is tested on a schedule. An encrypted backup nobody can restore is not a
backup.

## Checking

```bash
python3 scripts/security/verify_encryption_at_rest.py
```

The validator fails when a control is undeclared (treating absent as off, since
that is how an auditor reads it), when the algorithm is not AES-256-GCM, when
the KEK is held in env or a file rather than a KMS/HSM, when DEK rotation
exceeds 365 days, when TDE, DEK-at-rest, backup encryption or access logging is
not `true`, and when access approval is not required.

## Nonce discipline

The single operational rule worth stating loudly: **a nonce must never be reused
with the same DEK.** Under GCM, reuse breaks confidentiality and authenticity
simultaneously and retroactively — it is not detectable from the ciphertexts.
Enforce it by generating the nonce inside the encryption call from a CSPRNG and
never accepting one as a parameter. The descriptor records
`nonce_strategy: "96-bit random per encryption, never reused with the same DEK"`
so the intent is auditable.