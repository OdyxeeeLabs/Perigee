# Perigee Vault Metadata

On-chain vault descriptions, trait discovery indexes, and version history.

Closes [#639](https://github.com/OdyxeeeLabs/Perigee/issues/639).

Every other Perigee contract stores *state*; this one stores *descriptions*. A
vault's name, symbol, strategy, risk tier and documentation pointer need to be
discoverable without loading each vault contract individually — and they need to
be **versioned**, because a vault's strategy and risk tier change and a discovery
client reading only the current value has no way to know it did.

## Discovery by trait, not by enumeration

`list_by_strategy` and `list_by_risk` return every vault matching a trait. The
alternative — enumerate all vaults and filter off-chain — is O(vaults) per query
and gets slower as the platform grows. The indexes are maintained on write, so a
trait lookup is one storage read plus the entries.

Indexes are **rebuilt on every metadata write** rather than incrementally diffed.
For a record that changes a handful of times in its life, rewriting one vault's
index entries costs less in code and less in risk than incremental maintenance
that can drift out of sync with the record.

`updating_a_vault_moves_it_between_indexes` and
`deregistering_removes_the_vault_from_every_index` pin the invariant that the
index and the record never disagree — a mismatch there returns a vault that no
longer matches the trait it was queried by, silently.

## Versioning

Every update bumps `version` and appends a `MetadataVersion` to history, and
emits `meta_up` with `(previous_version, new_version)`. An indexer therefore never
has to poll for changes; it consumes events and keeps history itself.

History is trimmed to `HISTORY_DEPTH` (16) entries. Older versions are dropped
from storage but remain in the event stream, which is the durable record —
`deregistering_keeps_the_history` covers the case where the live record is gone
but the trail is not.

## Field bounds

| Field | Max length |
| --- | --- |
| `name` | 32 |
| `symbol` | 12 |
| `strategy` | 32 |
| `metadata_uri` | 256 |

`risk_score` is bounded to 0–100, and `risk_tier` gives a coarse filter so a
discovery client does not have to parse documentation to tell a conservative vault
from an aggressive one.

## Errors

| Code | Variant | Meaning |
| --- | --- | --- |
| 1 | `NotInitialized` | Used before `initialize`. |
| 2 | `AlreadyInitialized` | `initialize` called more than once. |
| 3 | `Unauthorized` | Caller is not the registry admin. |
| 4 | `MetadataNotFound` | No metadata for the vault. |
| 5 | `MetadataExists` | Metadata already registered. |
| 6 | `EmptyField` | A required field is empty. |
| 7 | `FieldTooLong` | A field exceeds its bound. |
| 8 | `InvalidRisk` | Risk score above 100. |
| 9 | `HistoryOutOfRange` | No retained version at that index. |
| 10 | `Overflow` | A counter overflowed. |

## Events

- `meta(vault)` → `version`
- `meta(vault, up)` → `(previous_version, version)`
- `meta_x(vault)` → `version`
- `cfg(vault)` → `virtual_offset` (not used here; see the accounting crate)

## Tests

```bash
cargo test -p perigee-vault-metadata
```

Covers registration and field bounds, discovery by strategy and risk, index
consistency across update and deregistration, version ordering, history trimming,
history survival across deregistration, and authorization.