# Perigee Strategy Composability

Strategy-of-strategies, recursive vaults and fee layering.

Closes [#660](https://github.com/OdyxeeeLabs/Perigee/issues/660).

## The problem this solves

A strategy does not have to be a single deployment. A vault can allocate to a
**composite** — a strategy whose children are themselves strategies, several of
which may themselves be composites. Without a registry that describes those
trees, a vault has no way to resolve "allocate 10% to strategy X" into the
actual leaf deployments, and no way to know what fee applies when more than one
layer charges.

## The tree

A `CompositeStrategy` holds an ordered list of `ChildWeight` children and its
own `fee_bps`. Children are addresses; a child that is itself a registered
composite is expanded, and one that is not is a leaf. Nothing about a leaf is
stored here — the leaf is whatever contract implements the strategy interface —
so the tree is a pure allocation description.

Registration validates that children are non-empty, free of duplicates and
self-references, and that their weights sum to exactly 100%.

## Depth is bounded, not trusted

`resolve_allocation` expands a composite into a flat list of `AllocationLeaf`.
Recursion is bounded by `MAX_DEPTH` (4) and by a visited set that catches cycles,
so a malformed tree — one that references itself directly or through a loop — is
rejected rather than expanded forever. A vault that could be tricked into
unbounded recursion would be a vault that could be tricked into a failed
transaction.

## Fee layering

When a composite charges `fee_bps` and its children charge their own fees, the
fee that applies to capital flowing through the tree is **not** the sum.
`effective_fee_bps` composes them:

```text
effective(node) = fee(node) + Σ weight(child) × effective(child) / 10_000
```

Each layer takes its cut from what actually reaches it, so the effective fee is
the layered result, not a naive addition that would over-charge. The result is
capped at `MAX_FEE_BPS` (20%).

## Errors

| Code | Variant | Meaning |
| --- | --- | --- |
| 1 | `NotInitialized` | Used before `initialize`. |
| 2 | `AlreadyInitialized` | `initialize` called twice. |
| 3 | `Unauthorized` | Caller is not the admin. |
| 4 | `StrategyNotFound` | No composite at that address. |
| 5 | `StrategyAlreadyRegistered` | Composite already registered. |
| 6 | `InvalidChildren` | Empty, duplicate, or weights not summing to 100%. |
| 7 | `InvalidWeight` | A zero or above-100% weight, or a fee above the cap. |
| 8 | `MaxDepthExceeded` | Expanding would exceed `MAX_DEPTH`. |
| 9 | `SelfReference` | A composite lists itself as a child. |
| 10 | `TooManyChildren` | More than `MAX_CHILDREN`. |
| 11 | `CyclicReference` | The tree contains a cycle. |
| 12 | `InvalidAmount` | A negative amount was supplied. |
| 13 | `MathOverflow` | An `i128` step overflowed. |

## Events

- `init()` → `admin`
- `comp_add(address)` → `fee_bps`
- `comp_set(address)` → `child_count`

## Tests

```bash
cargo test -p perigee-strategy-composability
```

Covers registration and validation, flat and recursive resolution, cycle and
depth guards, and layered fee composition.
