# Perigee Vault Guard

Circuit breaker, partial pause, and timelocked unpause.

Closes [#634](https://github.com/OdyxeeeLabs/Perigee/issues/634).

## Four controls, one guard

| Control | Entry point | Who | Timelock |
| --- | --- | --- | --- |
| Admin pause | `pause()` | admin | none on the way in |
| **Circuit breaker** | `trip()` | **guardian** | none, ever |
| Partial pause | `pause_operations(mask)` | admin | none |
| Timelocked unpause | `unpause()` | **anyone**, after the delay | `UNPAUSE_DELAY_LEDGERS` |

## Why the breaker has no timelock, and who holds it

**Pausing is reactive; unpausing is deliberate.** A timelock on the way in would
mean an exploit has to finish executing before anyone can stop it, which inverts
the purpose of the control. So the way in is immediate, and the delay is entirely
on the way out.

The breaker belongs to a **guardian**, a role deliberately outside admin:

- A compromised **admin** cannot use the one control with no timelock, so it cannot
  be used as a griefing vector and cannot quietly halt a healthy protocol.
- A compromised **guardian** *can* halt a vault. That is a bounded, recoverable
  annoyance; an unpaused exploit is unbounded.

And because unpausing is still timelocked and still emits an event, a guardian
cannot keep a vault halted indefinitely without the market noticing.
`a_trip_still_has_to_wait_out_the_unpause_delay` pins this.

`initialize` refuses when `admin == guardian`, so the two roles cannot collapse
into one by configuration.

## Why the unpause is not admin-gated

`unpause` requires only that the delay has elapsed — **anyone** may call it. A
timelock the admin can shorten is not a timelock.

What the delay actually buys is a **notification window**: users and watchers get
a full day (~17 280 ledgers at 5 s each) to see that something was paused, and to
react — migrate their position, revoke an approval, sound an alarm — before funds
move again. That is the same property a timelock provides on a chain where there
are no other channels, which is why the gate is time rather than identity.

## Partial pause

`pause_operations` takes a **bitmask** of operations rather than a boolean, so
withdrawals can be stopped while deposits continue, or a single risky operation
can be isolated:

| Bit | Operation |
| --- | --- |
| `OP_DEPOSIT` | Deposits |
| `OP_WITHDRAW` | Withdrawals |
| `OP_ROTATE` | Strategy rotation |
| `OP_FEE_CHANGE` | Fee schedule changes |
| `OP_UPGRADE` | Contract upgrades |

Bits outside `OP_ALL` are **rejected**. Without that check a typo such as
`OP_DEPOSIT | (1 << 9)` would store a bit that no operation reads, and the pause
would appear to cover more than it does.

This matters specifically during an incident. A deposit-drain exploit frequently
does not touch withdrawals, and pausing everything locks legitimate users out of
their funds — which converts a protocol problem into a custody problem.

`pause` (the global override) takes precedence over the mask, so a full halt does
stop everything even if the mask says otherwise.

Individual operations can be resumed without waiting: only lifting a *global*
pause stops every user, and only that is timelocked.

## The trip flag outlives the pause

After a timelocked unpause, `is_tripped()` remains true. The flag is what tells
anyone reading the state that this vault did not pause for a routine reason, and
`reset_trip` is what clears it — only the guardian, only once the pause has
actually been lifted. So the incident cannot be closed while it is still in
progress, and it cannot be erased before the pause ends either.

## Pause-state matrix (#861)

`is_paused(operation)` is the single source of truth, and the tests walk every
combination rather than sampling:

| State | Deposit | Withdraw | Rotate | FeeChange | Upgrade |
| --- | --- | --- | --- | --- | --- |
| Nothing paused | open | open | open | open | open |
| Partial mask | blocked only if its bit is set | blocked only if its bit is set | same | same | same |
| Global pause / trip | blocked | blocked | blocked | blocked | blocked |
| Global pause lifted | back to the partial mask | back to the partial mask | same | same | same |

There is **no** emergency exit that bypasses a global pause: while `pause()` or
`trip()` is in force, withdrawals are blocked like everything else. The only way
to keep withdrawals available during an incident is a *partial* pause that leaves
`OP_WITHDRAW` out of the mask. A global pause does not clear the partial mask, so
lifting it restores whatever partial mask was in place.

## Unpause timing (#862)

- The delay is measured in **ledger sequence** from `paused_at`, not wall-clock.
- `unpause` succeeds when `current_sequence - paused_at >= UNPAUSE_DELAY_LEDGERS`
  (inclusive): one ledger early fails, the exact threshold succeeds.
- A failed `unpause` writes nothing, so repeated early calls cannot shorten the
  delay and the pause stays in force.
- This guard has no separate "schedule" step and no cancel/replace: the pause
  itself starts the clock. The `pause` event carries the start ledger and the
  `unpause` event carries the elapsed ledgers, which together record both ends.

## Errors

| Code | Variant | Meaning |
| --- | --- | --- |
| 1 | `NotInitialized` | Used before `initialize`. |
| 2 | `AlreadyInitialized` | `initialize` called more than once. |
| 3 | `Unauthorized` | Caller is not the admin, or `admin == guardian` at init. |
| 4 | `AlreadyPaused` | Already globally paused, or the mask adds nothing. |
| 5 | `NotPaused` | Nothing to unpause, or the trip flag was not set. |
| 6 | `UnpauseTooEarly` | The delay has not elapsed. |
| 7 | `NotGuardian` | Caller is not the guardian. |
| 8 | `UnknownOperation` | The mask contains bits outside `OP_ALL`. |
| 9 | `OperationNotPaused` | The mask names operations that are not paused. |
| 10 | `AlreadyTripped` | The breaker was already tripped. |

## Events

- `init(admin)` → `guardian`
- `pause()` → `paused_at_ledger`
- `pause_ops()` → `paused_mask`
- `resume_ops()` → `paused_mask`
- `unpause()` → `elapsed_ledgers`
- `trip(guardian)` → `paused_at_ledger`
- `trip_reset()` → `guardian`

## Tests

```bash
cargo test -p perigee-vault-guard
```

Covers the role separation (including that admin cannot trip the breaker and that
the two roles cannot be the same address), admin pause, partial pause and mask
validation, mask accumulation and resume, the global override, the timelock in
both directions plus that anyone may lift it once elapsed, breaker behaviour
including that the trip flag survives the unpause, and that operation bits are
distinct and within `OP_ALL`.