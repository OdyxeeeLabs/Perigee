# Perigee Access Control — per-function permissions, wildcards, batch updates

> Implements [OdyxeeeLabs/Perigee#647](https://github.com/OdyxeeeLabs/Perigee/issues/647)
> — *"Implement access control lists: per-function permissions, wildcard support, batch updates"*
> (source: `contract.md` § Access Control & Governance, [#17](https://github.com/OdyxeeeLabs/Perigee/issues/17)).

## Overview

Most contracts in this workspace gate privileged calls with a single
`if caller != admin { return Err(Unauthorized) }`. That answers *is this the
admin?* and nothing else. Real protocols need finer answers:

- can this keeper call `harvest` but not `withdraw`?
- can this auditor read `balances` but not move funds?
- can a whole team be granted `rebalance` in one transaction instead of six?

This contract is the shared answer. Any contract can point at an
`AccessControl` instance and ask `has_permission(caller, selector)` at the top
of every privileged entry point.

## Model

### Three layers of authority

| Layer | Who manages it | What it is |
|---|---|---|
| **Root admin** | itself, via `transfer_admin` | Owns the ACL: direct grants, roles, wildcards, config. |
| **Roles** | root admin *or* the role's own admin | Named bundles of selectors, granted to addresses. |
| **Direct grants** | root admin only | A selector attached straight to one address. |

A caller is authorized for `selector` when any of these holds — checked
cheapest-first, because `has_permission` is on the hot path:

```text
root admin?                  -> allowed
direct grant on `selector`   -> allowed
direct grant on `*`          -> allowed
direct grant on `vault.*`    -> allowed   (registered namespace wildcard)
member of a role granting it -> allowed   (direct, `*`, or namespace)
otherwise                    -> denied
```

The root admin is stored **outside** the grant tables. No grant, role, batch,
or renounce operation can remove it, so a mis-issued batch can never lock the
ACL out. The only way it changes hands is the explicit, audited
`transfer_admin`.

### Standard Perigee roles

`setup_protocol_roles` provisions five canonical roles in one root-admin-only,
all-or-nothing call. The operational `admin` role is distinct from the ACL's
root admin. All five role definitions remain root-admin controlled, so an
operational role holder cannot redefine its own permissions.

| Role | Selectors |
|---|---|
| `owner` | `vault.deposit`, `vault.withdraw`, `vault.transfer`, `vault.configure` |
| `admin` | `roles.manage`, `vault.pause`, `vault.unpause`, `strategy.configure`, `strategy.approve`, `fees.configure`, `keeper.configure`, `emergency.pause` |
| `strategist` | `strategy.rebalance`, `strategy.rotate`, `strategy.harvest` |
| `keeper` | `strategy.harvest`, `strategy.rebalance`, `oracle.update` |
| `emergency` | `emergency.pause`, `emergency.cancel`, `vault.pause`, `strategy.pause` |

These are exact selectors, not wildcards. Host contracts use the usual
`has_permission` / `require_permission` guard with the same selector strings.
The emergency role intentionally has no unpause permission; recovery is
reserved for admin/root-admin policy. `has_protocol_role` queries membership,
and the existing `grant_role` / `revoke_role` methods let the root admin rotate
members without changing role permissions.

### Wildcards

| Selector | Kind | Matches |
|---|---|---|
| `deposit` | exact | only `deposit` |
| `*` | universal | every selector |
| `vault.*` | namespace | every selector starting with `vault.` |

A trailing `*` is the only legal position for a star, and there can be only
one: `de*` and `vault.*` are accepted; `de*pos`, `*x` and `a*b*` are rejected
with `InvalidSelector`. That keeps matching to "exact, universal, or
starts-with" with no overlapping cases.

Namespace wildcards must be **registered** before they can be granted:

```rust
client.register_wildcard(&admin, &str_symbol(&env, "vault.*"));   // once
client.grant(&admin, &keeper, &str_symbol(&env, "vault.*"));     // many
```

Registration is not bookkeeping. Matching an unregistered pattern would mean
probing every selector ever granted to an address; the registry caps the
wildcard probe at `max_wildcards` (8 by default), giving a fixed, predictable
gas cost no matter how many grants exist. It also lets `unregister_wildcard`
refuse to orphan live permissions — a per-pattern usage counter tracks how many
direct grants and role selectors still reference the pattern, and removal is
rejected while that count is non-zero.

### Precedence (#870)

The ACL is **purely additive**. There are no deny entries, so there is nothing
for an exact selector to "override": a call is allowed when *any* of the rules in
the authorization list above matches, and denied when none does.

| Situation | Result |
|---|---|
| No matching rule | denied |
| Exact grant only | allowed for that selector only (not siblings, not wildcards) |
| Namespace wildcard `vault.*` | allowed for selectors starting with `vault.` |
| Universal `*` | allowed for every well-formed selector |
| Exact **and** wildcard both held | allowed; revoking the exact grant does **not** remove access the wildcard still gives |
| Held directly **and** through a role | allowed until *both* are removed |
| Malformed selector (`""`, `a*b`, `*x`) | denied for everyone except the root admin, even with `*` |
| Root admin | always allowed |

To take access away from someone holding a wildcard, revoke the wildcard
itself; narrowing it with an exact rule is not supported. Rules for different
principals never interact.

### Batch updates

| Entry point | Semantics |
|---|---|
| `batch_grant` | additive; entries already held are skipped |
| `batch_revoke` | subtractive; entries not held are skipped |
| `batch_set_members` | replaces the role's **whole** membership |
| `batch_set_permissions` | replaces the role's **whole** selector set |

Batches are all-or-nothing: Soroban rolls the invocation back on error, so a
batch that trips a limit half way through leaves no partial grant behind. Each
batch is additionally capped by `max_batch_size`. The batch operations return
the number of entries actually applied, which makes them safe to re-run.

### Events

Exactly **one event per state change**. Idempotent calls that change nothing
emit nothing, so an indexer never has to distinguish "did nothing" from
"failed" by guessing. Every event carries `batch_size`, so a 32-way bulk update
is distinguishable from a one-off edit without re-reading state.

All events use topics `("acl", <action>)`:

| Action topic | Payload | Emitted by |
|---|---|---|
| `grant` | `GrantEvent` | grant / revoke / renounce / batch_grant / batch_revoke |
| `role_perm` | `RolePermissionEvent` | role selector changes |
| `role_memb` | `RoleMemberEvent` | role membership changes |
| `role_new` | `RoleCreatedEvent` | `create_role` |
| `role_adm` | `RoleAdminEvent` | `set_role_admin` |
| `role_del` | `RoleDestroyedEvent` | `destroy_role` |
| `wildcard` | `WildcardEvent` | register / unregister a namespace wildcard |
| `admin` | `AdminTransferredEvent` | `transfer_admin` |
| `lockdown` | `LockdownEvent` | `set_lockdown` |
| `config` | `ConfigEvent` | `initialize` / `set_config` |

### Lockdown

`set_lockdown(true)` freezes every mutating operation — the incident-response
lever for when a leaked key is granting itself more access. Queries keep
working. Three things deliberately survive:

- `set_lockdown` itself, or the freeze could never be lifted;
- `transfer_admin`, so a suspected-compromised key can be rotated mid-incident
  without first reopening the protocol to writes (gating it would buy no
  security: a compromised root key can simply unfreeze itself first);
- the self-service `renounce`, `renounce_all` and `renounce_role`, so an address
  can always give up its own access while the protocol is frozen.

### Limits

Every list is bounded by `AclConfig`, settable at init and via `set_config`:

| Field | Default | Bounds |
|---|---|---|
| `max_batch_size` | 32 | entries per batch call |
| `max_grants_per_address` | 64 | direct grants per address |
| `max_roles` | 32 | roles that may exist |
| `max_permissions_per_role` | 32 | selectors per role |
| `max_members_per_role` | 64 | members per role |
| `max_roles_per_address` | 8 | roles per address |
| `max_wildcards` | 8 | registered namespace wildcards |

Grants and memberships are reverse-indexed so `grants_of` and `renounce_all`
are cheap; an unbounded enumeration index would be an unbounded storage
liability, which is what these bounds exist to prevent.

## Usage

```rust
use access_control::{AccessControlClient, AclConfig};
use soroban_sdk::{vec, Address, Env, String, Vec};

let env = Env::default();
let admin = Address::generate(&env);
let id = env.register(AccessControl, ());
let acl = AccessControlClient::new(&env, &id);

acl.initialize(&admin, &AclConfig::default_config());

// A role with its own admin, so membership churn needs no root admin.
acl.create_role(&admin, &String::from_str(&env, "keepers"), &team_lead);
acl.grant_role_permission(
    &team_lead,
    &String::from_str(&env, "keepers"),
    &String::from_str(&env, "harvest"),
);
acl.grant_role(&team_lead, &String::from_str(&env, "keepers"), &keeper);

// Onboard a whole team in one transaction.
acl.batch_set_members(
    &team_lead,
    &String::from_str(&env, "keepers"),
    &vec![&env, keeper, alice, bob],
);

// The guard a host contract calls.
acl.require_permission(&keeper, &String::from_str(&env, "harvest"))?;
```

### As a guard in a host contract

```rust
pub fn harvest(env: Env, caller: Address, vault: Address) -> Result<(), Error> {
    let acl: Address = env.storage().instance().get(&DataKey::Acl).unwrap();
    let sel = String::from_str(&env, "harvest");
    access_control::AccessControlClient::new(&env, &acl).require_permission(&caller, &sel)?;
    // …the caller's ACL entry point has already required `caller.require_auth()`
    // or an equivalent signature, so this is an authorization check only.
    // …actual work
}
```

## Errors

| Code | Variant | Meaning |
|---|---|---|
| 1 | `NotInitialized` | The contract has not been initialized. |
| 2 | `AlreadyInitialized` | `initialize` called twice. |
| 3 | `Unauthorized` | Caller is not authorized for this entry point. |
| 4 | `Locked` | Lockdown is engaged. |
| 5 | `LimitExceeded` | A configured bound is exhausted. |
| 6 | `BatchTooLarge` | Batch exceeds `max_batch_size`. |
| 7 | `EmptyBatch` | Batch carries no entries. |
| 8 | `InvalidSelector` | Empty selector, or a `*` somewhere other than the end. |
| 9 | `SelectorTooLong` | Selector exceeds 64 bytes. |
| 10 | `InvalidRole` | Role name is empty, over-long, or contains `*`. |
| 11 | `RoleExists` | Role already exists. |
| 12 | `RoleNotFound` | Role does not exist. |
| 13 | `UnregisteredWildcard` | Namespace wildcard granted before registration. |
| 14 | `WildcardInUse` | Pattern is still referenced by a live permission. |
| 15 | `WildcardNotFound` | Pattern is not registered. |
| 16 | `ProtocolRolesAlreadyConfigured` | The canonical Perigee roles have already been provisioned. |

## Testing

```bash
cargo test -p perigee-access-control
cargo build -p perigee-access-control --target wasm32-unknown-unknown --release
```

71 tests cover initialization, per-function grants, standard Perigee roles,
exact and wildcard
matching, role delegation, all four batch entry points, every limit, lockdown,
self-service renounce, admin rotation, index consistency under mixed
operations, and the event invariant.

> Note for contributors: the Soroban test host returns only the events of the
> **most recent** top-level invocation, so event assertions must be read
> immediately after the call under test. `count_events` in `src/test.rs`
> documents this; tests needing a running total accumulate it themselves.