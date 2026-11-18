#![no_std]

//! # Perigee Access Control Lists — per-function permissions with wildcard support
//!
//! Closes OdyxeeeLabs/Perigee#647
//!
//! Source: contract.md § Access Control & Governance (#17)
//!
//! Every other contract in this workspace authorizes privileged calls with a
//! single `if caller != admin { return Err(Unauthorized) }`. That answers one
//! question — *is this the admin?* — and nothing more. In practice a protocol
//! needs to answer several more: can this keeper call `harvest` but not
//! `withdraw`? Can an auditor read `balances` but not move funds? Can a whole
//! team be granted `rebalance` in one transaction instead of six?
//!
//! This contract is the shared answer to those questions.
//!
//! ## Three layers of authority
//!
//! 1. **The root admin** ([`AccessControl::admin`]) is a single address that
//!    owns the ACL itself: it manages direct grants, roles, registered
//!    wildcards and the configuration. It is stored *outside* the grant tables,
//!    so **no ACL operation can ever remove it** — the contract cannot be
//!    locked out by a mis-issued batch. Handing over the keys is the explicit,
//!    audited [`AccessControl::transfer_admin`].
//! 2. **Roles** are named bundles of selectors. A role has its own admin, so
//!    day-to-day membership changes (who is on the `keepers` team this week)
//!    do not need the root admin, while *defining* the role does.
//! 3. **Direct grants** attach a selector straight to an address, bypassing
//!    roles entirely. Root-admin-only, for one-offs and emergencies.
//!
//! A caller is authorized for `selector` if any of the following holds. The
//! checks are ordered cheapest-first because [`AccessControl::has_permission`]
//! sits on the hot path of every guarded entry point:
//!
//! ```text
//! root admin?                  -> allowed
//! direct grant on `selector`   -> allowed
//! direct grant on `*`          -> allowed
//! direct grant on `vault.*`    -> allowed   (registered namespace wildcard)
//! member of a role granting it -> allowed   (direct, `*`, or namespace)
//! otherwise                    -> denied
//! ```
//!
//! ## Wildcards
//!
//! Selectors follow the rules documented in [`selector`]. The universal `*` is
//! always live; namespace wildcards such as `vault.*` must be registered first
//! via [`AccessControl::register_wildcard`].
//!
//! Registration is not bookkeeping for its own sake. Matching an unregistered
//! pattern would mean probing *every* selector ever granted to an address, so
//! the registry bounds the wildcard probe to `max_wildcards` entries (8 by
//! default) — a fixed, gas-predictable cost no matter how many grants exist.
//! It also lets [`AccessControl::unregister_wildcard`] refuse to orphan live
//! permissions, via a per-pattern usage counter.
//!
//! ## Batch updates
//!
//! Onboarding a keeper team is one transaction, not forty. Four batch entry
//! points cover the realistic shapes:
//!
//! | Entry point                                 | Semantics                            |
//! |---------------------------------------------|--------------------------------------|
//! | [`AccessControl::batch_grant`]               | additive; existing grants are skipped |
//! | [`AccessControl::batch_revoke`]              | subtractive; absent grants are skipped |
//! | [`AccessControl::batch_set_members`]         | replaces the role's whole membership |
//! | [`AccessControl::batch_set_permissions`]     | replaces the role's whole selector set |
//!
//! Batches are **all-or-nothing**: Soroban rolls the whole invocation back on
//! error, so a batch that trips a limit half way through leaves no partial
//! grant behind. Each batch is additionally capped by `max_batch_size` to keep
//! the worst-case ledger footprint bounded.
//!
//! ## Event invariant
//!
//! **Exactly one event is emitted per state change.** Idempotent calls that
//! change nothing emit nothing. Batch events carry the batch length in
//! `batch_size` so an indexer can tell a 32-way bulk update from a one-off
//! edit without re-reading state.
//!
//! ## Lockdown
//!
//! [`AccessControl::set_lockdown`] freezes every mutating operation — the
//! incident-response lever for when a leaked key is granting itself more
//! access. A few operations deliberately survive it:
//!
//! - [`AccessControl::set_lockdown`] itself, or the freeze could never be
//!   lifted;
//! - [`AccessControl::transfer_admin`], so an honest admin can rotate a
//!   suspected-compromised key mid-incident without unfreezing first;
//! - the self-service [`AccessControl::renounce`],
//!   [`AccessControl::renounce_all`] and [`AccessControl::renounce_role`], so an
//!   address can always give up its own access while the protocol is frozen.

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env, String, Vec,
};

mod selector;

#[cfg(test)]
mod test_precedence;

#[cfg(test)]
mod test;

pub use selector::{SelectorKind, MAX_SELECTOR_LEN, WILDCARD};

// ── Errors ────────────────────────────────────────────────────────────────────

/// Errors returned by the access control list.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    /// The contract has not been initialized yet.
    NotInitialized = 1,
    /// `initialize` was called more than once.
    AlreadyInitialized = 2,
    /// The caller is not authorized for this entry point.
    Unauthorized = 3,
    /// The ACL is in lockdown; only release and self-service renounce work.
    Locked = 4,
    /// A configured bound (grants, roles, members, wildcards) is exhausted.
    LimitExceeded = 5,
    /// The batch is larger than `max_batch_size`.
    BatchTooLarge = 6,
    /// The batch carries no entries.
    EmptyBatch = 7,
    /// A selector is empty, or carries a `*` somewhere other than the end.
    InvalidSelector = 8,
    /// A selector exceeds [`MAX_SELECTOR_LEN`].
    SelectorTooLong = 9,
    /// A role name is empty, over-long, or contains a `*`.
    InvalidRole = 10,
    /// The role already exists.
    RoleExists = 11,
    /// The role does not exist.
    RoleNotFound = 12,
    /// A namespace wildcard was granted without being registered first.
    UnregisteredWildcard = 13,
    /// The wildcard is still referenced by live grants or role permissions.
    WildcardInUse = 14,
    /// The wildcard is not registered.
    WildcardNotFound = 15,
    /// The canonical protocol roles have already been provisioned.
    ProtocolRolesAlreadyConfigured = 16,
}

// ── Types ─────────────────────────────────────────────────────────────────────

/// Bounds applied to every list this contract maintains.
///
/// Each one exists to stop an unbounded write. The contract is enumerable —
/// direct grants and memberships are both reverse-indexed so
/// [`AccessControl::grants_of`] and [`AccessControl::renounce_all`] are cheap —
/// and an unbounded enumeration index is an unbounded storage liability. Raise
/// a bound with [`AccessControl::set_config`] only once the extra cost is
/// understood.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AclConfig {
    /// Maximum entries in a single batch call.
    pub max_batch_size: u32,
    /// Maximum direct grants held by a single address.
    pub max_grants_per_address: u32,
    /// Maximum roles that may exist at once.
    pub max_roles: u32,
    /// Maximum selectors attached to a single role.
    pub max_permissions_per_role: u32,
    /// Maximum members of a single role.
    pub max_members_per_role: u32,
    /// Maximum roles a single address may hold.
    pub max_roles_per_address: u32,
    /// Maximum registered namespace wildcards, i.e. the ceiling on one wildcard probe.
    pub max_wildcards: u32,
}

impl AclConfig {
    /// Conservative defaults sized for a production deployment.
    pub fn default_config() -> Self {
        AclConfig {
            max_batch_size: 32,
            max_grants_per_address: 64,
            max_roles: 32,
            max_permissions_per_role: 32,
            max_members_per_role: 64,
            max_roles_per_address: 8,
            max_wildcards: 8,
        }
    }
}

/// One `(address, selector)` pair inside a batch.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GrantEntry {
    /// Address the permission applies to.
    pub target: Address,
    /// Selector to grant or revoke.
    pub selector: String,
}

/// Summary of a role.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoleInfo {
    /// Role name.
    pub role: String,
    /// Address allowed to manage the role's members and selectors.
    pub admin: Address,
    /// Number of selectors attached to the role.
    pub permission_count: u32,
    /// Number of members of the role.
    pub member_count: u32,
}

/// Canonical Perigee operational roles provisioned by
/// [`AccessControl::setup_protocol_roles`].
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ProtocolRole {
    Owner,
    Admin,
    Strategist,
    Keeper,
    Emergency,
}

impl ProtocolRole {
    /// Stable role name used by the generic role and permission APIs.
    pub fn name(self, env: &Env) -> String {
        String::from_str(
            env,
            match self {
                ProtocolRole::Owner => "owner",
                ProtocolRole::Admin => "admin",
                ProtocolRole::Strategist => "strategist",
                ProtocolRole::Keeper => "keeper",
                ProtocolRole::Emergency => "emergency",
            },
        )
    }
}

/// Storage keys.
///
/// `Grant`, `RolePermission` and `RoleMember` are the authoritative membership
/// flags; the matching `Grants`, `Permissions` and `Members` keys are the
/// reverse indexes that make enumeration possible. Both are written by every
/// mutation helper, so they cannot drift.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Root admin, held outside the grant tables so ACL edits cannot remove it.
    Admin,
    /// [`AclConfig`].
    Config,
    /// Lockdown flag.
    Lockdown,
    /// Direct grant flag: `(address, selector)`.
    Grant(Address, String),
    /// Direct grants held by an address.
    Grants(Address),
    /// Role membership flag: `(role, address)`.
    RoleMember(String, Address),
    /// Members of a role.
    Members(String),
    /// Roles held by an address.
    Roles(Address),
    /// Role selector flag: `(role, selector)`.
    RolePermission(String, String),
    /// Selectors attached to a role.
    Permissions(String),
    /// Admin of a role.
    RoleAdmin(String),
    /// Every role that exists.
    RoleList,
    /// Whether the canonical Perigee operational roles have been provisioned.
    ProtocolRolesConfigured,
    /// Every registered namespace prefix, stored without the trailing `*`.
    Wildcards,
    /// Live grants referencing a namespace prefix, keyed by that prefix.
    WildcardUse(String),
}

// ── Events ────────────────────────────────────────────────────────────────────

/// Emitted when a direct grant is added or removed.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GrantEvent {
    pub actor: Address,
    pub target: Address,
    pub selector: String,
    /// `true` for a grant, `false` for a revoke.
    pub granted: bool,
    /// Entries in the calling batch, or 1 for a single edit.
    pub batch_size: u32,
}

/// Emitted when a role selector is added or removed.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RolePermissionEvent {
    pub actor: Address,
    pub role: String,
    pub selector: String,
    pub granted: bool,
    pub batch_size: u32,
}

/// Emitted when a role member is added or removed.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoleMemberEvent {
    pub actor: Address,
    pub role: String,
    pub member: Address,
    pub granted: bool,
    pub batch_size: u32,
}

/// Emitted when a role is created.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoleCreatedEvent {
    pub actor: Address,
    pub role: String,
    pub admin: Address,
}

/// Emitted when a role's admin changes.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoleAdminEvent {
    pub actor: Address,
    pub role: String,
    pub admin: Address,
}

/// Emitted when a role is destroyed.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoleDestroyedEvent {
    pub actor: Address,
    pub role: String,
    /// Members the destroy detached from the role.
    pub members_detached: u32,
    /// Selectors the destroy detached from the role.
    pub permissions_detached: u32,
}

/// Emitted when a namespace wildcard is registered or removed.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WildcardEvent {
    pub actor: Address,
    /// The full pattern, including the trailing `*`.
    pub pattern: String,
    /// `true` for registration, `false` for removal.
    pub registered: bool,
}

/// Emitted when the root admin changes.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminTransferredEvent {
    pub actor: Address,
    pub previous_admin: Address,
    pub new_admin: Address,
}

/// Emitted when lockdown is engaged or released.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LockdownEvent {
    pub actor: Address,
    pub active: bool,
}

/// Emitted when the bounds in [`AclConfig`] change.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigEvent {
    pub actor: Address,
    pub config: AclConfig,
}

const TOPIC: soroban_sdk::Symbol = symbol_short!("acl");

fn emit_grant(env: &Env, e: GrantEvent) {
    env.events().publish((TOPIC, symbol_short!("grant")), e);
}

fn emit_role_permission(env: &Env, e: RolePermissionEvent) {
    env.events().publish((TOPIC, symbol_short!("role_perm")), e);
}

fn emit_role_member(env: &Env, e: RoleMemberEvent) {
    env.events().publish((TOPIC, symbol_short!("role_memb")), e);
}

fn emit_role_created(env: &Env, e: RoleCreatedEvent) {
    env.events().publish((TOPIC, symbol_short!("role_new")), e);
}

fn emit_role_admin(env: &Env, e: RoleAdminEvent) {
    env.events().publish((TOPIC, symbol_short!("role_adm")), e);
}

fn emit_role_destroyed(env: &Env, e: RoleDestroyedEvent) {
    env.events().publish((TOPIC, symbol_short!("role_del")), e);
}

fn emit_wildcard(env: &Env, e: WildcardEvent) {
    env.events().publish((TOPIC, symbol_short!("wildcard")), e);
}

fn emit_admin_transferred(env: &Env, e: AdminTransferredEvent) {
    env.events().publish((TOPIC, symbol_short!("admin")), e);
}

fn emit_lockdown(env: &Env, e: LockdownEvent) {
    env.events().publish((TOPIC, symbol_short!("lockdown")), e);
}

fn emit_config(env: &Env, e: ConfigEvent) {
    env.events().publish((TOPIC, symbol_short!("config")), e);
}

// ── Storage helpers ───────────────────────────────────────────────────────────

fn read_admin(env: &Env) -> Result<Address, Error> {
    env.storage()
        .instance()
        .get(&DataKey::Admin)
        .ok_or(Error::NotInitialized)
}

fn is_root(env: &Env, who: &Address) -> bool {
    env.storage().instance().get::<_, Address>(&DataKey::Admin) == Some(who.clone())
}

/// Require the caller to be the root admin, and require their signature.
fn require_root(env: &Env, who: &Address) -> Result<(), Error> {
    if !is_root(env, who) {
        return Err(Error::Unauthorized);
    }
    who.require_auth();
    Ok(())
}

/// Require the caller to administer `role` — as the root admin, or as the
/// role's own admin. Both paths require a signature.
fn require_role_admin(env: &Env, who: &Address, role: &String) -> Result<(), Error> {
    if is_root(env, who) {
        who.require_auth();
        return Ok(());
    }
    let admin: Option<Address> = env
        .storage()
        .persistent()
        .get(&DataKey::RoleAdmin(role.clone()));
    match admin {
        Some(a) if a == *who => {
            who.require_auth();
            Ok(())
        }
        _ => Err(Error::Unauthorized),
    }
}

fn read_config(env: &Env) -> Result<AclConfig, Error> {
    env.storage()
        .instance()
        .get(&DataKey::Config)
        .ok_or(Error::NotInitialized)
}

fn is_locked(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&DataKey::Lockdown)
        .unwrap_or(false)
}

/// Gate every mutating operation on the lockdown flag.
fn require_writable(env: &Env) -> Result<(), Error> {
    if is_locked(env) {
        return Err(Error::Locked);
    }
    Ok(())
}

fn read_str_list(env: &Env, key: &DataKey) -> Vec<String> {
    env.storage()
        .persistent()
        .get(key)
        .unwrap_or_else(|| Vec::new(env))
}

fn write_str_list(env: &Env, key: &DataKey, list: &Vec<String>) {
    if list.is_empty() {
        env.storage().persistent().remove(key);
    } else {
        env.storage().persistent().set(key, list);
    }
}

fn read_addr_list(env: &Env, key: &DataKey) -> Vec<Address> {
    env.storage()
        .persistent()
        .get(key)
        .unwrap_or_else(|| Vec::new(env))
}

fn write_addr_list(env: &Env, key: &DataKey, list: &Vec<Address>) {
    if list.is_empty() {
        env.storage().persistent().remove(key);
    } else {
        env.storage().persistent().set(key, list);
    }
}

fn keep_strings_except(env: &Env, list: &Vec<String>, drop: &String) -> Vec<String> {
    let mut kept = Vec::new(env);
    for entry in list.iter() {
        if entry != *drop {
            kept.push_back(entry);
        }
    }
    kept
}

fn keep_addrs_except(env: &Env, list: &Vec<Address>, drop: &Address) -> Vec<Address> {
    let mut kept = Vec::new(env);
    for entry in list.iter() {
        if entry != *drop {
            kept.push_back(entry);
        }
    }
    kept
}

fn wildcard_use(env: &Env, prefix: &String) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::WildcardUse(prefix.clone()))
        .unwrap_or(0)
}

/// Adjust the live-reference counter for a namespace prefix. Counters saturate
/// at zero so a desynchronised decrement can never wrap into `u32::MAX` and
/// permanently block unregistration.
fn bump_wildcard_use(env: &Env, prefix: &String, delta: i64) {
    let key = DataKey::WildcardUse(prefix.clone());
    let current = wildcard_use(env, prefix) as i64;
    let next = (current + delta).max(0) as u32;
    if next == 0 {
        env.storage().persistent().remove(&key);
    } else {
        env.storage().persistent().set(&key, &next);
    }
}

fn wildcard_prefixes(env: &Env) -> Vec<String> {
    read_str_list(env, &DataKey::Wildcards)
}

fn role_exists(env: &Env, role: &String) -> bool {
    env.storage()
        .persistent()
        .has(&DataKey::RoleAdmin(role.clone()))
}

// ── Validation ────────────────────────────────────────────────────────────────

/// Validate a batch length against the configured ceiling.
fn check_batch_size(env: &Env, len: u32) -> Result<(), Error> {
    if len == 0 {
        return Err(Error::EmptyBatch);
    }
    if len > read_config(env)?.max_batch_size {
        return Err(Error::BatchTooLarge);
    }
    Ok(())
}

fn validate_role(role: &String) -> Result<(), Error> {
    if role.is_empty() || role.len() > MAX_SELECTOR_LEN {
        return Err(Error::InvalidRole);
    }
    // A `*` in a role name would be ambiguous between "role" and "selector".
    let mut buf = [0u8; MAX_SELECTOR_LEN as usize];
    let n = role.len() as usize;
    role.copy_into_slice(&mut buf[..n]);
    if buf[..n].contains(&b'*') {
        return Err(Error::InvalidRole);
    }
    Ok(())
}

/// Reject a selector the contract cannot act on, and confirm a namespace
/// wildcard has been registered before it may be stored.
fn validate_grantable(env: &Env, sel: &String) -> Result<SelectorKind, Error> {
    let kind = selector::classify(env, sel)?;
    if let SelectorKind::Namespace(prefix) = &kind {
        if !wildcard_prefixes(env).contains(prefix) {
            return Err(Error::UnregisteredWildcard);
        }
    }
    Ok(kind)
}

/// Keep the wildcard usage counter in step with a selector being stored or
/// dropped. `classify` can only fail for a selector that never passed
/// validation, so the `Exact` fallback simply means "not a wildcard".
fn track_wildcard_use(env: &Env, sel: &String, delta: i64) {
    if let Ok(SelectorKind::Namespace(prefix)) = selector::classify(env, sel) {
        bump_wildcard_use(env, &prefix, delta);
    }
}

// ── Permission matching ───────────────────────────────────────────────────────

/// Direct grants held by `who` that cover `sel`.
fn direct_matches(env: &Env, who: &Address, sel: &String) -> bool {
    if env
        .storage()
        .persistent()
        .has(&DataKey::Grant(who.clone(), sel.clone()))
    {
        return true;
    }

    let universal = DataKey::Grant(who.clone(), String::from_str(env, WILDCARD));
    if env.storage().persistent().has(&universal) {
        return true;
    }

    for prefix in wildcard_prefixes(env).iter() {
        if selector::is_prefix(&prefix, sel)
            && env.storage().persistent().has(&DataKey::Grant(
                who.clone(),
                selector::pattern(env, &prefix),
            ))
        {
            return true;
        }
    }

    false
}

/// Whether `role` grants `sel`.
fn role_matches(env: &Env, role: &String, sel: &String) -> bool {
    if env
        .storage()
        .persistent()
        .has(&DataKey::RolePermission(role.clone(), sel.clone()))
    {
        return true;
    }

    let universal = DataKey::RolePermission(role.clone(), String::from_str(env, WILDCARD));
    if env.storage().persistent().has(&universal) {
        return true;
    }

    for prefix in wildcard_prefixes(env).iter() {
        if selector::is_prefix(&prefix, sel)
            && env.storage().persistent().has(&DataKey::RolePermission(
                role.clone(),
                selector::pattern(env, &prefix),
            ))
        {
            return true;
        }
    }

    false
}

/// The core authorization decision, shared by the query and assert entry
/// points. `sel` is assumed to have been validated by the caller.
fn permits(env: &Env, caller: &Address, sel: &String) -> bool {
    if is_root(env, caller) {
        return true;
    }
    if selector::classify(env, sel).is_err() {
        // A malformed selector is denied rather than rejected: a bad argument
        // must never widen access.
        return false;
    }
    if direct_matches(env, caller, sel) {
        return true;
    }
    for role in read_str_list(env, &DataKey::Roles(caller.clone())).iter() {
        if role_matches(env, &role, sel) {
            return true;
        }
    }
    false
}

// ── Mutation primitives ───────────────────────────────────────────────────────

/// Add a direct grant. Returns `true` when the grant was newly created.
fn apply_grant(env: &Env, target: &Address, sel: &String) -> Result<bool, Error> {
    let key = DataKey::Grant(target.clone(), sel.clone());
    if env.storage().persistent().has(&key) {
        return Ok(false);
    }

    validate_grantable(env, sel)?;

    let cfg = read_config(env)?;
    let index_key = DataKey::Grants(target.clone());
    let mut index = read_str_list(env, &index_key);
    if index.len() >= cfg.max_grants_per_address {
        return Err(Error::LimitExceeded);
    }

    env.storage().persistent().set(&key, &true);
    index.push_back(sel.clone());
    write_str_list(env, &index_key, &index);
    track_wildcard_use(env, sel, 1);
    Ok(true)
}

/// Remove a direct grant. Returns `true` when a grant was actually removed.
fn apply_revoke(env: &Env, target: &Address, sel: &String) -> bool {
    let key = DataKey::Grant(target.clone(), sel.clone());
    if !env.storage().persistent().has(&key) {
        return false;
    }
    env.storage().persistent().remove(&key);

    let index_key = DataKey::Grants(target.clone());
    let index = read_str_list(env, &index_key);
    let kept = keep_strings_except(env, &index, sel);
    write_str_list(env, &index_key, &kept);

    track_wildcard_use(env, sel, -1);
    true
}

/// Attach `sel` to `role`. Returns `true` when newly attached.
fn apply_role_permission(env: &Env, role: &String, sel: &String) -> Result<bool, Error> {
    let key = DataKey::RolePermission(role.clone(), sel.clone());
    if env.storage().persistent().has(&key) {
        return Ok(false);
    }

    validate_grantable(env, sel)?;

    let cfg = read_config(env)?;
    let index_key = DataKey::Permissions(role.clone());
    let mut index = read_str_list(env, &index_key);
    if index.len() >= cfg.max_permissions_per_role {
        return Err(Error::LimitExceeded);
    }

    env.storage().persistent().set(&key, &true);
    index.push_back(sel.clone());
    write_str_list(env, &index_key, &index);
    track_wildcard_use(env, sel, 1);
    Ok(true)
}

/// Detach `sel` from `role`. Returns `true` when it was attached.
fn apply_role_revoke(env: &Env, role: &String, sel: &String) -> bool {
    let key = DataKey::RolePermission(role.clone(), sel.clone());
    if !env.storage().persistent().has(&key) {
        return false;
    }
    env.storage().persistent().remove(&key);

    let index_key = DataKey::Permissions(role.clone());
    let index = read_str_list(env, &index_key);
    let kept = keep_strings_except(env, &index, sel);
    write_str_list(env, &index_key, &kept);

    track_wildcard_use(env, sel, -1);
    true
}

/// Add `member` to `role`, maintaining both the member index and the reverse
/// per-address role index. Returns `true` when newly added.
fn apply_role_member(env: &Env, role: &String, member: &Address) -> Result<bool, Error> {
    let key = DataKey::RoleMember(role.clone(), member.clone());
    if env.storage().persistent().has(&key) {
        return Ok(false);
    }

    let cfg = read_config(env)?;
    let members_key = DataKey::Members(role.clone());
    let mut members = read_addr_list(env, &members_key);
    if members.len() >= cfg.max_members_per_role {
        return Err(Error::LimitExceeded);
    }

    let roles_key = DataKey::Roles(member.clone());
    let mut roles = read_str_list(env, &roles_key);
    if roles.len() >= cfg.max_roles_per_address {
        return Err(Error::LimitExceeded);
    }

    env.storage().persistent().set(&key, &true);
    members.push_back(member.clone());
    write_addr_list(env, &members_key, &members);
    roles.push_back(role.clone());
    write_str_list(env, &roles_key, &roles);
    Ok(true)
}

/// Remove `member` from `role`, maintaining both indexes. Returns `true` when
/// the membership existed.
fn apply_role_member_revoke(env: &Env, role: &String, member: &Address) -> bool {
    let key = DataKey::RoleMember(role.clone(), member.clone());
    if !env.storage().persistent().has(&key) {
        return false;
    }
    env.storage().persistent().remove(&key);

    let members_key = DataKey::Members(role.clone());
    let members = read_addr_list(env, &members_key);
    let kept_members = keep_addrs_except(env, &members, member);
    write_addr_list(env, &members_key, &kept_members);

    let roles_key = DataKey::Roles(member.clone());
    let roles = read_str_list(env, &roles_key);
    let kept_roles = keep_strings_except(env, &roles, role);
    write_str_list(env, &roles_key, &kept_roles);
    true
}

fn protocol_role_permissions(env: &Env, role: ProtocolRole) -> Vec<String> {
    let names: &[&str] = match role {
        ProtocolRole::Owner => &[
            "vault.deposit",
            "vault.withdraw",
            "vault.transfer",
            "vault.configure",
        ],
        ProtocolRole::Admin => &[
            "roles.manage",
            "vault.pause",
            "vault.unpause",
            "strategy.configure",
            "strategy.approve",
            "fees.configure",
            "keeper.configure",
            "emergency.pause",
        ],
        ProtocolRole::Strategist => &[
            "strategy.rebalance",
            "strategy.rotate",
            "strategy.harvest",
        ],
        ProtocolRole::Keeper => &[
            "strategy.harvest",
            "strategy.rebalance",
            "oracle.update",
        ],
        ProtocolRole::Emergency => &[
            "emergency.pause",
            "emergency.cancel",
            "vault.pause",
            "strategy.pause",
        ],
    };
    let mut permissions = Vec::new(env);
    for name in names {
        permissions.push_back(String::from_str(env, name));
    }
    permissions
}

fn provision_protocol_role(
    env: &Env,
    actor: &Address,
    role: ProtocolRole,
    member: &Address,
) -> Result<(), Error> {
    let name = role.name(env);
    validate_role(&name)?;
    if role_exists(env, &name) {
        return Err(Error::RoleExists);
    }
    let config = read_config(env)?;
    let mut roles = read_str_list(env, &DataKey::RoleList);
    if roles.len() >= config.max_roles {
        return Err(Error::LimitExceeded);
    }

    // Keep the root as role admin: operational role holders can be rotated
    // without granting them authority to redefine their own permissions.
    env.storage()
        .persistent()
        .set(&DataKey::RoleAdmin(name.clone()), actor);
    roles.push_back(name.clone());
    write_str_list(env, &DataKey::RoleList, &roles);
    emit_role_created(
        env,
        RoleCreatedEvent {
            actor: actor.clone(),
            role: name.clone(),
            admin: actor.clone(),
        },
    );

    for selector in protocol_role_permissions(env, role).iter() {
        if apply_role_permission(env, &name, &selector)? {
            emit_role_permission(
                env,
                RolePermissionEvent {
                    actor: actor.clone(),
                    role: name.clone(),
                    selector,
                    granted: true,
                    batch_size: 1,
                },
            );
        }
    }
    if apply_role_member(env, &name, member)? {
        emit_role_member(
            env,
            RoleMemberEvent {
                actor: actor.clone(),
                role: name,
                member: member.clone(),
                granted: true,
                batch_size: 1,
            },
        );
    }
    Ok(())
}

// ── Contract ──────────────────────────────────────────────────────────────────

/// Per-function access control list with wildcard support and batch updates.
#[contract]
pub struct AccessControl;

#[contractimpl]
impl AccessControl {
    // ── Initialization & root administration ────────────────────────────────

    /// Initialize the ACL, making `admin` the root admin.
    ///
    /// The root admin is deliberately *not* seeded with a `*` grant: it bypasses
    /// the ACL tables entirely (see [`AccessControl::has_permission`]), so it
    /// cannot be stripped by a mis-issued batch, and
    /// [`AccessControl::renounce_all`] — which drops everything a caller holds —
    /// leaves the root admin's management authority intact.
    pub fn initialize(env: Env, admin: Address, config: AclConfig) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        if config.max_batch_size == 0 || config.max_wildcards == 0 {
            return Err(Error::LimitExceeded);
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Config, &config);
        env.storage().instance().set(&DataKey::Lockdown, &false);
        emit_config(
            &env,
            ConfigEvent {
                actor: admin,
                config,
            },
        );
        Ok(())
    }

    /// Provision the five standard Perigee roles with fixed granular
    /// permissions. Role membership and permissions remain manageable through
    /// the existing role APIs; only the root admin can administer these roles.
    ///
    /// The `admin` role here is an operational role and is distinct from the
    /// ACL's root admin. The root admin remains the sole authority over role
    /// definitions, preventing an operational admin from escalating its own
    /// permissions.
    pub fn setup_protocol_roles(
        env: Env,
        actor: Address,
        owner: Address,
        admin: Address,
        strategist: Address,
        keeper: Address,
        emergency: Address,
    ) -> Result<(), Error> {
        require_root(&env, &actor)?;
        require_writable(&env)?;
        if env
            .storage()
            .instance()
            .get::<_, bool>(&DataKey::ProtocolRolesConfigured)
            .unwrap_or(false)
        {
            return Err(Error::ProtocolRolesAlreadyConfigured);
        }

        provision_protocol_role(&env, &actor, ProtocolRole::Owner, &owner)?;
        provision_protocol_role(&env, &actor, ProtocolRole::Admin, &admin)?;
        provision_protocol_role(&env, &actor, ProtocolRole::Strategist, &strategist)?;
        provision_protocol_role(&env, &actor, ProtocolRole::Keeper, &keeper)?;
        provision_protocol_role(&env, &actor, ProtocolRole::Emergency, &emergency)?;
        env.storage()
            .instance()
            .set(&DataKey::ProtocolRolesConfigured, &true);
        Ok(())
    }

    /// Hand the root admin role to `new_admin`.
    ///
    /// The only way the root admin changes, and therefore the only way the ACL
    /// can change hands: no grant or role operation can remove it.
    ///
    /// Deliberately **not** blocked by lockdown. During an incident the most
    /// useful thing an honest admin can do is rotate a suspected-compromised
    /// key, and gating this would force an unfreeze first. Gating it would also
    /// buy no security: a root key that is already compromised can simply call
    /// [`AccessControl::set_lockdown`] to unfreeze before rotating.
    pub fn transfer_admin(env: Env, actor: Address, new_admin: Address) -> Result<(), Error> {
        let previous = read_admin(&env)?;
        require_root(&env, &actor)?;
        env.storage().instance().set(&DataKey::Admin, &new_admin);
        emit_admin_transferred(
            &env,
            AdminTransferredEvent {
                actor,
                previous_admin: previous,
                new_admin,
            },
        );
        Ok(())
    }

    /// Replace the bounds in [`AclConfig`].
    pub fn set_config(env: Env, actor: Address, config: AclConfig) -> Result<(), Error> {
        require_root(&env, &actor)?;
        require_writable(&env)?;
        if config.max_batch_size == 0 || config.max_wildcards == 0 {
            return Err(Error::LimitExceeded);
        }
        env.storage().instance().set(&DataKey::Config, &config);
        emit_config(&env, ConfigEvent { actor, config });
        Ok(())
    }

    /// Freeze or unfreeze every mutating operation.
    ///
    /// Releasing works even while locked, so the freeze is always escapable. So
    /// do the self-service renounce entry points, so an address can give up
    /// its own access while the protocol is frozen.
    pub fn set_lockdown(env: Env, actor: Address, active: bool) -> Result<(), Error> {
        require_root(&env, &actor)?;
        env.storage().instance().set(&DataKey::Lockdown, &active);
        emit_lockdown(&env, LockdownEvent { actor, active });
        Ok(())
    }

    // ── Queries ────────────────────────────────────────────────────────────

    /// The root admin.
    pub fn admin(env: Env) -> Result<Address, Error> {
        read_admin(&env)
    }

    /// The active bounds.
    pub fn config(env: Env) -> Result<AclConfig, Error> {
        read_config(&env)
    }

    /// Whether lockdown is engaged.
    pub fn is_lockdown(env: Env) -> bool {
        is_locked(&env)
    }

    /// Whether `caller` may invoke `sel`, directly or through a role.
    ///
    /// This is the guard entry point: host contracts call it at the top of
    /// every privileged function.
    pub fn has_permission(env: Env, caller: Address, sel: String) -> bool {
        permits(&env, &caller, &sel)
    }

    /// Assert that `caller` may invoke `sel`, for use with `?` in host contracts.
    pub fn require_permission(env: Env, caller: Address, sel: String) -> Result<(), Error> {
        if permits(&env, &caller, &sel) {
            Ok(())
        } else {
            Err(Error::Unauthorized)
        }
    }

    /// Every direct grant held by `caller`.
    pub fn grants_of(env: Env, caller: Address) -> Vec<String> {
        read_str_list(&env, &DataKey::Grants(caller))
    }

    /// Whether `caller` holds a direct (non-role) grant on `sel`.
    pub fn is_granted(env: Env, caller: Address, sel: String) -> bool {
        env.storage().persistent().has(&DataKey::Grant(caller, sel))
    }

    /// Every role `caller` belongs to.
    pub fn roles_of(env: Env, caller: Address) -> Vec<String> {
        read_str_list(&env, &DataKey::Roles(caller))
    }

    /// Whether `caller` belongs to `role`.
    pub fn has_role(env: Env, caller: Address, role: String) -> bool {
        env.storage()
            .persistent()
            .has(&DataKey::RoleMember(role, caller))
    }

    /// Whether `caller` belongs to one of the canonical Perigee roles.
    pub fn has_protocol_role(env: Env, caller: Address, role: ProtocolRole) -> bool {
        env.storage()
            .persistent()
            .has(&DataKey::RoleMember(role.name(&env), caller))
    }

    /// Whether the canonical Perigee role set has been provisioned.
    pub fn protocol_roles_configured(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::ProtocolRolesConfigured)
            .unwrap_or(false)
    }

    /// Every member of `role`.
    pub fn members_of(env: Env, role: String) -> Vec<Address> {
        read_addr_list(&env, &DataKey::Members(role))
    }

    /// Every selector attached to `role`.
    pub fn permissions_of(env: Env, role: String) -> Vec<String> {
        read_str_list(&env, &DataKey::Permissions(role))
    }

    /// Summary of `role`, including its admin.
    pub fn role_info(env: Env, role: String) -> Result<RoleInfo, Error> {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::RoleAdmin(role.clone()))
            .ok_or(Error::RoleNotFound)?;
        Ok(RoleInfo {
            permission_count: read_str_list(&env, &DataKey::Permissions(role.clone())).len(),
            member_count: read_addr_list(&env, &DataKey::Members(role.clone())).len(),
            role,
            admin,
        })
    }

    /// Whether `role` exists.
    pub fn role_exists(env: Env, role: String) -> bool {
        role_exists(&env, &role)
    }

    /// Every role that exists.
    pub fn list_roles(env: Env) -> Vec<String> {
        read_str_list(&env, &DataKey::RoleList)
    }

    /// Every registered namespace wildcard, with the trailing `*` restored.
    pub fn list_wildcards(env: Env) -> Vec<String> {
        let mut out = Vec::new(&env);
        for prefix in wildcard_prefixes(&env).iter() {
            out.push_back(selector::pattern(&env, &prefix));
        }
        out
    }

    /// Whether a namespace wildcard pattern is registered and live.
    pub fn is_wildcard_registered(env: Env, pattern: String) -> bool {
        match selector::classify(&env, &pattern) {
            Ok(SelectorKind::Namespace(prefix)) => wildcard_prefixes(&env).contains(&prefix),
            _ => false,
        }
    }

    /// How many live grants and role selectors reference a namespace pattern.
    ///
    /// [`AccessControl::unregister_wildcard`] refuses while this is non-zero, so
    /// a pattern can never be pulled out from under a live permission.
    pub fn wildcard_usage(env: Env, pattern: String) -> u32 {
        match selector::classify(&env, &pattern) {
            Ok(SelectorKind::Namespace(prefix)) => wildcard_use(&env, &prefix),
            _ => 0,
        }
    }

    // ── Direct grants ──────────────────────────────────────────────────────

    /// Grant `caller` the selector `sel`.
    pub fn grant(env: Env, actor: Address, caller: Address, sel: String) -> Result<bool, Error> {
        require_root(&env, &actor)?;
        require_writable(&env)?;
        let added = apply_grant(&env, &caller, &sel)?;
        if added {
            emit_grant(
                &env,
                GrantEvent {
                    actor,
                    target: caller,
                    selector: sel,
                    granted: true,
                    batch_size: 1,
                },
            );
        }
        Ok(added)
    }

    /// Revoke the selector `sel` from `caller`.
    pub fn revoke(env: Env, actor: Address, caller: Address, sel: String) -> Result<bool, Error> {
        require_root(&env, &actor)?;
        require_writable(&env)?;
        let removed = apply_revoke(&env, &caller, &sel);
        if removed {
            emit_grant(
                &env,
                GrantEvent {
                    actor,
                    target: caller,
                    selector: sel,
                    granted: false,
                    batch_size: 1,
                },
            );
        }
        Ok(removed)
    }

    /// Grant many `(address, selector)` pairs in one transaction.
    ///
    /// Entries already held are skipped, so the call is idempotent. Returns the
    /// number of entries actually created.
    pub fn batch_grant(env: Env, actor: Address, entries: Vec<GrantEntry>) -> Result<u32, Error> {
        require_root(&env, &actor)?;
        require_writable(&env)?;
        let size = entries.len();
        check_batch_size(&env, size)?;

        let mut applied = 0u32;
        for entry in entries.iter() {
            if apply_grant(&env, &entry.target, &entry.selector)? {
                applied += 1;
                emit_grant(
                    &env,
                    GrantEvent {
                        actor: actor.clone(),
                        target: entry.target.clone(),
                        selector: entry.selector.clone(),
                        granted: true,
                        batch_size: size,
                    },
                );
            }
        }
        Ok(applied)
    }

    /// Revoke many `(address, selector)` pairs in one transaction.
    ///
    /// Entries not held are skipped, so the call is idempotent. Returns the
    /// number of entries actually removed.
    pub fn batch_revoke(env: Env, actor: Address, entries: Vec<GrantEntry>) -> Result<u32, Error> {
        require_root(&env, &actor)?;
        require_writable(&env)?;
        let size = entries.len();
        check_batch_size(&env, size)?;

        let mut applied = 0u32;
        for entry in entries.iter() {
            if apply_revoke(&env, &entry.target, &entry.selector) {
                applied += 1;
                emit_grant(
                    &env,
                    GrantEvent {
                        actor: actor.clone(),
                        target: entry.target.clone(),
                        selector: entry.selector.clone(),
                        granted: false,
                        batch_size: size,
                    },
                );
            }
        }
        Ok(applied)
    }

    /// Give up one of `caller`'s own direct grants.
    ///
    /// Self-service: available during lockdown, and open to anyone rather than
    /// only the root admin.
    pub fn renounce(env: Env, caller: Address, sel: String) -> Result<bool, Error> {
        caller.require_auth();
        let removed = apply_revoke(&env, &caller, &sel);
        if removed {
            emit_grant(
                &env,
                GrantEvent {
                    actor: caller.clone(),
                    target: caller,
                    selector: sel,
                    granted: false,
                    batch_size: 1,
                },
            );
        }
        Ok(removed)
    }

    /// Give up every direct grant **and** every role `caller` holds.
    ///
    /// The escape hatch for a compromised or over-privileged operator: one
    /// signature, one transaction, back to zero privilege. Available during
    /// lockdown. The root admin is unaffected, because it lives outside the ACL
    /// tables.
    pub fn renounce_all(env: Env, caller: Address) -> Result<u32, Error> {
        caller.require_auth();
        let mut removed = 0u32;

        for sel in read_str_list(&env, &DataKey::Grants(caller.clone())).iter() {
            if apply_revoke(&env, &caller, &sel) {
                removed += 1;
                emit_grant(
                    &env,
                    GrantEvent {
                        actor: caller.clone(),
                        target: caller.clone(),
                        selector: sel.clone(),
                        granted: false,
                        batch_size: removed,
                    },
                );
            }
        }

        for role in read_str_list(&env, &DataKey::Roles(caller.clone())).iter() {
            if apply_role_member_revoke(&env, &role, &caller) {
                removed += 1;
                emit_role_member(
                    &env,
                    RoleMemberEvent {
                        actor: caller.clone(),
                        role: role.clone(),
                        member: caller.clone(),
                        granted: false,
                        batch_size: removed,
                    },
                );
            }
        }

        Ok(removed)
    }

    // ── Roles ──────────────────────────────────────────────────────────────

    /// Create `role`, administered by `role_admin`.
    pub fn create_role(
        env: Env,
        actor: Address,
        role: String,
        role_admin: Address,
    ) -> Result<(), Error> {
        require_root(&env, &actor)?;
        require_writable(&env)?;
        validate_role(&role)?;
        if role_exists(&env, &role) {
            return Err(Error::RoleExists);
        }
        let cfg = read_config(&env)?;
        let mut roles = read_str_list(&env, &DataKey::RoleList);
        if roles.len() >= cfg.max_roles {
            return Err(Error::LimitExceeded);
        }
        env.storage()
            .persistent()
            .set(&DataKey::RoleAdmin(role.clone()), &role_admin);
        roles.push_back(role.clone());
        write_str_list(&env, &DataKey::RoleList, &roles);
        emit_role_created(
            &env,
            RoleCreatedEvent {
                actor,
                role,
                admin: role_admin,
            },
        );
        Ok(())
    }

    /// Change the admin of `role`. Root admin only.
    pub fn set_role_admin(
        env: Env,
        actor: Address,
        role: String,
        new_admin: Address,
    ) -> Result<(), Error> {
        require_root(&env, &actor)?;
        require_writable(&env)?;
        if !role_exists(&env, &role) {
            return Err(Error::RoleNotFound);
        }
        env.storage()
            .persistent()
            .set(&DataKey::RoleAdmin(role.clone()), &new_admin);
        emit_role_admin(
            &env,
            RoleAdminEvent {
                actor,
                role,
                admin: new_admin,
            },
        );
        Ok(())
    }

    /// Attach `sel` to `role`.
    ///
    /// Callable by the root admin or the role's own admin.
    pub fn grant_role_permission(
        env: Env,
        actor: Address,
        role: String,
        sel: String,
    ) -> Result<bool, Error> {
        if !role_exists(&env, &role) {
            return Err(Error::RoleNotFound);
        }
        require_role_admin(&env, &actor, &role)?;
        require_writable(&env)?;
        let added = apply_role_permission(&env, &role, &sel)?;
        if added {
            emit_role_permission(
                &env,
                RolePermissionEvent {
                    actor,
                    role,
                    selector: sel,
                    granted: true,
                    batch_size: 1,
                },
            );
        }
        Ok(added)
    }

    /// Detach `sel` from `role`.
    pub fn revoke_role_permission(
        env: Env,
        actor: Address,
        role: String,
        sel: String,
    ) -> Result<bool, Error> {
        if !role_exists(&env, &role) {
            return Err(Error::RoleNotFound);
        }
        require_role_admin(&env, &actor, &role)?;
        require_writable(&env)?;
        let removed = apply_role_revoke(&env, &role, &sel);
        if removed {
            emit_role_permission(
                &env,
                RolePermissionEvent {
                    actor,
                    role,
                    selector: sel,
                    granted: false,
                    batch_size: 1,
                },
            );
        }
        Ok(removed)
    }

    /// Add `member` to `role`.
    pub fn grant_role(
        env: Env,
        actor: Address,
        role: String,
        member: Address,
    ) -> Result<bool, Error> {
        if !role_exists(&env, &role) {
            return Err(Error::RoleNotFound);
        }
        require_role_admin(&env, &actor, &role)?;
        require_writable(&env)?;
        let added = apply_role_member(&env, &role, &member)?;
        if added {
            emit_role_member(
                &env,
                RoleMemberEvent {
                    actor,
                    role,
                    member,
                    granted: true,
                    batch_size: 1,
                },
            );
        }
        Ok(added)
    }

    /// Remove `member` from `role`.
    pub fn revoke_role(
        env: Env,
        actor: Address,
        role: String,
        member: Address,
    ) -> Result<bool, Error> {
        if !role_exists(&env, &role) {
            return Err(Error::RoleNotFound);
        }
        require_role_admin(&env, &actor, &role)?;
        require_writable(&env)?;
        let removed = apply_role_member_revoke(&env, &role, &member);
        if removed {
            emit_role_member(
                &env,
                RoleMemberEvent {
                    actor,
                    role,
                    member,
                    granted: false,
                    batch_size: 1,
                },
            );
        }
        Ok(removed)
    }

    /// Give up a role membership. Self-service, available during lockdown.
    pub fn renounce_role(env: Env, caller: Address, role: String) -> Result<bool, Error> {
        caller.require_auth();
        let removed = apply_role_member_revoke(&env, &role, &caller);
        if removed {
            emit_role_member(
                &env,
                RoleMemberEvent {
                    actor: caller.clone(),
                    role,
                    member: caller,
                    granted: false,
                    batch_size: 1,
                },
            );
        }
        Ok(removed)
    }

    /// Destroy `role`, detaching every member and selector it carried.
    ///
    /// Root admin only, so a role admin cannot dissolve the role out from
    /// under the rest of the protocol. Returns the number of memberships and
    /// selectors removed, counting duplicates as they are reported.
    pub fn destroy_role(env: Env, actor: Address, role: String) -> Result<u32, Error> {
        require_root(&env, &actor)?;
        require_writable(&env)?;
        if !role_exists(&env, &role) {
            return Err(Error::RoleNotFound);
        }

        let permissions = read_str_list(&env, &DataKey::Permissions(role.clone()));
        let members = read_addr_list(&env, &DataKey::Members(role.clone()));

        for sel in permissions.iter() {
            apply_role_revoke(&env, &role, &sel);
        }
        for member in members.iter() {
            apply_role_member_revoke(&env, &role, &member);
        }

        env.storage()
            .persistent()
            .remove(&DataKey::RoleAdmin(role.clone()));
        let roles = read_str_list(&env, &DataKey::RoleList);
        let kept = keep_strings_except(&env, &roles, &role);
        write_str_list(&env, &DataKey::RoleList, &kept);

        emit_role_destroyed(
            &env,
            RoleDestroyedEvent {
                actor,
                role,
                members_detached: members.len(),
                permissions_detached: permissions.len(),
            },
        );
        Ok(members.len() + permissions.len())
    }

    // ── Batch role updates ─────────────────────────────────────────────────

    /// Replace the whole membership of `role` in one transaction.
    ///
    /// The usual way to move a team from one roster to another without a window
    /// in which nobody is authorized. Returns the resulting member count.
    pub fn batch_set_members(
        env: Env,
        actor: Address,
        role: String,
        members: Vec<Address>,
    ) -> Result<u32, Error> {
        if !role_exists(&env, &role) {
            return Err(Error::RoleNotFound);
        }
        require_role_admin(&env, &actor, &role)?;
        require_writable(&env)?;
        let size = members.len();
        check_batch_size(&env, size)?;

        let current = read_addr_list(&env, &DataKey::Members(role.clone()));
        for member in current.iter() {
            if !members.contains(&member) && apply_role_member_revoke(&env, &role, &member) {
                emit_role_member(
                    &env,
                    RoleMemberEvent {
                        actor: actor.clone(),
                        role: role.clone(),
                        member: member.clone(),
                        granted: false,
                        batch_size: size,
                    },
                );
            }
        }
        for member in members.iter() {
            if apply_role_member(&env, &role, &member)? {
                emit_role_member(
                    &env,
                    RoleMemberEvent {
                        actor: actor.clone(),
                        role: role.clone(),
                        member: member.clone(),
                        granted: true,
                        batch_size: size,
                    },
                );
            }
        }

        Ok(members.len())
    }

    /// Replace the whole selector set of `role` in one transaction.
    ///
    /// Returns the resulting selector count. Because a batch is atomic, a role
    /// is never observed holding a half-applied permission set.
    pub fn batch_set_permissions(
        env: Env,
        actor: Address,
        role: String,
        selectors: Vec<String>,
    ) -> Result<u32, Error> {
        if !role_exists(&env, &role) {
            return Err(Error::RoleNotFound);
        }
        require_role_admin(&env, &actor, &role)?;
        require_writable(&env)?;
        let size = selectors.len();
        check_batch_size(&env, size)?;

        let current = read_str_list(&env, &DataKey::Permissions(role.clone()));
        for sel in current.iter() {
            if !selectors.contains(&sel) && apply_role_revoke(&env, &role, &sel) {
                emit_role_permission(
                    &env,
                    RolePermissionEvent {
                        actor: actor.clone(),
                        role: role.clone(),
                        selector: sel.clone(),
                        granted: false,
                        batch_size: size,
                    },
                );
            }
        }
        for sel in selectors.iter() {
            if apply_role_permission(&env, &role, &sel)? {
                emit_role_permission(
                    &env,
                    RolePermissionEvent {
                        actor: actor.clone(),
                        role: role.clone(),
                        selector: sel.clone(),
                        granted: true,
                        batch_size: size,
                    },
                );
            }
        }

        Ok(selectors.len())
    }

    // ── Namespace wildcards ────────────────────────────────────────────────

    /// Register a namespace wildcard such as `vault.*`.
    ///
    /// The prefix (the pattern minus its trailing `*`) becomes a candidate in
    /// every wildcard probe. Registration is capped by `max_wildcards`, which
    /// is what keeps [`AccessControl::has_permission`] at a fixed cost.
    pub fn register_wildcard(env: Env, actor: Address, pattern: String) -> Result<bool, Error> {
        require_root(&env, &actor)?;
        require_writable(&env)?;

        let prefix = match selector::classify(&env, &pattern)? {
            SelectorKind::Namespace(prefix) => prefix,
            _ => return Err(Error::InvalidSelector),
        };

        let mut registered = wildcard_prefixes(&env);
        if registered.contains(&prefix) {
            return Ok(false);
        }
        if registered.len() >= read_config(&env)?.max_wildcards {
            return Err(Error::LimitExceeded);
        }

        registered.push_back(prefix);
        write_str_list(&env, &DataKey::Wildcards, &registered);
        emit_wildcard(
            &env,
            WildcardEvent {
                actor,
                pattern,
                registered: true,
            },
        );
        Ok(true)
    }

    /// Remove a namespace wildcard.
    ///
    /// Refused with [`Error::WildcardInUse`] while any direct grant or role
    /// selector still references the pattern, so unregistering can never
    /// silently disable a live permission.
    pub fn unregister_wildcard(env: Env, actor: Address, pattern: String) -> Result<bool, Error> {
        require_root(&env, &actor)?;
        require_writable(&env)?;

        let prefix = match selector::classify(&env, &pattern)? {
            SelectorKind::Namespace(prefix) => prefix,
            _ => return Err(Error::InvalidSelector),
        };

        let registered = wildcard_prefixes(&env);
        if !registered.contains(&prefix) {
            return Err(Error::WildcardNotFound);
        }
        if wildcard_use(&env, &prefix) > 0 {
            return Err(Error::WildcardInUse);
        }

        let kept = keep_strings_except(&env, &registered, &prefix);
        write_str_list(&env, &DataKey::Wildcards, &kept);

        emit_wildcard(
            &env,
            WildcardEvent {
                actor,
                pattern,
                registered: false,
            },
        );
        Ok(true)
    }
}

#[cfg(test)]
mod test_batch_atomicity;
