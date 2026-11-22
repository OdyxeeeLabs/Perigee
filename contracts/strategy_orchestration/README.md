# Perigee Strategy Orchestration

Automated strategy rotation and cross-chain yield deployment.

Closes [#659](https://github.com/OdyxeeeLabs/Perigee/issues/659) and
[#656](https://github.com/OdyxeeeLabs/Perigee/issues/656).

## The problem this solves

Rotation and cross-chain deployment are the two ways a vault moves capital
into a strategy, and both fail in the same quiet way: the decision is taken
off-chain and the result is written down afterwards, so by the time anything
disagrees the capital is already committed. These two contracts pull the
*decision* on-chain.

## RotationEngine (#659)

A vault stores its intended allocation with `set_target_weights`. Anyone can
then call `plan_rotation` with the observed allocation and get back a
`RotationPlan`: the legs that are worth moving, prioritised by drift, capped by
`max_legs`.

### The gas saving is the cap

A rotation that moves every strategy every cycle pays for legs that do not
matter. `max_legs` bounds how many strategies move in one cycle, so only the
largest drifts are rotated and the rest wait for the next cycle. The plan
reports `legs_skipped` so the caller can see what was deferred.

### The threshold means small deviations cost nothing

`drift_threshold_bps` means a position has to wander by at least that much
before it is worth a transaction. A vault that rebalances to the exact basis
point pays its entire return in gas; one that lets positions wander within a
threshold pays nothing until the drift actually justifies a transaction.

### Dust is never rotated

`min_leg_bps` filters out legs where both sides are below the floor. Moving a
dust position costs more in gas than the position is worth.

### Slippage is enforced on-chain

`record_rotation` is called by the vault with the amount it intended to move
and the amount it actually received, and reverts if the difference exceeds the
policy's `slippage_bps`. The check happens after the move, so a vault that
cannot honour its own slippage limit cannot record a rotation it should not
have made.

## CrossChainYield (#656)

A strategy on another chain is not a strategy this chain can call directly.
The vault holds the deposit token here, and deploying it remotely means
locking it into a bridge and having it re-minted on the other side. That
`send`/`recall` round trip is exactly the interface this contract drives.

### The bridge interface

```rust,ignore
pub trait Bridge {
    fn send(env: Env, token: Address, amount: i128, chain_id: u32, recipient: Address) -> u64;
    fn recall(env: Env, chain_id: u32, amount: i128) -> u64;
}
```

`send` locks the token on this chain and instructs it to be minted on the
destination; `recall` burns the remote tokens and unlocks them here. Both are
the whole of the bridge surface the vault needs, so a Wormhole or LayerZero
integration is a contract that implements this trait.

### Deployment is capped per chain

`add_chain` registers a chain with its bridge, remote vault and
`max_allocation_bps`. `deploy` checks that `confirmed + pending + amount`
stays within that fraction of the total book the admin reports, so a bridge
failure cannot strand more than the cap allows.

### Settlement is relayer-confirmed

`deploy` moves funds into `pending`. Only the relayer can call `confirm`, which
releases funds from pending into confirmed. The state therefore changes only
when the bridge's message has actually been processed on the remote chain —
not when the vault hopes it has been.

### Recalls reduce the confirmed balance

`withdraw` burns the remote tokens and unlocks them here, reducing the
confirmed balance by exactly the amount recalled.

## Errors

| Code | Variant | Meaning |
| --- | --- | --- |
| 1 | `NotInitialized` | Used before `initialize`. |
| 2 | `AlreadyInitialized` | `initialize` called twice. |
| 3 | `Unauthorized` | Caller is not the admin. |
| 4 | `PolicyNotSet` | The vault has no rotation policy. |
| 5 | `TargetsNotSet` | The vault has no target weights. |
| 6 | `InvalidPolicy` | A policy field was out of range. |
| 7 | `InvalidTargets` | Empty targets or weights not summing to 100%. |
| 8 | `DuplicateStrategy` | A target weight repeats a strategy. |
| 9 | `TooManyStrategies` | More targets than `MAX_STRATEGIES`. |
| 10 | `SlippageExceeded` | Received less than the slippage limit allows. |
| 11 | `InvalidAmount` | A non-positive amount was supplied. |
| 12 | `ChainNotRegistered` | The chain is not registered. |
| 13 | `ChainAlreadyRegistered` | The chain is already registered. |
| 14 | `ChainInactive` | The chain has been deactivated. |
| 15 | `CapExceeded` | The deployment would exceed the chain's cap. |
| 16 | `NothingPending` | Nothing to confirm, or nothing confirmed to withdraw. |
| 17 | `TooManyChains` | More chains than `MAX_CHAINS`. |
| 18 | `MathOverflow` | An `i128` step overflowed. |

## Events

RotationEngine:

- `init()` → `admin`
- `policy(vault)` → `(drift_threshold_bps, max_legs)`
- `targets(vault)` → `count`
- `rotated(vault)` → `(from, to, amount, received)`

CrossChainYield:

- `init()` → `(token, relayer)`
- `managed()` → `value`
- `chain_add(chain_id)` → `(bridge, max_allocation_bps)`
- `chain_set(chain_id)` → `active`
- `deploy(chain_id)` → `(amount, message_id)`
- `confirm(chain_id)` → `amount`
- `withdraw(chain_id)` → `(amount, message_id)`

## Tests

```bash
cargo test -p perigee-strategy-orchestration
```

Covers policy and target validation, drift-based planning (above and below the
threshold), dust filtering, the `max_legs` cap, the slippage gate, chain
registration and deactivation, bridge-driven deployments within and beyond the
cap, relayer-confirmed settlement, and recalls.
