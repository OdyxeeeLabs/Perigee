# Bulk vault metadata update (`POST /vaults/bulk-metadata`)

Relabel several of your own vaults in one request. Implemented in
`core/src/vault_bulk_metadata.rs`; metadata is stored in the `vault_metadata`
table (migration `011_create_vault_metadata_table`).

## Atomicity

The operation is **all-or-nothing**. Every item is validated first, then all
updates run in one database transaction. If any item is rejected, nothing is
written and the response lists every rejected item by its index in the
request. Clients can fix the listed items and resend the whole batch.

## Request

```json
{
  "updates": [
    { "id": "vault-1", "version": 3, "name": "Treasury", "labels": ["core", "tier:1"] },
    { "id": "vault-2", "version": 1, "reporting_reference": "RPT-2026-11" },
    { "id": "vault-3", "version": 4, "reporting_reference": null }
  ]
}
```

| Field | Rules |
| --- | --- |
| `updates` | 1–50 items. |
| `id` | Required. Each vault may appear only once per batch. |
| `version` | Required. The vault's current version (optimistic locking). |
| `name` | Optional. 1–128 printable characters. |
| `labels` | Optional. Replaces the label set: up to 10 labels of 1–32 characters from `a-z 0-9 - _ : .`. Duplicates are collapsed. |
| `reporting_reference` | Optional. 1–64 printable characters, or `null` to clear it. |

Each item must change at least one of `name`, `labels`, `reporting_reference`.
Omitted fields keep their current value.

**On-chain policy cannot be changed here.** Policy and lifecycle fields
(`config_json`, `status`, `policy`, fees, assets, `manager_id`, ...) are
rejected with `forbidden_field`; any other unknown field is rejected with
`unknown_field`.

## Responses

`200 OK`: every item applied; each vault's `version` went up by one.

```json
{
  "atomic": true,
  "applied": 2,
  "vaults": [
    { "id": "vault-1", "version": 4, "name": "Treasury", "labels": ["core", "tier:1"],
      "reporting_reference": null, "updated_at": "2026-11-19T08:30:15.120Z" }
  ]
}
```

Rejected batches return `application/problem+json` with `"applied": 0` and,
for item problems, an `errors` array of `{index, id, field, code, message}`:

| Status | When |
| --- | --- |
| `422` | Batch is empty, larger than 50, or an item fails validation (`duplicate_id`, `forbidden_field`, `unknown_field`, `invalid_*`, `empty_update`). |
| `409` | At least one item's `version` is stale (`version_conflict`). Reload and retry. |
| `404` | Every rejected item refers to a vault that does not exist, is deleted, belongs to another manager, or is outside the token's vault scope (`not_found`). |
| `403` | The caller's role cannot write vaults. |
| `503` | The vault store is unavailable; nothing was written. |

## Tenant isolation

Only the caller's own vaults can be updated. A vault owned by another manager
gets the same `not_found` answer as a vault that does not exist, so the
endpoint does not reveal which vault ids exist. Vault-scoped tokens are limited
to their scopes.
