# Perigee Timelock Controller

Schedules arbitrary contract calls for execution after a configurable minimum
delay.

## Roles

- **Admin** initializes the controller, configures the minimum delay and
  authorities, and transfers administration.
- **Proposer** queues an invocation (target, function, arguments, and 32-byte
  salt).
- **Executor** executes a queued invocation once its `ready_at` timestamp is
  reached.
- **Canceller** cancels an operation while it is pending.

Each role requires its own address authorization. A repeated operation ID is
rejected while pending, executing, or completed. A canceled operation may be
queued again with the same arguments and salt.

## Delay and execution

The delay is measured in seconds using the Soroban ledger timestamp. Queueing
computes `ready_at = current_timestamp + min_delay` with overflow checking.
The configured delay must be between 1 second and 90 days; this bound ensures
the queued operation record stays within Soroban's persistent-storage TTL.
Execution marks the operation as executing before making the contract call,
preventing reentrant execution; the target call and status update are atomic.
If the target invocation fails, the transaction rolls back and the operation
remains pending.

Only the admin can change the delay or role addresses; these configuration
changes are immediate and emit events. The admin key is therefore part of the
controller's trust boundary. For stronger governance, configure the admin as a
multisig/governance contract or queue changes through the deployment's
governance process.

The target call is invoked with the controller contract as its caller. Target
contracts should authorize the controller address or use compatible Soroban
contract authorization for calls that require authentication.

## Validation

```powershell
cargo test -p perigee-timelock-controller
```
