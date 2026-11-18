# Perigee Cross-Chain Governance — message passing, remote execution, state sync

> Implements [OdyxeeeLabs/Perigee#650](https://github.com/OdyxeeeLabs/Perigee/issues/650)
> — *"Add cross-chain governance: message passing (LayerZero/Wormhole), remote execution, state sync"*
> (source: `contract.md` § Access Control & Governance, [#20](https://github.com/OdyxeeeLabs/Perigee/issues/20)).

## Overview

Cross-chain governance enables Perigee to coordinate governance decisions, execute proposals across multiple blockchain networks, and maintain synchronized governance state between the hub (Stellar/Soroban) and remote spoke chains.

Rather than relying on ad-hoc off-chain multisigs or fragmented governance silos, this contract formalizes:
1. **Multi-bridge message passing** (LayerZero, Wormhole, and Native protocol wire formats).
2. **Secure remote execution** with replay protection, sender authorization, and optional timelock delays.
3. **Bidirectional state synchronization** for proposals, voting snapshots, protocol parameters, and Merkle state roots.

## Architecture

```text
               ┌────────────────────────────────────────────────────────┐
               │              Cross-Chain Governance Hub                 │
               │               (perigee-cross-chain-gov)                │
               └──────────────────────────┬─────────────────────────────┘
                                          │
                  ┌───────────────────────┼────────────────────────┐
                  ▼                       ▼                        ▼
         ┌─────────────────┐     ┌─────────────────┐      ┌─────────────────┐
         │ Message Passing │     │ Remote Execute  │      │   State Sync    │
         │ (LZ / WH / Nat) │     │ (Immediate/Time)│      │(Props/Roots/Prm)│
         └─────────────────┘     └─────────────────┘      └─────────────────┘
```

## Features

### 1. Message Passing (LayerZero / Wormhole / Native)

The contract supports pluggable bridge wire protocols:

- **LayerZero (`LZ_GOV_V1`)**: Validates endpoint IDs (`src_eid`, `dst_eid`), nonce, and packet payload hash.
- **Wormhole (`WH_GOV_V1`)**: Validates emitter chain, guardian quorum constraints, sequence numbers, and payload hash.
- **Native (`NATIVE_GOV_V1`)**: Native domain-separated message hash with timestamp checks.

Anti-replay protection is strictly enforced by:
- Storing processed canonical message hashes in `ProcessedMessage(BytesN<32>)`.
- Tracking monotonic inbound sequence numbers per remote chain `InboundSequence(source_chain_id)`.
- Gating message ingestion through an authorized sender whitelist `TrustedSender(source_chain_id, sender)`.

### 2. Remote Execution

Authorized remote governance messages can execute privileged calls on local Soroban contracts:

- **Immediate Execution**: When `min_execution_delay == 0`, valid messages invoke target contracts directly via `env.invoke_contract(&target, &function, args)`.
- **Timelocked Queued Execution**: When `min_execution_delay > 0`, incoming actions transition to `Queued` status until the delay expires (`execute_after`). Anyone can then trigger `execute_queued(execution_id)` once ready.
- **Cancellation**: Admin can cancel a queued execution prior to execution.
- **Outbound Dispatch**: Admin/governance can dispatch remote execution payloads to spoke chains (`dispatch_remote_execution`), producing indexed relayer events and incrementing outbound sequence nonces.

### 3. State Sync

Governance state can be synchronized and queried cross-chain:

| State Item | Description | Functions |
|---|---|---|
| **Proposals** | Sync proposal outcome, for/against votes, and closed status | `sync_proposal_state`, `get_synced_proposal`, `broadcast_proposal_state` |
| **Voting Power** | Sync voter voting units and snapshot ID across chains | `sync_voting_power`, `get_synced_voting_power`, `broadcast_voting_power` |
| **Parameters** | Sync protocol-wide fee, risk, and configuration values | `sync_parameter`, `get_synced_parameter`, `broadcast_parameter` |
| **State Roots** | Sync Merkle state roots across epochs and heights | `sync_state_root`, `get_synced_state_root`, `get_latest_state_root`, `broadcast_state_root` |

## Emergency Controls

- **Circuit Breaker (`set_paused`)**: Instantly freeze cross-chain message processing during an active bridge compromise or exploit.
- **Admin Rotation (`transfer_admin`)**: Rotate administrative authority.
- **Protocol Toggles (`set_bridge_protocol`)**: Disable a compromised bridge wire format while keeping remaining bridges functional.

## Execution status model and timeouts (#873)

All times are **ledger timestamps in seconds** (`env.ledger().timestamp()`), not
ledger sequence numbers. `min_execution_delay` is therefore a number of seconds.

| From | To | How |
| --- | --- | --- |
| *(none)* | `Executed` | `receive_and_execute` with `min_execution_delay == 0` |
| *(none)* | `Queued` | `receive_and_execute` with a delay; `execute_after = now + delay` |
| `Queued` | `Executed` | `execute_queued` when `now >= execute_after` (inclusive) |
| `Queued` | `Cancelled` | `cancel_queued` (admin) |

`Pending` and `Failed` exist in the enum but are **not written by this contract**:
a remote call that fails reverts the whole `execute_queued` invocation, so the
record is left `Queued` rather than marked `Failed`. That makes retry the same
operation as the first attempt — call `execute_queued` again.

- A retry always runs the **stored** `target`, `function` and `args`; none of them
  can be changed after the message is accepted.
- Re-delivering an `execution_id` through a new message is rejected with
  `ExecutionAlreadyExecuted` and reverts entirely, so it cannot overwrite the
  approved payload or consume a sequence number.
- `Executed` and `Cancelled` are terminal. A second `execute_queued` or
  `cancel_queued` fails with `ExecutionAlreadyExecuted` (the error name covers
  "already finished" in either direction).
- There is no automatic expiry: a queued execution whose target never recovers
  stays `Queued` until it succeeds or the admin cancels it.

## Replay protection (#872)

Inbound messages are checked in this order, and a rejected message writes
nothing: pause → destination chain → action type → bridge enabled → proof
header → trusted sender (per source chain) → processed-hash → strictly
increasing sequence per source chain. Sequence gaps are allowed, so a late
message with a lower sequence than one already accepted is rejected as
`InvalidSequence`.
