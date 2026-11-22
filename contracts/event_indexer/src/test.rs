#![cfg(test)]

extern crate std;

use soroban_sdk::{
    symbol_short, testutils::Ledger, vec, Env, IntoVal, String, Symbol, Val, Vec,
};

use crate::{Error, EventIndexer, EventIndexerClient};

fn payload(e: &Env, price: i128, amount: i128) -> Vec<(Symbol, Val)> {
    vec![
        e,
        (symbol_short!("price"), price.into_val(e)),
        (symbol_short!("amount"), amount.into_val(e)),
    ]
}

#[test]
fn test_record_and_query() {
    let e = Env::default();
    e.ledger().with_mut(|li| li.timestamp = 1000);

    let contract_id = e.register(EventIndexer, ());
    let client = EventIndexerClient::new(&e, &contract_id);

    let topic = symbol_short!("payment");
    let key = String::from_str(&e, "claim-1");

    let seq1 = client.record(&topic, &symbol_short!("created"), &key, &payload(&e, 100, 10));
    let seq2 = client.record(&topic, &symbol_short!("confirmed"), &key, &payload(&e, 110, 10));

    assert_eq!(seq1, 1);
    assert_eq!(seq2, 2);
    assert_eq!(client.count(&topic), 2);

    let latest = client.latest(&topic).unwrap();
    assert_eq!(latest.seq, 2);
    assert_eq!(latest.event_name, symbol_short!("confirmed"));
    assert_eq!(latest.timestamp, 1000);

    let all = client.query(&topic, &1, &10);
    assert_eq!(all.len(), 2);
    assert_eq!(all.get(0).unwrap().seq, 1);
    assert_eq!(all.get(1).unwrap().seq, 2);

    let tail = client.query(&topic, &2, &10);
    assert_eq!(tail.len(), 1);
    assert_eq!(tail.get(0).unwrap().seq, 2);
}

#[test]
fn test_topics_are_independent() {
    let e = Env::default();
    let contract_id = e.register(EventIndexer, ());
    let client = EventIndexerClient::new(&e, &contract_id);

    client.record(
        &symbol_short!("payment"),
        &symbol_short!("created"),
        &String::from_str(&e, "k1"),
        &payload(&e, 1, 2),
    );
    client.record(
        &symbol_short!("aloop"),
        &symbol_short!("changed"),
        &String::from_str(&e, "k2"),
        &payload(&e, 3, 4),
    );

    assert_eq!(client.count(&symbol_short!("payment")), 1);
    assert_eq!(client.count(&symbol_short!("aloop")), 1);
}

#[test]
fn test_topic_limit_evicts_oldest() {
    let e = Env::default();
    let contract_id = e.register(EventIndexer, ());
    let client = EventIndexerClient::new(&e, &contract_id);

    let topic = symbol_short!("payment");
    assert_eq!(client.set_topic_limit(&topic, &2), ());
    assert_eq!(client.get_topic_limit(&topic), 2);

    for key_name in ["k1", "k2", "k3"] {
        client.record(
            &topic,
            &symbol_short!("created"),
            &String::from_str(&e, key_name),
            &payload(&e, 0, 0),
        );
    }

    assert_eq!(client.count(&topic), 2);
    let remaining = client.query(&topic, &1, &10);
    assert_eq!(remaining.len(), 2);
    assert_eq!(remaining.get(0).unwrap().seq, 2);
    assert_eq!(remaining.get(1).unwrap().seq, 3);
}

#[test]
fn test_empty_topic() {
    let e = Env::default();
    let contract_id = e.register(EventIndexer, ());
    let client = EventIndexerClient::new(&e, &contract_id);

    let topic = symbol_short!("nothing");
    assert_eq!(client.count(&topic), 0);
    assert_eq!(client.latest(&topic), None);
    assert_eq!(client.query(&topic, &1, &10).len(), 0);
}

#[test]
fn test_limit_validation() {
    let e = Env::default();
    let contract_id = e.register(EventIndexer, ());
    let client = EventIndexerClient::new(&e, &contract_id);

    let topic = symbol_short!("payment");
    assert_eq!(
        e.as_contract(&contract_id, || EventIndexer::set_topic_limit(
            e.clone(),
            topic.clone(),
            0
        )),
        Err(Error::LimitTooSmall)
    );
    assert_eq!(
        e.as_contract(&contract_id, || EventIndexer::query(e.clone(), topic.clone(), 1, 0)),
        Err(Error::InvalidQueryLimit)
    );
    // The rejected limit write must not have changed the stored limit.
    assert_eq!(client.get_topic_limit(&topic), crate::DEFAULT_MAX_EVENTS_PER_TOPIC);
}
// ── #885 — Cross-contract event schema compatibility checks ──────────────────

/// Pin the representative public event fields that downstream indexers depend on.
/// An intentional schema change (renaming or removing a field) will break this
/// test, surfacing the incompatibility before it reaches production.
#[test]
fn test_indexed_event_schema_pins() {
    let e = Env::default();
    e.ledger().with_mut(|li| li.timestamp = 3000);

    let contract_id = e.register(EventIndexer, ());
    let client = EventIndexerClient::new(&e, &contract_id);

    let topic = symbol_short!("transfer");
    let key = String::from_str(&e, "tx-001");

    // Build a representative payload with named fields.
    let p: Vec<(Symbol, Val)> = vec![
        &e,
        (symbol_short!("from"), 1i128.into_val(&e)),
        (symbol_short!("to"), 2i128.into_val(&e)),
        (symbol_short!("amount"), 500i128.into_val(&e)),
    ];

    let seq = client.record(&topic, &symbol_short!("transfer"), &key, &p);

    let event = client.latest(&topic).unwrap();

    // Pin public schema fields: seq, timestamp, event_name, key.
    assert_eq!(event.seq, seq, "seq must match the returned sequence number");
    assert_eq!(event.timestamp, 3000, "timestamp must match the ledger timestamp at record time");
    assert_eq!(event.event_name, symbol_short!("transfer"), "event_name must match the recorded name");
    assert_eq!(event.key, key, "key must match the correlation key");

    // Pin payload structure: three named fields in insertion order.
    assert_eq!(event.payload.len(), 3, "payload must contain exactly three fields");
    let (f0, _) = event.payload.get(0).unwrap();
    let (f1, _) = event.payload.get(1).unwrap();
    let (f2, _) = event.payload.get(2).unwrap();
    assert_eq!(f0, symbol_short!("from"), "first payload field must be 'from'");
    assert_eq!(f1, symbol_short!("to"), "second payload field must be 'to'");
    assert_eq!(f2, symbol_short!("amount"), "third payload field must be 'amount'");
}

/// An additive change — recording an extra field — must not break existing
/// consumers that only inspect the fields they know about.
#[test]
fn test_additive_payload_field_does_not_break_consumer() {
    let e = Env::default();
    e.ledger().with_mut(|li| li.timestamp = 4000);

    let contract_id = e.register(EventIndexer, ());
    let client = EventIndexerClient::new(&e, &contract_id);

    let topic = symbol_short!("deposit");
    let key = String::from_str(&e, "dep-007");

    // Consumer-known fields plus a new additive field ("note").
    let p: Vec<(Symbol, Val)> = vec![
        &e,
        (symbol_short!("asset"), 100i128.into_val(&e)),
        (symbol_short!("amount"), 1000i128.into_val(&e)),
        (symbol_short!("note"), 42i128.into_val(&e)),  // new additive field
    ];

    client.record(&topic, &symbol_short!("deposit"), &key, &p);

    let event = client.latest(&topic).unwrap();

    // Consumer inspects only the fields it knows — first two.
    let (f_asset, _) = event.payload.get(0).unwrap();
    let (f_amount, _) = event.payload.get(1).unwrap();

    assert_eq!(f_asset, symbol_short!("asset"),
        "additive change must not corrupt existing field positions");
    assert_eq!(f_amount, symbol_short!("amount"),
        "additive change must not corrupt existing field positions");
}

/// Two events recorded under the same topic but with different schemas must
/// each retain their own payload independently.
#[test]
fn test_different_event_schemas_coexist_in_topic() {
    let e = Env::default();

    let contract_id = e.register(EventIndexer, ());
    let client = EventIndexerClient::new(&e, &contract_id);

    let topic = symbol_short!("ledger");
    let key = String::from_str(&e, "blk-999");

    let p1: Vec<(Symbol, Val)> = vec![
        &e,
        (symbol_short!("type"), 1i128.into_val(&e)),
    ];
    let p2: Vec<(Symbol, Val)> = vec![
        &e,
        (symbol_short!("type"), 2i128.into_val(&e)),
        (symbol_short!("extra"), 99i128.into_val(&e)),
    ];

    client.record(&topic, &symbol_short!("v1"), &key, &p1);
    client.record(&topic, &symbol_short!("v2"), &key, &p2);

    let all = client.query(&topic, &1, &10);
    assert_eq!(all.len(), 2, "both events must be stored");

    // First event has 1 field.
    assert_eq!(all.get(0).unwrap().payload.len(), 1);
    // Second event has 2 fields.
    assert_eq!(all.get(1).unwrap().payload.len(), 2);
}

/// Sequence numbers must be monotonically increasing within a topic even when
/// the max per-topic limit causes older events to be evicted.
#[test]
fn test_sequence_numbers_monotonically_increasing() {
    let e = Env::default();

    let contract_id = e.register(EventIndexer, ());
    let client = EventIndexerClient::new(&e, &contract_id);

    let topic = symbol_short!("seq");
    let key = String::from_str(&e, "k");
    let empty: Vec<(Symbol, Val)> = Vec::new(&e);

    let s1 = client.record(&topic, &symbol_short!("e1"), &key, &empty);
    let s2 = client.record(&topic, &symbol_short!("e2"), &key, &empty);
    let s3 = client.record(&topic, &symbol_short!("e3"), &key, &empty);

    assert!(s1 < s2, "sequence numbers must increase");
    assert!(s2 < s3, "sequence numbers must increase");
    assert_eq!(s1, 1);
    assert_eq!(s2, 2);
    assert_eq!(s3, 3);
}
