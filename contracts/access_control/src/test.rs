#![cfg(test)]

//! Test suite for the access control list.
//!
//! Organized by the three capabilities the issue asks for — per-function
//! permissions, wildcard support, batch updates — with the authorization and
//! limit invariants that hold across all three.

use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Events as _},
    vec, Address, Env, String, Symbol, TryIntoVal, Vec,
};

use crate::{
    AccessControl, AccessControlClient, AclConfig, Error, GrantEntry, ProtocolRole, RoleInfo,
};

type Sel = String;

/// Boot a fresh ACL with the default bounds. Returns the env, the contract id,
/// and the root admin.
fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let id = env.register(AccessControl, ());
    let client = AccessControlClient::new(&env, &id);
    client.initialize(&admin, &AclConfig::default_config());
    (env, id, admin)
}

/// Boot a fresh ACL with custom bounds.
fn setup_with(config: AclConfig) -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let id = env.register(AccessControl, ());
    let client = AccessControlClient::new(&env, &id);
    client.initialize(&admin, &config);
    (env, id, admin)
}

fn s(env: &Env, text: &str) -> Sel {
    String::from_str(env, text)
}

fn cfg_with(max_batch_size: u32) -> AclConfig {
    AclConfig {
        max_batch_size,
        ..AclConfig::default_config()
    }
}

/// Number of `acl`-topic events whose second topic is `topic`, in the events of
/// the **most recent** top-level invocation.
///
/// The Soroban test host returns only the events produced by the latest
/// invocation, so this must be read immediately after the call under test.
/// Tests that need a running total across several calls must accumulate it
/// themselves; see `every_mutation_publishes_an_auditable_event`.
fn count_events(env: &Env, topic: Symbol) -> u32 {
    env.events()
        .all()
        .iter()
        .filter(|(_, topics, _)| topic_at(env, topics, 1) == Some(topic.clone()))
        .count() as u32
}

/// Decode the `index`-th topic of an event as a `Symbol`.
fn topic_at(env: &Env, topics: &Vec<soroban_sdk::Val>, index: u32) -> Option<Symbol> {
    let raw = topics.get(index)?;
    let decoded: Result<Symbol, _> = raw.try_into_val(env);
    decoded.ok()
}

// ── Initialization ────────────────────────────────────────────────────────────

#[test]
fn initialize_sets_admin_config_and_lockdown_state() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let config = AclConfig::default_config();
    let id = env.register(AccessControl, ());
    let client = AccessControlClient::new(&env, &id);

    client.initialize(&admin, &config);
    assert_eq!(count_events(&env, symbol_short!("config")), 1);

    assert_eq!(client.admin(), admin);
    assert_eq!(client.config(), config);
    assert!(!client.is_lockdown());
}

#[test]
fn initialize_is_single_shot() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    assert_eq!(
        client.try_initialize(&admin, &AclConfig::default_config()),
        Err(Ok(Error::AlreadyInitialized))
    );
}

#[test]
fn initialize_rejects_degenerate_bounds() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let id = env.register(AccessControl, ());
    let client = AccessControlClient::new(&env, &id);

    let no_batches = AclConfig {
        max_batch_size: 0,
        ..AclConfig::default_config()
    };
    assert_eq!(
        client.try_initialize(&admin, &no_batches),
        Err(Ok(Error::LimitExceeded))
    );

    let no_wildcards = AclConfig {
        max_wildcards: 0,
        ..AclConfig::default_config()
    };
    assert_eq!(
        client.try_initialize(&admin, &no_wildcards),
        Err(Ok(Error::LimitExceeded))
    );
}

#[test]
fn queries_fail_before_initialization() {
    let env = Env::default();
    let id = env.register(AccessControl, ());
    let client = AccessControlClient::new(&env, &id);
    assert_eq!(client.try_admin(), Err(Ok(Error::NotInitialized)));
    assert_eq!(client.try_config(), Err(Ok(Error::NotInitialized)));
}

// ── Per-function permissions ──────────────────────────────────────────────────

#[test]
fn direct_grant_authorizes_only_its_selector() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let keeper = Address::generate(&env);

    assert!(!client.has_permission(&keeper, &s(&env, "harvest")));
    client.grant(&admin, &keeper, &s(&env, "harvest"));
    assert_eq!(count_events(&env, symbol_short!("grant")), 1);

    assert!(client.has_permission(&keeper, &s(&env, "harvest")));
    assert!(!client.has_permission(&keeper, &s(&env, "withdraw")));
    assert!(client.is_granted(&keeper, &s(&env, "harvest")));
    assert!(!client.is_granted(&keeper, &s(&env, "withdraw")));
    assert_eq!(client.grants_of(&keeper), vec![&env, s(&env, "harvest")]);
}

#[test]
fn revoke_removes_the_grant_and_is_idempotent() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let keeper = Address::generate(&env);

    client.grant(&admin, &keeper, &s(&env, "harvest"));
    assert!(!client.grant(&admin, &keeper, &s(&env, "harvest"))); // no-op
    assert_eq!(count_events(&env, symbol_short!("grant")), 0);

    client.revoke(&admin, &keeper, &s(&env, "harvest"));
    assert_eq!(count_events(&env, symbol_short!("grant")), 1);
    assert!(!client.revoke(&admin, &keeper, &s(&env, "harvest"))); // no-op
    assert_eq!(count_events(&env, symbol_short!("grant")), 0);

    assert!(!client.has_permission(&keeper, &s(&env, "harvest")));
    assert_eq!(client.grants_of(&keeper), Vec::new(&env));
}

#[test]
fn grant_and_revoke_require_the_root_admin() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let stranger = Address::generate(&env);
    let target = Address::generate(&env);

    assert_eq!(
        client.try_grant(&stranger, &target, &s(&env, "harvest")),
        Err(Ok(Error::Unauthorized))
    );
    assert_eq!(
        client.try_revoke(&stranger, &target, &s(&env, "harvest")),
        Err(Ok(Error::Unauthorized))
    );
    // The root admin can always act, even on itself.
    assert!(client.has_permission(&admin, &s(&env, "harvest")));
}

#[test]
fn require_permission_raises_on_denial() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let keeper = Address::generate(&env);
    client.grant(&admin, &keeper, &s(&env, "harvest"));

    client.require_permission(&keeper, &s(&env, "harvest"));
    assert_eq!(
        client.try_require_permission(&keeper, &s(&env, "withdraw")),
        Err(Ok(Error::Unauthorized))
    );
}

#[test]
fn malformed_selectors_are_denied_not_authorized() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let keeper = Address::generate(&env);
    client.grant(&admin, &keeper, &s(&env, "harvest"));

    // A bad argument must never widen access.
    assert!(!client.has_permission(&keeper, &s(&env, "")));
    assert!(!client.has_permission(&keeper, &s(&env, "ha*r")));
    assert!(!client.has_permission(&keeper, &s(&env, "harvest*")));
    assert_eq!(
        client.try_grant(&admin, &keeper, &s(&env, "")),
        Err(Ok(Error::InvalidSelector))
    );
    assert_eq!(
        client.try_grant(&admin, &keeper, &s(&env, "ha*r")),
        Err(Ok(Error::InvalidSelector))
    );
}

#[test]
fn over_long_selectors_are_rejected() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let target = Address::generate(&env);
    let long = "x".repeat(crate::MAX_SELECTOR_LEN as usize + 1);

    assert_eq!(
        client.try_grant(&admin, &target, &s(&env, &long)),
        Err(Ok(Error::SelectorTooLong))
    );
    assert!(!client.has_permission(&target, &s(&env, &long)));
}

// ── Wildcard support ──────────────────────────────────────────────────────────

#[test]
fn universal_wildcard_authorizes_every_selector() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let auditor = Address::generate(&env);

    client.grant(&admin, &auditor, &s(&env, "*"));

    assert!(client.has_permission(&auditor, &s(&env, "deposit")));
    assert!(client.has_permission(&auditor, &s(&env, "vault.withdraw")));
    assert!(client.has_permission(&auditor, &s(&env, "anything.at.all")));
    // Not even the universal wildcard matches a malformed selector.
    assert!(!client.has_permission(&auditor, &s(&env, "bad*sel")));
}

#[test]
fn namespace_wildcard_must_be_registered_before_granting() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let keeper = Address::generate(&env);

    assert_eq!(
        client.try_grant(&admin, &keeper, &s(&env, "vault.*")),
        Err(Ok(Error::UnregisteredWildcard))
    );

    client.register_wildcard(&admin, &s(&env, "vault.*"));
    assert_eq!(count_events(&env, symbol_short!("wildcard")), 1);

    assert!(client.grant(&admin, &keeper, &s(&env, "vault.*")));
}

#[test]
fn namespace_wildcard_covers_only_its_prefix() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let keeper = Address::generate(&env);

    client.register_wildcard(&admin, &s(&env, "vault.*"));
    client.grant(&admin, &keeper, &s(&env, "vault.*"));

    assert!(client.has_permission(&keeper, &s(&env, "vault.deposit")));
    assert!(client.has_permission(&keeper, &s(&env, "vault.withdraw")));
    assert!(client.has_permission(&keeper, &s(&env, "vault.")));

    // Prefix matching is anchored at the start, so unrelated selectors — and
    // selectors that merely *contain* the prefix — are untouched.
    assert!(!client.has_permission(&keeper, &s(&env, "deposit")));
    assert!(!client.has_permission(&keeper, &s(&env, "xvault.deposit")));
    assert!(!client.has_permission(&keeper, &s(&env, "the_vault.deposit")));
}

#[test]
fn overlapping_namespace_wildcards_are_independent() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let keeper = Address::generate(&env);

    client.register_wildcard(&admin, &s(&env, "vault.*"));
    client.register_wildcard(&admin, &s(&env, "vault.admin.*"));
    client.grant(&admin, &keeper, &s(&env, "vault.*"));
    client.grant(&admin, &keeper, &s(&env, "vault.admin.*"));

    let admin_only = Address::generate(&env);
    client.grant(&admin, &admin_only, &s(&env, "vault.admin.*"));

    // Both wildcards reach `vault.admin.rotate`.
    assert!(client.has_permission(&keeper, &s(&env, "vault.admin.rotate")));
    // Only the narrower wildcard reaches it for the second address.
    assert!(client.has_permission(&admin_only, &s(&env, "vault.admin.rotate")));
    assert!(!client.has_permission(&admin_only, &s(&env, "vault.deposit")));
}

#[test]
fn wildcard_registration_is_idempotent_and_bounded() {
    let (env, id, admin) = setup_with(AclConfig {
        max_wildcards: 2,
        ..AclConfig::default_config()
    });
    let client = AccessControlClient::new(&env, &id);

    assert!(client.register_wildcard(&admin, &s(&env, "vault.*")));
    assert!(!client.register_wildcard(&admin, &s(&env, "vault.*"))); // no-op
    assert!(client.register_wildcard(&admin, &s(&env, "pool.*")));

    assert_eq!(
        client.try_register_wildcard(&admin, &s(&env, "other.*")),
        Err(Ok(Error::LimitExceeded))
    );

    assert!(client.is_wildcard_registered(&s(&env, "vault.*")));
    assert!(!client.is_wildcard_registered(&s(&env, "other.*")));
    // Only namespace patterns can be "registered wildcards".
    assert!(!client.is_wildcard_registered(&s(&env, "*")));
    assert!(!client.is_wildcard_registered(&s(&env, "deposit")));

    assert_eq!(
        client.list_wildcards(),
        vec![&env, s(&env, "vault.*"), s(&env, "pool.*")]
    );
}

#[test]
fn registering_a_non_pattern_is_rejected() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);

    for bad in ["deposit", "*", ""] {
        assert_eq!(
            client.try_register_wildcard(&admin, &s(&env, bad)),
            Err(Ok(Error::InvalidSelector)),
            "expected {bad} to be rejected"
        );
    }
    assert_eq!(
        client.try_register_wildcard(&Address::generate(&env), &s(&env, "vault.*")),
        Err(Ok(Error::Unauthorized))
    );
    assert_eq!(
        client.try_unregister_wildcard(&admin, &s(&env, "vault.*")),
        Err(Ok(Error::WildcardNotFound))
    );
    assert_eq!(client.wildcard_usage(&s(&env, "vault.*")), 0);
    assert_eq!(client.wildcard_usage(&s(&env, "deposit")), 0);
}

#[test]
fn unregistering_an_in_use_wildcard_is_refused() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let keeper = Address::generate(&env);
    let auditor = Address::generate(&env);

    client.register_wildcard(&admin, &s(&env, "vault.*"));
    client.grant(&admin, &keeper, &s(&env, "vault.*"));
    client.grant(&admin, &auditor, &s(&env, "vault.*"));

    assert_eq!(client.wildcard_usage(&s(&env, "vault.*")), 2);
    assert_eq!(
        client.try_unregister_wildcard(&admin, &s(&env, "vault.*")),
        Err(Ok(Error::WildcardInUse))
    );

    client.revoke(&admin, &keeper, &s(&env, "vault.*"));
    assert_eq!(client.wildcard_usage(&s(&env, "vault.*")), 1);
    assert_eq!(
        client.try_unregister_wildcard(&admin, &s(&env, "vault.*")),
        Err(Ok(Error::WildcardInUse))
    );

    client.revoke(&admin, &auditor, &s(&env, "vault.*"));
    assert_eq!(client.wildcard_usage(&s(&env, "vault.*")), 0);
    assert!(client.unregister_wildcard(&admin, &s(&env, "vault.*")));
    assert!(!client.is_wildcard_registered(&s(&env, "vault.*")));
    assert!(client.list_wildcards().is_empty());
}

#[test]
fn role_permissions_also_track_wildcard_usage() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let team = Address::generate(&env);

    client.register_wildcard(&admin, &s(&env, "pool.*"));
    client.create_role(&admin, &s(&env, "lp"), &admin);
    client.grant_role_permission(&admin, &s(&env, "lp"), &s(&env, "pool.*"));

    // The counter spans role selectors, not just direct grants.
    assert_eq!(client.wildcard_usage(&s(&env, "pool.*")), 1);
    assert_eq!(
        client.try_unregister_wildcard(&admin, &s(&env, "pool.*")),
        Err(Ok(Error::WildcardInUse))
    );

    client.revoke_role_permission(&admin, &s(&env, "lp"), &s(&env, "pool.*"));
    assert_eq!(client.wildcard_usage(&s(&env, "pool.*")), 0);
    assert!(client.unregister_wildcard(&admin, &s(&env, "pool.*")));
    let _ = team;
}

#[test]
fn a_dormant_namespace_grant_stops_matching_once_unregistered() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let keeper = Address::generate(&env);

    client.register_wildcard(&admin, &s(&env, "vault.*"));
    client.grant(&admin, &keeper, &s(&env, "vault.*"));
    assert!(client.has_permission(&keeper, &s(&env, "vault.deposit")));

    // `unregister_wildcard` is refused while the grant lives, so a live
    // permission can never be silently disabled…
    assert_eq!(
        client.try_unregister_wildcard(&admin, &s(&env, "vault.*")),
        Err(Ok(Error::WildcardInUse))
    );

    // …and once the grant is gone the pattern can be retired.
    client.revoke(&admin, &keeper, &s(&env, "vault.*"));
    client.unregister_wildcard(&admin, &s(&env, "vault.*"));
    assert!(!client.has_permission(&keeper, &s(&env, "vault.deposit")));
}

// ── Roles ─────────────────────────────────────────────────────────────────────

#[test]
fn role_membership_grants_every_permission_of_the_role() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let keeper = Address::generate(&env);

    client.create_role(&admin, &s(&env, "keepers"), &admin);
    client.grant_role_permission(&admin, &s(&env, "keepers"), &s(&env, "harvest"));
    client.grant_role_permission(&admin, &s(&env, "keepers"), &s(&env, "rebalance"));
    client.grant_role(&admin, &s(&env, "keepers"), &keeper);

    assert!(client.has_permission(&keeper, &s(&env, "harvest")));
    assert!(client.has_permission(&keeper, &s(&env, "rebalance")));
    assert!(!client.has_permission(&keeper, &s(&env, "withdraw")));

    // The role is not a direct grant.
    assert!(!client.is_granted(&keeper, &s(&env, "harvest")));
    assert!(client.has_role(&keeper, &s(&env, "keepers")));
    assert_eq!(client.roles_of(&keeper), vec![&env, s(&env, "keepers")]);
    assert_eq!(
        client.members_of(&s(&env, "keepers")),
        vec![&env, keeper.clone()]
    );
    assert_eq!(
        client.permissions_of(&s(&env, "keepers")),
        vec![&env, s(&env, "harvest"), s(&env, "rebalance")]
    );
    assert!(client.role_exists(&s(&env, "keepers")));
    assert_eq!(client.list_roles(), vec![&env, s(&env, "keepers")]);
}

#[test]
fn role_creation_emits_one_event() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);

    client.create_role(&admin, &s(&env, "keepers"), &admin);
    assert_eq!(count_events(&env, symbol_short!("role_new")), 1);

    // A rejected duplicate changes nothing and therefore emits nothing.
    assert_eq!(
        client.try_create_role(&admin, &s(&env, "keepers"), &admin),
        Err(Ok(Error::RoleExists))
    );
    assert_eq!(count_events(&env, symbol_short!("role_new")), 0);
}

#[test]
fn role_info_reports_counts_and_admin() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let role_admin = Address::generate(&env);
    let member = Address::generate(&env);

    client.create_role(&admin, &s(&env, "keepers"), &role_admin);
    client.grant_role_permission(&role_admin, &s(&env, "keepers"), &s(&env, "harvest"));
    client.grant_role(&role_admin, &s(&env, "keepers"), &member);

    assert_eq!(
        client.role_info(&s(&env, "keepers")),
        RoleInfo {
            role: s(&env, "keepers"),
            admin: role_admin,
            permission_count: 1,
            member_count: 1,
        }
    );
    assert_eq!(
        client.try_role_info(&s(&env, "nope")),
        Err(Ok(Error::RoleNotFound))
    );
}

#[test]
fn a_role_with_the_wildcard_authorizes_its_members() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let member = Address::generate(&env);

    client.create_role(&admin, &s(&env, "admins"), &admin);
    client.grant_role_permission(&admin, &s(&env, "admins"), &s(&env, "*"));
    client.grant_role(&admin, &s(&env, "admins"), &member);

    assert!(client.has_permission(&member, &s(&env, "deposit")));
    assert!(client.has_permission(&member, &s(&env, "admin.rotate")));
}

#[test]
fn a_role_can_carry_a_namespace_wildcard() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let member = Address::generate(&env);

    client.register_wildcard(&admin, &s(&env, "vault.*"));
    client.create_role(&admin, &s(&env, "vault_ops"), &admin);
    client.grant_role_permission(&admin, &s(&env, "vault_ops"), &s(&env, "vault.*"));
    client.grant_role(&admin, &s(&env, "vault_ops"), &member);

    assert!(client.has_permission(&member, &s(&env, "vault.deposit")));
    assert!(!client.has_permission(&member, &s(&env, "pool.deposit")));
}

#[test]
fn role_admin_manages_its_own_role_without_the_root_admin() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let role_admin = Address::generate(&env);
    let member = Address::generate(&env);
    let stranger = Address::generate(&env);

    client.create_role(&admin, &s(&env, "keepers"), &role_admin);

    client.grant_role_permission(&role_admin, &s(&env, "keepers"), &s(&env, "harvest"));
    client.grant_role(&role_admin, &s(&env, "keepers"), &member);
    assert!(client.has_permission(&member, &s(&env, "harvest")));

    // A stranger can touch neither the role nor its direct grants.
    assert_eq!(
        client.try_grant_role_permission(&stranger, &s(&env, "keepers"), &s(&env, "withdraw")),
        Err(Ok(Error::Unauthorized))
    );
    assert_eq!(
        client.try_grant_role(&stranger, &s(&env, "keepers"), &stranger),
        Err(Ok(Error::Unauthorized))
    );
    assert_eq!(
        client.try_revoke_role(&stranger, &s(&env, "keepers"), &member),
        Err(Ok(Error::Unauthorized))
    );
    assert_eq!(
        client.try_grant(&stranger, &member, &s(&env, "harvest")),
        Err(Ok(Error::Unauthorized))
    );

    client.revoke_role(&role_admin, &s(&env, "keepers"), &member);
    assert!(!client.has_permission(&member, &s(&env, "harvest")));
}

#[test]
fn role_admin_cannot_manage_other_roles_or_the_root() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let role_admin = Address::generate(&env);

    client.create_role(&admin, &s(&env, "keepers"), &role_admin);
    client.create_role(&admin, &s(&env, "auditors"), &admin);

    assert_eq!(
        client.try_grant_role_permission(&role_admin, &s(&env, "auditors"), &s(&env, "read")),
        Err(Ok(Error::Unauthorized))
    );
    // Nor can a role admin mint a role or hand out direct grants.
    assert_eq!(
        client.try_create_role(&role_admin, &s(&env, "rogue"), &role_admin),
        Err(Ok(Error::Unauthorized))
    );
    assert_eq!(
        client.try_set_role_admin(&role_admin, &s(&env, "keepers"), &role_admin),
        Err(Ok(Error::Unauthorized))
    );
}

#[test]
fn role_lifecycle_reports_missing_and_duplicate_roles() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);

    client.create_role(&admin, &s(&env, "keepers"), &admin);

    assert_eq!(
        client.try_grant_role(&admin, &s(&env, "nope"), &admin),
        Err(Ok(Error::RoleNotFound))
    );
    assert_eq!(
        client.try_revoke_role(&admin, &s(&env, "nope"), &admin),
        Err(Ok(Error::RoleNotFound))
    );
    assert_eq!(
        client.try_set_role_admin(&admin, &s(&env, "nope"), &admin),
        Err(Ok(Error::RoleNotFound))
    );
    assert_eq!(
        client.try_destroy_role(&admin, &s(&env, "nope")),
        Err(Ok(Error::RoleNotFound))
    );

    // Duplicate creation is rejected once the role exists.
    assert_eq!(
        client.try_create_role(&admin, &s(&env, "keepers"), &admin),
        Err(Ok(Error::RoleExists))
    );
}

#[test]
fn role_names_are_validated() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);

    assert_eq!(
        client.try_create_role(&admin, &s(&env, ""), &admin),
        Err(Ok(Error::InvalidRole))
    );
    assert_eq!(
        client.try_create_role(&admin, &s(&env, "keep*ers"), &admin),
        Err(Ok(Error::InvalidRole))
    );
    let long = "r".repeat(crate::MAX_SELECTOR_LEN as usize + 1);
    assert_eq!(
        client.try_create_role(&admin, &s(&env, &long), &admin),
        Err(Ok(Error::InvalidRole))
    );
}

#[test]
fn set_role_admin_rotates_control() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let old_admin = Address::generate(&env);
    let new_admin = Address::generate(&env);

    client.create_role(&admin, &s(&env, "keepers"), &old_admin);
    client.set_role_admin(&admin, &s(&env, "keepers"), &new_admin);
    assert_eq!(count_events(&env, symbol_short!("role_adm")), 1);

    assert_eq!(
        client.try_grant_role(&old_admin, &s(&env, "keepers"), &old_admin),
        Err(Ok(Error::Unauthorized))
    );
    client.grant_role(&new_admin, &s(&env, "keepers"), &new_admin);
    assert!(client.has_role(&new_admin, &s(&env, "keepers")));
}

#[test]
fn destroy_role_detaches_every_member_and_selector() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let a = Address::generate(&env);
    let b = Address::generate(&env);

    client.create_role(&admin, &s(&env, "keepers"), &admin);
    client.grant_role_permission(&admin, &s(&env, "keepers"), &s(&env, "harvest"));
    client.grant_role(&admin, &s(&env, "keepers"), &a);
    client.grant_role(&admin, &s(&env, "keepers"), &b);
    assert!(client.has_permission(&a, &s(&env, "harvest")));

    assert_eq!(client.destroy_role(&admin, &s(&env, "keepers")), 3);

    assert!(!client.role_exists(&s(&env, "keepers")));
    assert!(!client.has_permission(&a, &s(&env, "harvest")));
    assert!(!client.has_permission(&b, &s(&env, "harvest")));
    // Reverse indexes are cleaned up too.
    assert!(client.roles_of(&a).is_empty());
    assert!(client.members_of(&s(&env, "keepers")).is_empty());
    assert!(client.list_roles().is_empty());
}

#[test]
fn destroying_a_role_emits_one_event() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    client.create_role(&admin, &s(&env, "keepers"), &admin);

    client.destroy_role(&admin, &s(&env, "keepers"));
    assert_eq!(count_events(&env, symbol_short!("role_del")), 1);

    assert_eq!(
        client.try_destroy_role(&admin, &s(&env, "keepers")),
        Err(Ok(Error::RoleNotFound))
    );
    assert_eq!(count_events(&env, symbol_short!("role_del")), 0);
}

#[test]
fn destroy_role_releases_its_wildcard_usage() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let member = Address::generate(&env);

    client.register_wildcard(&admin, &s(&env, "vault.*"));
    client.create_role(&admin, &s(&env, "vault_ops"), &admin);
    client.grant_role_permission(&admin, &s(&env, "vault_ops"), &s(&env, "vault.*"));
    client.grant_role(&admin, &s(&env, "vault_ops"), &member);

    client.destroy_role(&admin, &s(&env, "vault_ops"));
    assert_eq!(client.wildcard_usage(&s(&env, "vault.*")), 0);
    assert!(client.unregister_wildcard(&admin, &s(&env, "vault.*")));
}

// ── Canonical Perigee roles ───────────────────────────────────────────────────

fn setup_protocol_roles(
    client: &AccessControlClient<'_>,
    env: &Env,
    root: &Address,
) -> [Address; 5] {
    let members = [
        Address::generate(env),
        Address::generate(env),
        Address::generate(env),
        Address::generate(env),
        Address::generate(env),
    ];
    client.setup_protocol_roles(
        root,
        &members[0],
        &members[1],
        &members[2],
        &members[3],
        &members[4],
    );
    members
}

#[test]
fn standard_roles_apply_granular_permissions() {
    let (env, id, root) = setup();
    let client = AccessControlClient::new(&env, &id);
    let [owner, admin, strategist, keeper, emergency] =
        setup_protocol_roles(&client, &env, &root);

    assert!(client.protocol_roles_configured());
    assert!(client.has_protocol_role(&owner, &ProtocolRole::Owner));
    assert!(client.has_protocol_role(&admin, &ProtocolRole::Admin));
    assert!(client.has_protocol_role(&strategist, &ProtocolRole::Strategist));
    assert!(client.has_protocol_role(&keeper, &ProtocolRole::Keeper));
    assert!(client.has_protocol_role(&emergency, &ProtocolRole::Emergency));

    assert!(client.has_permission(&owner, &s(&env, "vault.withdraw")));
    assert!(!client.has_permission(&owner, &s(&env, "strategy.rotate")));

    assert!(client.has_permission(&admin, &s(&env, "fees.configure")));
    assert!(!client.has_permission(&admin, &s(&env, "vault.withdraw")));

    assert!(client.has_permission(&strategist, &s(&env, "strategy.rotate")));
    assert!(!client.has_permission(&strategist, &s(&env, "fees.configure")));

    assert!(client.has_permission(&keeper, &s(&env, "strategy.harvest")));
    assert!(!client.has_permission(&keeper, &s(&env, "strategy.configure")));

    assert!(client.has_permission(&emergency, &s(&env, "emergency.pause")));
    assert!(client.has_permission(&emergency, &s(&env, "strategy.pause")));
    assert!(!client.has_permission(&emergency, &s(&env, "vault.unpause")));
}

#[test]
fn standard_roles_are_initialized_once_and_root_authorized() {
    let (env, id, root) = setup();
    let client = AccessControlClient::new(&env, &id);
    let stranger = Address::generate(&env);
    let members: [Address; 5] = core::array::from_fn(|_| Address::generate(&env));

    assert_eq!(
        client.try_setup_protocol_roles(
            &stranger,
            &members[0],
            &members[1],
            &members[2],
            &members[3],
            &members[4],
        ),
        Err(Ok(Error::Unauthorized))
    );
    client.setup_protocol_roles(
        &root,
        &members[0],
        &members[1],
        &members[2],
        &members[3],
        &members[4],
    );
    assert_eq!(
        client.try_setup_protocol_roles(
            &root,
            &members[0],
            &members[1],
            &members[2],
            &members[3],
            &members[4],
        ),
        Err(Ok(Error::ProtocolRolesAlreadyConfigured))
    );
    assert_eq!(client.list_roles().len(), 5);
}

#[test]
fn failed_standard_role_setup_is_atomic_when_role_limit_is_too_low() {
    let (env, id, root) = setup_with(AclConfig {
        max_roles: 4,
        ..AclConfig::default_config()
    });
    let client = AccessControlClient::new(&env, &id);
    let members: [Address; 5] = core::array::from_fn(|_| Address::generate(&env));

    assert_eq!(
        client.try_setup_protocol_roles(
            &root,
            &members[0],
            &members[1],
            &members[2],
            &members[3],
            &members[4],
        ),
        Err(Ok(Error::LimitExceeded))
    );
    assert!(!client.protocol_roles_configured());
    assert!(client.list_roles().is_empty());
}

#[test]
fn standard_role_permissions_cannot_be_escalated_by_operational_admin() {
    let (env, id, root) = setup();
    let client = AccessControlClient::new(&env, &id);
    let [_owner, admin, _strategist, _keeper, _emergency] =
        setup_protocol_roles(&client, &env, &root);

    assert_eq!(
        client.try_grant_role_permission(
            &admin,
            &s(&env, "strategist"),
            &s(&env, "*"),
        ),
        Err(Ok(Error::Unauthorized))
    );
    assert_eq!(
        client.try_grant_role(&admin, &s(&env, "keeper"), &admin),
        Err(Ok(Error::Unauthorized))
    );
}

// ── Batch updates ─────────────────────────────────────────────────────────────

#[test]
fn batch_grant_creates_every_pair_in_one_call() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);

    let entries = vec![
        &env,
        GrantEntry {
            target: alice.clone(),
            selector: s(&env, "harvest"),
        },
        GrantEntry {
            target: alice.clone(),
            selector: s(&env, "rebalance"),
        },
        GrantEntry {
            target: bob.clone(),
            selector: s(&env, "harvest"),
        },
    ];

    assert_eq!(client.batch_grant(&admin, &entries), 3);
    // All three come from one invocation, so they are all visible at once.
    assert_eq!(count_events(&env, symbol_short!("grant")), 3);

    assert!(client.has_permission(&alice, &s(&env, "harvest")));
    assert!(client.has_permission(&alice, &s(&env, "rebalance")));
    assert!(client.has_permission(&bob, &s(&env, "harvest")));
    assert!(!client.has_permission(&bob, &s(&env, "rebalance")));
    assert_eq!(client.grants_of(&alice).len(), 2);
}

#[test]
fn batch_grant_is_idempotent() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let alice = Address::generate(&env);
    let entries = vec![
        &env,
        GrantEntry {
            target: alice.clone(),
            selector: s(&env, "harvest"),
        },
        GrantEntry {
            target: alice.clone(),
            selector: s(&env, "rebalance"),
        },
    ];

    assert_eq!(client.batch_grant(&admin, &entries), 2);
    assert_eq!(client.batch_grant(&admin, &entries), 0);
    assert_eq!(client.grants_of(&alice).len(), 2);
}

#[test]
fn batch_revoke_removes_every_pair_and_tolerates_absent_ones() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let alice = Address::generate(&env);
    let entries = vec![
        &env,
        GrantEntry {
            target: alice.clone(),
            selector: s(&env, "harvest"),
        },
        GrantEntry {
            target: alice.clone(),
            selector: s(&env, "rebalance"),
        },
    ];
    client.batch_grant(&admin, &entries);

    assert_eq!(client.batch_revoke(&admin, &entries), 2);
    assert_eq!(client.batch_revoke(&admin, &entries), 0);
    assert!(client.grants_of(&alice).is_empty());
    assert!(!client.has_permission(&alice, &s(&env, "harvest")));
}

#[test]
fn batch_size_bounds_are_enforced() {
    let (env, id, admin) = setup_with(cfg_with(2));
    let client = AccessControlClient::new(&env, &id);
    let alice = Address::generate(&env);

    assert_eq!(
        client.try_batch_grant(&admin, &Vec::new(&env)),
        Err(Ok(Error::EmptyBatch))
    );
    assert_eq!(
        client.try_batch_revoke(&admin, &Vec::new(&env)),
        Err(Ok(Error::EmptyBatch))
    );

    let three = vec![
        &env,
        GrantEntry {
            target: alice.clone(),
            selector: s(&env, "a"),
        },
        GrantEntry {
            target: alice.clone(),
            selector: s(&env, "b"),
        },
        GrantEntry {
            target: alice.clone(),
            selector: s(&env, "c"),
        },
    ];
    assert_eq!(
        client.try_batch_grant(&admin, &three),
        Err(Ok(Error::BatchTooLarge))
    );
    assert_eq!(
        client.try_batch_revoke(&admin, &three),
        Err(Ok(Error::BatchTooLarge))
    );
    // The rejected batch left nothing behind.
    assert!(client.grants_of(&alice).is_empty());
}

#[test]
fn batch_updates_require_the_root_admin() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let stranger = Address::generate(&env);
    let entries = vec![
        &env,
        GrantEntry {
            target: stranger.clone(),
            selector: s(&env, "harvest"),
        },
    ];

    assert_eq!(
        client.try_batch_grant(&stranger, &entries),
        Err(Ok(Error::Unauthorized))
    );
    assert_eq!(
        client.try_batch_revoke(&stranger, &entries),
        Err(Ok(Error::Unauthorized))
    );

    // The root admin succeeds where the stranger was refused.
    assert_eq!(client.batch_grant(&admin, &entries), 1);
    assert!(client.has_permission(&stranger, &s(&env, "harvest")));
}

#[test]
fn batch_set_members_replaces_the_whole_roster() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let c = Address::generate(&env);

    client.create_role(&admin, &s(&env, "keepers"), &admin);
    client.grant_role_permission(&admin, &s(&env, "keepers"), &s(&env, "harvest"));
    client.batch_set_members(&admin, &s(&env, "keepers"), &vec![&env, a.clone()]);

    assert!(client.has_role(&a, &s(&env, "keepers")));
    assert!(!client.has_role(&b, &s(&env, "keepers")));

    // Swap `a` out for `b` and `c` in a single transaction.
    assert_eq!(
        client.batch_set_members(
            &admin,
            &s(&env, "keepers"),
            &vec![&env, b.clone(), c.clone()]
        ),
        2
    );
    assert!(!client.has_role(&a, &s(&env, "keepers")));
    assert!(client.has_role(&b, &s(&env, "keepers")));
    assert!(client.has_role(&c, &s(&env, "keepers")));
    // The detached address no longer inherits the role's permissions, and its
    // reverse index was cleaned up.
    assert!(!client.has_permission(&a, &s(&env, "harvest")));
    assert!(client.roles_of(&a).is_empty());
    assert_eq!(client.members_of(&s(&env, "keepers")).len(), 2);
}

#[test]
fn batch_set_permissions_replaces_the_whole_selector_set() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let member = Address::generate(&env);

    client.create_role(&admin, &s(&env, "keepers"), &admin);
    client.batch_set_members(&admin, &s(&env, "keepers"), &vec![&env, member.clone()]);
    client.batch_set_permissions(
        &admin,
        &s(&env, "keepers"),
        &vec![&env, s(&env, "harvest"), s(&env, "rebalance")],
    );
    assert!(client.has_permission(&member, &s(&env, "rebalance")));

    assert_eq!(
        client.batch_set_permissions(&admin, &s(&env, "keepers"), &vec![&env, s(&env, "harvest")]),
        1
    );
    assert!(client.has_permission(&member, &s(&env, "harvest")));
    assert!(!client.has_permission(&member, &s(&env, "rebalance")));
    assert_eq!(client.permissions_of(&s(&env, "keepers")).len(), 1);
}

#[test]
fn batch_role_updates_reject_empty_and_oversized_batches() {
    let (env, id, admin) = setup_with(cfg_with(2));
    let client = AccessControlClient::new(&env, &id);
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let c = Address::generate(&env);
    client.create_role(&admin, &s(&env, "keepers"), &admin);

    assert_eq!(
        client.try_batch_set_members(&admin, &s(&env, "keepers"), &Vec::new(&env)),
        Err(Ok(Error::EmptyBatch))
    );
    assert_eq!(
        client.try_batch_set_permissions(&admin, &s(&env, "keepers"), &Vec::new(&env)),
        Err(Ok(Error::EmptyBatch))
    );
    assert_eq!(
        client.try_batch_set_members(&admin, &s(&env, "keepers"), &vec![&env, a, b, c]),
        Err(Ok(Error::BatchTooLarge))
    );
    assert_eq!(
        client.try_batch_set_permissions(
            &admin,
            &s(&env, "keepers"),
            &vec![&env, s(&env, "a"), s(&env, "b"), s(&env, "c")]
        ),
        Err(Ok(Error::BatchTooLarge))
    );
}

#[test]
fn batch_role_updates_check_existence_and_authority() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let stranger = Address::generate(&env);
    let a = Address::generate(&env);
    client.create_role(&admin, &s(&env, "keepers"), &admin);

    assert_eq!(
        client.try_batch_set_members(&admin, &s(&env, "nope"), &vec![&env, a.clone()]),
        Err(Ok(Error::RoleNotFound))
    );
    assert_eq!(
        client.try_batch_set_permissions(&admin, &s(&env, "nope"), &vec![&env, s(&env, "harvest")]),
        Err(Ok(Error::RoleNotFound))
    );
    assert_eq!(
        client.try_batch_set_members(&stranger, &s(&env, "keepers"), &vec![&env, a.clone()]),
        Err(Ok(Error::Unauthorized))
    );
    assert_eq!(
        client.try_batch_set_permissions(&stranger, &s(&env, "keepers"), &vec![&env, s(&env, "x")]),
        Err(Ok(Error::Unauthorized))
    );
    assert!(client.members_of(&s(&env, "keepers")).is_empty());
}

#[test]
fn batch_set_permissions_applies_wildcards() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let member = Address::generate(&env);

    client.register_wildcard(&admin, &s(&env, "vault.*"));
    client.create_role(&admin, &s(&env, "vault_ops"), &admin);
    client.batch_set_members(&admin, &s(&env, "vault_ops"), &vec![&env, member.clone()]);
    client.batch_set_permissions(
        &admin,
        &s(&env, "vault_ops"),
        &vec![&env, s(&env, "vault.*")],
    );

    assert!(client.has_permission(&member, &s(&env, "vault.withdraw")));
    assert_eq!(client.wildcard_usage(&s(&env, "vault.*")), 1);

    // Re-registering the same selectors changes nothing.
    assert_eq!(
        client.batch_set_permissions(
            &admin,
            &s(&env, "vault_ops"),
            &vec![&env, s(&env, "vault.*")]
        ),
        1
    );
    assert_eq!(client.wildcard_usage(&s(&env, "vault.*")), 1);
}

// ── Limits ────────────────────────────────────────────────────────────────────

#[test]
fn grants_per_address_are_bounded() {
    let (env, id, admin) = setup_with(AclConfig {
        max_grants_per_address: 2,
        ..AclConfig::default_config()
    });
    let client = AccessControlClient::new(&env, &id);
    let keeper = Address::generate(&env);
    let other = Address::generate(&env);

    client.grant(&admin, &keeper, &s(&env, "a"));
    client.grant(&admin, &keeper, &s(&env, "b"));
    assert_eq!(
        client.try_grant(&admin, &keeper, &s(&env, "c")),
        Err(Ok(Error::LimitExceeded))
    );
    // The bound is per address, not global.
    client.grant(&admin, &other, &s(&env, "c"));
    assert_eq!(client.grants_of(&keeper).len(), 2);

    // Re-granting an existing selector is a no-op, not a limit breach.
    assert!(!client.grant(&admin, &keeper, &s(&env, "a")));
}

#[test]
fn the_number_of_roles_is_bounded() {
    let (env, id, admin) = setup_with(AclConfig {
        max_roles: 1,
        ..AclConfig::default_config()
    });
    let client = AccessControlClient::new(&env, &id);

    client.create_role(&admin, &s(&env, "one"), &admin);
    assert_eq!(
        client.try_create_role(&admin, &s(&env, "two"), &admin),
        Err(Ok(Error::LimitExceeded))
    );
}

#[test]
fn permissions_per_role_are_bounded() {
    let (env, id, admin) = setup_with(AclConfig {
        max_permissions_per_role: 1,
        ..AclConfig::default_config()
    });
    let client = AccessControlClient::new(&env, &id);

    client.create_role(&admin, &s(&env, "keepers"), &admin);
    client.grant_role_permission(&admin, &s(&env, "keepers"), &s(&env, "harvest"));
    assert_eq!(
        client.try_grant_role_permission(&admin, &s(&env, "keepers"), &s(&env, "rebalance")),
        Err(Ok(Error::LimitExceeded))
    );
    assert!(!client.grant_role_permission(&admin, &s(&env, "keepers"), &s(&env, "harvest")));
}

#[test]
fn members_per_role_are_bounded() {
    let (env, id, admin) = setup_with(AclConfig {
        max_members_per_role: 1,
        ..AclConfig::default_config()
    });
    let client = AccessControlClient::new(&env, &id);
    let a = Address::generate(&env);
    let b = Address::generate(&env);

    client.create_role(&admin, &s(&env, "keepers"), &admin);
    client.grant_role(&admin, &s(&env, "keepers"), &a);
    assert_eq!(
        client.try_grant_role(&admin, &s(&env, "keepers"), &b),
        Err(Ok(Error::LimitExceeded))
    );
    assert!(!client.grant_role(&admin, &s(&env, "keepers"), &a));
}

#[test]
fn roles_per_address_are_bounded() {
    let (env, id, admin) = setup_with(AclConfig {
        max_roles_per_address: 1,
        ..AclConfig::default_config()
    });
    let client = AccessControlClient::new(&env, &id);
    let member = Address::generate(&env);

    client.create_role(&admin, &s(&env, "one"), &admin);
    client.create_role(&admin, &s(&env, "two"), &admin);
    client.grant_role(&admin, &s(&env, "one"), &member);
    assert_eq!(
        client.try_grant_role(&admin, &s(&env, "two"), &member),
        Err(Ok(Error::LimitExceeded))
    );
    // The rejected membership left the reverse index untouched.
    assert_eq!(client.roles_of(&member), vec![&env, s(&env, "one")]);
}

// ── Renounce ──────────────────────────────────────────────────────────────────

#[test]
fn renounce_drops_one_of_the_callers_own_grants() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let keeper = Address::generate(&env);
    client.grant(&admin, &keeper, &s(&env, "harvest"));
    client.grant(&admin, &keeper, &s(&env, "rebalance"));

    assert!(client.renounce(&keeper, &s(&env, "harvest")));
    assert!(!client.renounce(&keeper, &s(&env, "harvest")));
    assert!(!client.has_permission(&keeper, &s(&env, "harvest")));
    assert!(client.has_permission(&keeper, &s(&env, "rebalance")));
}

#[test]
fn renounce_all_returns_an_address_to_zero_privilege() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let operator = Address::generate(&env);

    client.grant(&admin, &operator, &s(&env, "harvest"));
    client.grant(&admin, &operator, &s(&env, "rebalance"));
    client.create_role(&admin, &s(&env, "keepers"), &admin);
    client.create_role(&admin, &s(&env, "auditors"), &admin);
    client.grant_role(&admin, &s(&env, "keepers"), &operator);
    client.grant_role(&admin, &s(&env, "auditors"), &operator);

    // 2 grants + 2 memberships.
    assert_eq!(client.renounce_all(&operator), 4);

    assert!(!client.has_permission(&operator, &s(&env, "harvest")));
    assert!(!client.has_permission(&operator, &s(&env, "rebalance")));
    assert!(client.grants_of(&operator).is_empty());
    assert!(client.roles_of(&operator).is_empty());
    assert!(client.members_of(&s(&env, "keepers")).is_empty());
    assert!(!client.has_role(&operator, &s(&env, "auditors")));
}

#[test]
fn renounce_all_releases_wildcard_usage_and_spares_the_root_admin() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let operator = Address::generate(&env);

    client.register_wildcard(&admin, &s(&env, "vault.*"));
    client.grant(&admin, &admin, &s(&env, "vault.*"));
    client.grant(&admin, &operator, &s(&env, "vault.*"));
    assert_eq!(client.wildcard_usage(&s(&env, "vault.*")), 2);

    client.renounce_all(&operator);
    assert_eq!(client.wildcard_usage(&s(&env, "vault.*")), 1);

    // Even after renouncing everything, the root admin keeps full authority.
    client.renounce_all(&admin);
    assert!(client.has_permission(&admin, &s(&env, "anything")));
    assert!(client.grant(&admin, &operator, &s(&env, "harvest")));

    client.revoke(&admin, &admin, &s(&env, "vault.*"));
    assert!(client.unregister_wildcard(&admin, &s(&env, "vault.*")));
}

#[test]
fn renounce_role_drops_a_single_membership() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let member = Address::generate(&env);
    client.create_role(&admin, &s(&env, "keepers"), &admin);
    client.grant_role_permission(&admin, &s(&env, "keepers"), &s(&env, "harvest"));
    client.grant_role(&admin, &s(&env, "keepers"), &member);

    assert!(client.renounce_role(&member, &s(&env, "keepers")));
    assert!(!client.renounce_role(&member, &s(&env, "keepers")));
    assert!(!client.has_permission(&member, &s(&env, "harvest")));
    assert!(client.roles_of(&member).is_empty());
}

#[test]
fn renounce_emits_one_event_per_change() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let keeper = Address::generate(&env);
    client.grant(&admin, &keeper, &s(&env, "harvest"));
    client.create_role(&admin, &s(&env, "keepers"), &admin);
    client.grant_role(&admin, &s(&env, "keepers"), &keeper);

    client.renounce(&keeper, &s(&env, "harvest"));
    assert_eq!(count_events(&env, symbol_short!("grant")), 1);
    assert_eq!(count_events(&env, symbol_short!("role_memb")), 0);

    // The only thing left is the role membership, so that is the only change
    // `renounce_all` reports — the role's own selectors are untouched.
    client.renounce_all(&keeper);
    assert_eq!(count_events(&env, symbol_short!("grant")), 0);
    assert_eq!(count_events(&env, symbol_short!("role_memb")), 1);
}

// ── Lockdown ──────────────────────────────────────────────────────────────────

#[test]
fn lockdown_freezes_mutations_but_keeps_queries_and_release() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let keeper = Address::generate(&env);
    let target = Address::generate(&env);
    client.grant(&admin, &keeper, &s(&env, "harvest"));
    client.create_role(&admin, &s(&env, "keepers"), &admin);

    client.set_lockdown(&admin, &true);
    assert_eq!(count_events(&env, symbol_short!("lockdown")), 1);
    assert!(client.is_lockdown());

    // Reads keep working.
    assert!(client.has_permission(&keeper, &s(&env, "harvest")));
    assert_eq!(client.admin(), admin);

    let entries = vec![
        &env,
        GrantEntry {
            target: target.clone(),
            selector: s(&env, "harvest"),
        },
    ];
    // Boolean-returning mutations.
    for result in [
        client.try_grant(&admin, &target, &s(&env, "harvest")),
        client.try_revoke(&admin, &keeper, &s(&env, "harvest")),
        client.try_grant_role(&admin, &s(&env, "keepers"), &target),
        client.try_grant_role_permission(&admin, &s(&env, "keepers"), &s(&env, "x")),
        client.try_register_wildcard(&admin, &s(&env, "vault.*")),
        client.try_unregister_wildcard(&admin, &s(&env, "vault.*")),
    ] {
        assert_eq!(result, Err(Ok(Error::Locked)));
    }

    // Unit-returning mutations.
    assert_eq!(
        client.try_create_role(&admin, &s(&env, "new"), &admin),
        Err(Ok(Error::Locked))
    );
    assert_eq!(
        client.try_set_role_admin(&admin, &s(&env, "keepers"), &target),
        Err(Ok(Error::Locked))
    );

    // Count- and unit-returning mutations.
    assert_eq!(
        client.try_batch_grant(&admin, &entries),
        Err(Ok(Error::Locked))
    );
    assert_eq!(
        client.try_batch_revoke(&admin, &entries),
        Err(Ok(Error::Locked))
    );
    assert_eq!(
        client.try_destroy_role(&admin, &s(&env, "keepers")),
        Err(Ok(Error::Locked))
    );
    assert_eq!(
        client.try_batch_set_members(&admin, &s(&env, "keepers"), &vec![&env, target.clone()]),
        Err(Ok(Error::Locked))
    );
    assert_eq!(
        client.try_batch_set_permissions(&admin, &s(&env, "keepers"), &vec![&env, s(&env, "x")]),
        Err(Ok(Error::Locked))
    );
    assert_eq!(
        client.try_set_config(&admin, &AclConfig::default_config()),
        Err(Ok(Error::Locked))
    );

    // The freeze is escapable: `set_lockdown` is deliberately *not* gated
    // by the lockdown, or the protocol could never be unfrozen.
    client.set_lockdown(&admin, &false);
    assert!(!client.is_lockdown());

    // `transfer_admin` survives lockdown too, so a compromised root key can be
    // rotated without first reopening the protocol to writes. Authority moves
    // with it, so the handover address is the one that can unfreeze.
    client.set_lockdown(&admin, &true);
    client.transfer_admin(&admin, &target);
    assert_eq!(client.admin(), target);

    client.set_lockdown(&target, &false);
    client.grant(&target, &target, &s(&env, "harvest"));
    assert!(client.has_permission(&target, &s(&env, "harvest")));
}

#[test]
fn renounce_stays_available_during_lockdown() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let keeper = Address::generate(&env);
    client.grant(&admin, &keeper, &s(&env, "harvest"));
    client.grant(&admin, &keeper, &s(&env, "rebalance"));
    client.create_role(&admin, &s(&env, "keepers"), &admin);
    client.grant_role(&admin, &s(&env, "keepers"), &keeper);
    client.set_lockdown(&admin, &true);

    assert!(client.renounce(&keeper, &s(&env, "harvest")));
    assert!(client.renounce_role(&keeper, &s(&env, "keepers")));
    assert_eq!(client.renounce_all(&keeper), 1);
    assert!(!client.has_permission(&keeper, &s(&env, "rebalance")));
}

#[test]
fn lockdown_requires_the_root_admin() {
    let (env, id, _admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let stranger = Address::generate(&env);
    assert_eq!(
        client.try_set_lockdown(&stranger, &true),
        Err(Ok(Error::Unauthorized))
    );
    assert!(!client.is_lockdown());
}

// ── Root admin rotation & configuration ───────────────────────────────────────

#[test]
fn transfer_admin_moves_all_management_authority() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let new_admin = Address::generate(&env);
    let keeper = Address::generate(&env);

    client.transfer_admin(&admin, &new_admin);
    assert_eq!(count_events(&env, symbol_short!("admin")), 1);
    assert_eq!(client.admin(), new_admin);

    // The old admin loses management authority, and — because it never held an
    // ACL grant of its own — it loses the root bypass with it. Rotation is a
    // clean handover, not a demotion to "still allowed".
    assert_eq!(
        client.try_grant(&admin, &keeper, &s(&env, "harvest")),
        Err(Ok(Error::Unauthorized))
    );
    assert!(!client.has_permission(&admin, &s(&env, "harvest")));

    // The new admin holds the same authority, again without any ACL grant.
    assert!(client.has_permission(&new_admin, &s(&env, "harvest")));
    assert!(client.grant(&new_admin, &keeper, &s(&env, "harvest")));
    assert!(client.has_permission(&keeper, &s(&env, "harvest")));
}

#[test]
fn transfer_admin_rejects_non_admins() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let stranger = Address::generate(&env);
    assert_eq!(
        client.try_transfer_admin(&stranger, &stranger),
        Err(Ok(Error::Unauthorized))
    );
    assert_eq!(client.admin(), admin);
}

#[test]
fn set_config_updates_bounds() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let tighter = AclConfig {
        max_batch_size: 1,
        max_wildcards: 1,
        ..AclConfig::default_config()
    };

    client.set_config(&admin, &tighter);
    assert_eq!(client.config(), tighter);
    assert_eq!(
        client.try_set_config(
            &admin,
            &AclConfig {
                max_batch_size: 0,
                ..AclConfig::default_config()
            }
        ),
        Err(Ok(Error::LimitExceeded))
    );
    assert_eq!(
        client.try_set_config(&Address::generate(&env), &AclConfig::default_config()),
        Err(Ok(Error::Unauthorized))
    );
    assert_eq!(client.config(), tighter);
}

// ── Regression: a revoked grant must not survive via a stale index ────────────

#[test]
fn indexes_stay_consistent_across_mixed_operations() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let actor = Address::generate(&env);

    client.register_wildcard(&admin, &s(&env, "vault.*"));
    client.create_role(&admin, &s(&env, "vault_ops"), &admin);
    client.grant_role_permission(&admin, &s(&env, "vault_ops"), &s(&env, "vault.*"));

    // Direct grant, role membership, then revoke only the direct grant.
    client.grant(&admin, &actor, &s(&env, "vault.deposit"));
    client.grant_role(&admin, &s(&env, "vault_ops"), &actor);
    assert!(client.has_permission(&actor, &s(&env, "vault.deposit")));

    client.revoke(&admin, &actor, &s(&env, "vault.deposit"));
    // The direct grant is gone from the index…
    assert_eq!(client.grants_of(&actor), Vec::new(&env));
    assert!(!client.is_granted(&actor, &s(&env, "vault.deposit")));
    // …but the role still covers this selector, so the effective answer is
    // unchanged. Exact indexing and effective permission are distinct facts.
    assert!(client.has_permission(&actor, &s(&env, "vault.deposit")));
    assert!(client.has_permission(&actor, &s(&env, "vault.withdraw")));

    client.revoke_role(&admin, &s(&env, "vault_ops"), &actor);
    assert!(!client.has_permission(&actor, &s(&env, "vault.deposit")));
    assert!(!client.has_permission(&actor, &s(&env, "vault.withdraw")));

    // Counts line up on both sides once everything is undone.
    client.revoke_role_permission(&admin, &s(&env, "vault_ops"), &s(&env, "vault.*"));
    assert_eq!(client.wildcard_usage(&s(&env, "vault.*")), 0);
    client.unregister_wildcard(&admin, &s(&env, "vault.*"));
    assert!(client.list_wildcards().is_empty());
}

// ── Events ────────────────────────────────────────────────────────────────────

#[test]
fn role_permission_mutation_requires_an_existing_role() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let member = Address::generate(&env);

    assert_eq!(
        client.try_grant_role_permission(&admin, &s(&env, "nope"), &s(&env, "harvest")),
        Err(Ok(Error::RoleNotFound))
    );
    assert_eq!(
        client.try_revoke_role_permission(&admin, &s(&env, "nope"), &s(&env, "harvest")),
        Err(Ok(Error::RoleNotFound))
    );

    // Revoking a selector the role never had is a no-op, not an error.
    client.create_role(&admin, &s(&env, "keepers"), &admin);
    client.grant_role_permission(&admin, &s(&env, "keepers"), &s(&env, "harvest"));
    assert!(!client.revoke_role_permission(&admin, &s(&env, "keepers"), &s(&env, "rebalance")));
    assert!(client.revoke_role_permission(&admin, &s(&env, "keepers"), &s(&env, "harvest")));
    assert!(client.permissions_of(&s(&env, "keepers")).is_empty());
    assert!(!client.has_permission(&member, &s(&env, "harvest")));
}

#[test]
fn unregistering_a_non_namespace_pattern_is_rejected() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);

    for bad in ["deposit", "*", ""] {
        assert_eq!(
            client.try_unregister_wildcard(&admin, &s(&env, bad)),
            Err(Ok(Error::InvalidSelector)),
            "expected {bad} to be rejected"
        );
    }
}

#[test]
fn every_mutation_publishes_an_auditable_event() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let member = Address::generate(&env);
    let role_admin = Address::generate(&env);

    // The test host only exposes the latest invocation's events, so a running
    // total has to be accumulated as each call is made.
    let mut role_new = 0;
    let mut role_adm = 0;
    let mut role_perm = 0;
    let mut role_memb = 0;
    let mut role_del = 0;
    let mut grant = 0;
    let mut config = 0;

    let mut tally = |env: &Env| {
        role_new += count_events(env, symbol_short!("role_new"));
        role_adm += count_events(env, symbol_short!("role_adm"));
        role_perm += count_events(env, symbol_short!("role_perm"));
        role_memb += count_events(env, symbol_short!("role_memb"));
        role_del += count_events(env, symbol_short!("role_del"));
        grant += count_events(env, symbol_short!("grant"));
        config += count_events(env, symbol_short!("config"));
    };

    tally(&env); // initialize
    client.create_role(&admin, &s(&env, "keepers"), &role_admin.clone());
    tally(&env);
    client.set_role_admin(&admin, &s(&env, "keepers"), &role_admin);
    tally(&env);
    client.grant_role_permission(&admin, &s(&env, "keepers"), &s(&env, "harvest"));
    tally(&env);
    client.revoke_role_permission(&admin, &s(&env, "keepers"), &s(&env, "harvest"));
    tally(&env);
    client.grant_role(&admin, &s(&env, "keepers"), &member);
    tally(&env);
    client.revoke_role(&admin, &s(&env, "keepers"), &member);
    tally(&env);
    client.grant(&admin, &member, &s(&env, "harvest"));
    tally(&env);
    client.revoke(&admin, &member, &s(&env, "harvest"));
    tally(&env);
    client.destroy_role(&admin, &s(&env, "keepers"));
    tally(&env);

    assert_eq!(config, 1);
    assert_eq!(role_new, 1);
    assert_eq!(role_adm, 1);
    assert_eq!(role_perm, 2);
    assert_eq!(role_memb, 2);
    assert_eq!(role_del, 1);
    assert_eq!(grant, 2);
}

#[test]
fn event_topics_are_stable() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    client.grant(&admin, &Address::generate(&env), &s(&env, "harvest"));

    let all = env.events().all();
    let (_, topics, _) = all.last().unwrap();
    assert_eq!(topic_at(&env, &topics, 0), Some(symbol_short!("acl")));
    assert_eq!(topic_at(&env, &topics, 1), Some(symbol_short!("grant")));
}
