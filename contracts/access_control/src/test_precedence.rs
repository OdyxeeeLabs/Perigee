#![cfg(test)]

//! Selector and wildcard precedence matrix (#870).
//!
//! The ACL is purely **additive**: there are no deny entries, so "precedence"
//! reduces to *any matching allow wins, and the absence of every match is a
//! denial*. These tests pin that model across exact, namespace, universal and
//! role-based grants, for several principals at once, so that no revoke or
//! overlap can silently widen access.

use soroban_sdk::{testutils::Address as _, Address, Env, String};

use crate::{AccessControl, AccessControlClient, AclConfig};

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let id = env.register(AccessControl, ());
    let client = AccessControlClient::new(&env, &id);
    client.initialize(&admin, &AclConfig::default_config());
    (env, id, admin)
}

fn s(env: &Env, text: &str) -> String {
    String::from_str(env, text)
}

#[test]
fn missing_rule_is_a_denial_for_every_selector_shape() {
    let (env, id, _admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let nobody = Address::generate(&env);
    for sel in ["deposit", "vault.deposit", "*", "vault.*"] {
        assert!(!client.has_permission(&nobody, &s(&env, sel)), "{sel}");
    }
}

#[test]
fn exact_grant_does_not_imply_any_wildcard_or_sibling() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let alice = Address::generate(&env);
    client.grant(&admin, &alice, &s(&env, "vault.deposit"));

    assert!(client.has_permission(&alice, &s(&env, "vault.deposit")));
    assert!(!client.has_permission(&alice, &s(&env, "vault.withdraw")));
    assert!(!client.has_permission(&alice, &s(&env, "vault.")));
    assert!(!client.has_permission(&alice, &s(&env, "vault.*")));
    assert!(!client.has_permission(&alice, &s(&env, "*")));
}

#[test]
fn wildcard_grant_covers_selectors_without_an_exact_rule() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let alice = Address::generate(&env);
    client.register_wildcard(&admin, &s(&env, "vault.*"));
    client.grant(&admin, &alice, &s(&env, "vault.*"));

    assert!(client.has_permission(&alice, &s(&env, "vault.deposit")));
    assert!(client.has_permission(&alice, &s(&env, "vault.anything.nested")));
    // The wildcard does not leak to other namespaces or to the bare prefix word.
    assert!(!client.has_permission(&alice, &s(&env, "vault")));
    assert!(!client.has_permission(&alice, &s(&env, "strategy.harvest")));
}

#[test]
fn revoking_an_exact_grant_does_not_remove_access_held_through_a_wildcard() {
    // There is no deny entry: an exact rule never *narrows* a wildcard. To take
    // access away from a wildcard holder the wildcard itself must be revoked.
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let alice = Address::generate(&env);
    client.register_wildcard(&admin, &s(&env, "vault.*"));
    client.grant(&admin, &alice, &s(&env, "vault.*"));
    client.grant(&admin, &alice, &s(&env, "vault.withdraw"));

    client.revoke(&admin, &alice, &s(&env, "vault.withdraw"));
    assert!(client.has_permission(&alice, &s(&env, "vault.withdraw")));

    client.revoke(&admin, &alice, &s(&env, "vault.*"));
    assert!(!client.has_permission(&alice, &s(&env, "vault.withdraw")));
}

#[test]
fn revoking_a_wildcard_keeps_independent_exact_grants() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let alice = Address::generate(&env);
    client.register_wildcard(&admin, &s(&env, "vault.*"));
    client.grant(&admin, &alice, &s(&env, "vault.*"));
    client.grant(&admin, &alice, &s(&env, "vault.withdraw"));

    client.revoke(&admin, &alice, &s(&env, "vault.*"));
    assert!(client.has_permission(&alice, &s(&env, "vault.withdraw")));
    assert!(!client.has_permission(&alice, &s(&env, "vault.deposit")));
}

#[test]
fn universal_wildcard_covers_everything_and_is_not_widened_by_namespaces() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    client.grant(&admin, &alice, &s(&env, "*"));
    client.register_wildcard(&admin, &s(&env, "vault.*"));
    client.grant(&admin, &bob, &s(&env, "vault.*"));

    for sel in ["deposit", "vault.deposit", "strategy.harvest"] {
        assert!(client.has_permission(&alice, &s(&env, sel)), "alice {sel}");
    }
    // A namespace wildcard never behaves like the universal one.
    assert!(!client.has_permission(&bob, &s(&env, "strategy.harvest")));
}

#[test]
fn principals_do_not_share_rules() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    let carol = Address::generate(&env);
    client.register_wildcard(&admin, &s(&env, "vault.*"));

    client.grant(&admin, &alice, &s(&env, "vault.*"));
    client.grant(&admin, &bob, &s(&env, "vault.deposit"));

    assert!(client.has_permission(&alice, &s(&env, "vault.withdraw")));
    assert!(!client.has_permission(&bob, &s(&env, "vault.withdraw")));
    assert!(client.has_permission(&bob, &s(&env, "vault.deposit")));
    assert!(!client.has_permission(&carol, &s(&env, "vault.deposit")));

    // Revoking Alice's wildcard leaves Bob's exact grant alone.
    client.revoke(&admin, &alice, &s(&env, "vault.*"));
    assert!(!client.has_permission(&alice, &s(&env, "vault.deposit")));
    assert!(client.has_permission(&bob, &s(&env, "vault.deposit")));
}

#[test]
fn role_and_direct_rules_combine_additively_and_revoke_independently() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let alice = Address::generate(&env);
    let role = s(&env, "keeper");
    client.create_role(&admin, &role, &admin);
    client.grant_role_permission(&admin, &role, &s(&env, "strategy.harvest"));
    client.grant_role(&admin, &role, &alice);
    client.grant(&admin, &alice, &s(&env, "strategy.harvest"));

    // Held both directly and through the role; dropping one keeps the other.
    client.revoke(&admin, &alice, &s(&env, "strategy.harvest"));
    assert!(client.has_permission(&alice, &s(&env, "strategy.harvest")));
    client.revoke_role(&admin, &role, &alice);
    assert!(!client.has_permission(&alice, &s(&env, "strategy.harvest")));
}

#[test]
fn unregistering_an_in_use_wildcard_is_refused_and_access_is_unchanged() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let alice = Address::generate(&env);
    client.register_wildcard(&admin, &s(&env, "vault.*"));
    client.grant(&admin, &alice, &s(&env, "vault.*"));
    assert!(client.has_permission(&alice, &s(&env, "vault.deposit")));

    // Unregistering is refused while the pattern is in use...
    assert!(client.try_unregister_wildcard(&admin, &s(&env, "vault.*")).is_err());
    // ...and the permission still holds, i.e. no half-removed state.
    assert!(client.has_permission(&alice, &s(&env, "vault.deposit")));
}

#[test]
fn malformed_and_probe_selectors_never_widen_access() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    let alice = Address::generate(&env);
    client.grant(&admin, &alice, &s(&env, "*"));
    // Even a universal grant is denied for malformed selectors: a bad argument is
    // never an authorization.
    for sel in ["", "a*b", "*x", "a*b*"] {
        assert!(!client.has_permission(&alice, &s(&env, sel)), "{sel:?}");
    }
}

#[test]
fn the_root_admin_is_allowed_without_any_grant() {
    let (env, id, admin) = setup();
    let client = AccessControlClient::new(&env, &id);
    assert!(client.has_permission(&admin, &s(&env, "anything.at.all")));
}
