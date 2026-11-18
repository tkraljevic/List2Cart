use crate::apply::{ApplyContext, apply_remote_versions, store_control_record};
use crate::db::Database;
use crate::ledger::Version;
use crate::models::{ControlPayload, PlannedItemSyncPayload, ShoppingListSyncPayload, Unit};
use chrono::Utc;
use ed25519_dalek::SigningKey;
use rusqlite::{Connection, params};
use std::collections::BTreeMap;
use syncnet::identity::verifying_key_to_base64;
use syncnet::trust::{ListPermission, MemberRole, NetworkKey, SignedVersionEnvelope, Validity};
use uuid::Uuid;

mod acl_tests;
mod family_tests;
mod proof_order_tests;

struct Fixture {
    root: SigningKey,
    network: NetworkKey,
    now: i64,
    created: String,
    base: u128,
}

impl Fixture {
    fn new() -> Self {
        let root = SigningKey::from_bytes(&[7; 32]);
        let time = Utc::now();
        Self {
            network: NetworkKey::from_signing_key("isolated test", root.clone()),
            root,
            now: time.timestamp(),
            created: time.to_rfc3339(),
            base: Uuid::now_v7().as_u128() & !0xffff,
        }
    }

    fn version(
        &self,
        seq: u128,
        entity: &str,
        id: &str,
        channel: &str,
        payload: String,
    ) -> Version {
        self.signed(seq, entity, id, channel, payload, &self.root)
    }

    fn signed(
        &self,
        seq: u128,
        entity: &str,
        id: &str,
        channel: &str,
        payload: String,
        key: &SigningKey,
    ) -> Version {
        let env = SignedVersionEnvelope {
            version_id: Uuid::from_u128(self.base + seq).to_string(),
            entity: entity.into(),
            entity_id: id.into(),
            channel: channel.into(),
            node_id: verifying_key_to_base64(&key.verifying_key()),
            supersedes: String::new(),
            archived: false,
            created_at: self.created.clone(),
            created_at_unix: self.now,
            schema_version: 1,
            author_pub_key: String::new(),
            grant_ref: "grant-a".into(),
            payload,
            signature: String::new(),
        }
        .sign(key);
        Version {
            version_id: env.version_id,
            entity: env.entity,
            entity_id: env.entity_id,
            node_id: env.node_id,
            supersedes: env.supersedes,
            archived: env.archived,
            payload: env.payload,
            created_at: env.created_at,
            schema_version: env.schema_version,
            channel: env.channel,
            author_pub_key: env.author_pub_key,
            grant_ref: env.grant_ref,
            signature: env.signature,
            created_at_unix: env.created_at_unix,
        }
    }

    fn list(&self, seq: u128, id: &str) -> Version {
        self.version(
            seq,
            "shopping_lists",
            id,
            &format!("list:{id}"),
            serde_json::to_string(&ShoppingListSyncPayload {
                sync_id: id.into(),
                name: "Subota".into(),
                store_sync_id: None,
                store_name: None,
                updated_at: self.created.clone(),
                archived: false,
            })
            .unwrap(),
        )
    }

    fn item(&self, seq: u128, checked: bool) -> Version {
        self.version(
            seq,
            "planned_items",
            "item-1",
            "list:list-a",
            serde_json::to_string(&PlannedItemSyncPayload {
                sync_id: "item-1".into(),
                list_sync_id: "list-a".into(),
                barcode: None,
                name: "Kruh".into(),
                qty_millis: 1000,
                unit: Unit::Kom,
                estimated_price_cents: 150,
                checked,
                updated_at: self.created.clone(),
                archived: false,
            })
            .unwrap(),
        )
    }

    fn apply(
        &self,
        conn: &Connection,
        versions: &[Version],
    ) -> Result<crate::apply::ApplyReport, String> {
        let key = self.network.public_b64();
        apply_remote_versions(
            conn,
            ApplyContext {
                network_pub: &self.network.public,
                peer_key: &key,
                recipient_b64: Some(&key),
                now_unix: self.now,
            },
            versions,
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
    }
}

fn state(conn: &Connection) -> (String, bool, bool) {
    conn.query_row(
        "SELECT name, checked, archived FROM planned_items WHERE sync_id = 'item-1'",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .unwrap()
}

#[test]
fn revoking_one_list_grant_keeps_other_lists_and_membership_active() {
    use crate::apply::allowed_channels_for_recipient;
    use syncnet::trust::Revocation;
    let db = Database::open_in_memory().unwrap();
    let conn = db.test_connection();
    let f = Fixture::new();
    let family = SigningKey::from_bytes(&[81; 32]);
    let family_pub = verifying_key_to_base64(&family.verifying_key());
    let member = f
        .network
        .admit_member(
            "family",
            &family.verifying_key(),
            MemberRole::FamilyMember,
            "family-principal",
            "root",
            Validity {
                issued_at: f.now - 10,
                valid_for_secs: 3600,
            },
        )
        .unwrap();
    store_control_record(
        conn,
        &f.network.public,
        &ControlPayload::Membership(member),
        f.now,
    )
    .unwrap();
    let grant = |id: &str, list: &str| {
        f.network
            .issue_list_grant(
                id,
                list,
                &family.verifying_key(),
                ListPermission::Write,
                "root",
                Validity {
                    issued_at: f.now - 10,
                    valid_for_secs: 3600,
                },
            )
            .unwrap()
    };
    let a = ControlPayload::ListGrant(grant("grant-revoked", "list-a"));
    let b = ControlPayload::ListGrant(grant("grant-a", "list-b"));
    for ctrl in [&a, &b] {
        store_control_record(conn, &f.network.public, ctrl, f.now).unwrap();
    }
    let revoked = Revocation {
        id: String::new(),
        kind: "list_grant".into(),
        target_id: "grant-revoked".into(),
        public_key: family_pub.clone(),
        issued_at: f.now,
        revoked_at: f.now,
        revoked_by: "root".into(),
        signer_key: String::new(),
        signature: String::new(),
    }
    .sign(&f.root);
    store_control_record(
        conn,
        &f.network.public,
        &ControlPayload::Revocation(revoked),
        f.now,
    )
    .unwrap();
    let allowed =
        allowed_channels_for_recipient(conn, &f.network.public, &family_pub, f.now).unwrap();
    assert!(
        allowed.contains("list:list-b"),
        "revoking list-a must not revoke list-b"
    );
    assert!(!allowed.contains("list:list-a"));
    assert!(allowed.contains("control"));
    assert!(!allowed.iter().any(|ch| ch.starts_with("private:")));
    let valid = f.signed(
        1,
        "shopping_lists",
        "list-b",
        "list:list-b",
        f.list(1, "list-b").payload,
        &family,
    );
    assert!(f.apply(conn, &[valid]).unwrap().rejected.is_empty());
    store_control_record(conn, &f.network.public, &a, f.now).unwrap();
    assert!(
        !allowed_channels_for_recipient(conn, &f.network.public, &family_pub, f.now)
            .unwrap()
            .contains("list:list-a"),
        "replaying a revoked grant must not restore it"
    );
    let renewed = ControlPayload::ListGrant(grant("new-grant-a", "list-a"));
    store_control_record(conn, &f.network.public, &renewed, f.now).unwrap();
    assert!(
        allowed_channels_for_recipient(conn, &f.network.public, &family_pub, f.now)
            .unwrap()
            .contains("list:list-a")
    );
}

#[test]
fn rejected_version_keeps_coverage_open_until_successful_replay() {
    use crate::ledger::{VersionRange, frontier_key, get_confirmed_frontier, list_holes};
    let db = Database::open_in_memory().unwrap();
    let f = Fixture::new();
    let peer = f.network.public_b64();
    let key = frontier_key(&peer, "list:list-a");
    let first = f.list(1, "list-a");
    let second = f.item(2, false);
    let third = f.item(3, true);
    let fourth = f.item(4, false);
    let mut bad = third.clone();
    bad.signature = String::new();
    let coverage = BTreeMap::from([(
        key.clone(),
        VersionRange {
            from_id: String::new(),
            to_id: fourth.version_id.clone(),
        },
    )]);
    let apply = |versions: &[Version], coverage: &BTreeMap<_, _>| {
        apply_remote_versions(
            db.test_connection(),
            ApplyContext {
                network_pub: &f.network.public,
                peer_key: &peer,
                recipient_b64: Some(&peer),
                now_unix: f.now,
            },
            versions,
            &BTreeMap::new(),
            coverage,
        )
        .unwrap()
    };
    let report = apply(
        &[first.clone(), second.clone(), bad, fourth.clone()],
        &coverage,
    );
    assert_eq!(report.rejected.len(), 1);
    assert_eq!(
        get_confirmed_frontier(db.test_connection(), &peer)
            .unwrap()
            .get(&key)
            .map(String::as_str),
        Some("")
    );
    assert!(!list_holes(db.test_connection()).unwrap().is_empty());
    let report = apply(&[first, second, third, fourth.clone()], &coverage);
    assert!(report.rejected.is_empty());
    assert!(list_holes(db.test_connection()).unwrap().is_empty());
    assert_eq!(
        get_confirmed_frontier(db.test_connection(), &peer).unwrap()[&key],
        fourth.version_id
    );
}

#[test]
fn empty_or_discontinuous_coverage_cannot_skip_unreceived_versions() {
    use crate::ledger::{VersionRange, frontier_key, get_confirmed_frontier, list_holes};
    let db = Database::open_in_memory().unwrap();
    let f = Fixture::new();
    let peer = f.network.public_b64();
    let key = frontier_key(&peer, "list:list-a");
    let first = f.list(1, "list-a");
    let missing = f.item(2, false);
    let last = f.item(3, true);
    let coverage = BTreeMap::from([(
        key.clone(),
        VersionRange {
            from_id: String::new(),
            to_id: last.version_id.clone(),
        },
    )]);
    let apply = |versions: &[Version], ranges: &BTreeMap<_, _>| {
        apply_remote_versions(
            db.test_connection(),
            ApplyContext {
                network_pub: &f.network.public,
                peer_key: &peer,
                recipient_b64: Some(&peer),
                now_unix: f.now,
            },
            versions,
            &BTreeMap::new(),
            ranges,
        )
        .unwrap()
    };
    apply(&[], &coverage);
    assert!(
        !get_confirmed_frontier(db.test_connection(), &peer)
            .unwrap()
            .contains_key(&key)
    );
    // Receiving a high independent record does not prove that the preceding range arrived.
    apply(std::slice::from_ref(&first), &BTreeMap::new());
    apply(
        std::slice::from_ref(&last),
        &BTreeMap::from([(
            key.clone(),
            VersionRange {
                from_id: missing.version_id.clone(),
                to_id: last.version_id.clone(),
            },
        )]),
    );
    assert_eq!(
        get_confirmed_frontier(db.test_connection(), &peer).unwrap()[&key],
        ""
    );
    assert!(!list_holes(db.test_connection()).unwrap().is_empty());
    apply(&[first, missing, last.clone()], &coverage);
    assert!(list_holes(db.test_connection()).unwrap().is_empty());
    assert_eq!(
        get_confirmed_frontier(db.test_connection(), &peer).unwrap()[&key],
        last.version_id
    );
}

#[test]
fn delta_budget_counts_escaped_json_metadata_and_exact_boundaries() {
    use crate::ledger::compute_delta_paged;
    use std::collections::BTreeSet;
    let db = Database::open_in_memory().unwrap();
    let f = Fixture::new();
    let versions: Vec<_> = ["list-a", "list-b"]
        .iter()
        .enumerate()
        .map(|(i, id)| {
            let mut payload: ShoppingListSyncPayload =
                serde_json::from_str(&f.list(i as u128 + 1, id).payload).unwrap();
            payload.name = "Čokolada \\\"\n\t".repeat(20);
            f.version(
                i as u128 + 1,
                "shopping_lists",
                id,
                &format!("list:{id}"),
                serde_json::to_string(&payload).unwrap(),
            )
        })
        .collect();
    assert!(
        f.apply(db.test_connection(), &versions)
            .unwrap()
            .rejected
            .is_empty()
    );
    let allowed = BTreeSet::from(["list:list-a".into(), "list:list-b".into()]);
    let page = |budget| {
        compute_delta_paged(
            db.test_connection(),
            &BTreeMap::new(),
            &[],
            &allowed,
            budget,
        )
    };
    let full = page(1024 * 1024).unwrap();
    let length = serde_json::to_vec(&full).unwrap().len();
    assert_eq!(full.versions.len(), 2);
    assert_eq!(
        serde_json::to_vec(&page(length).unwrap()).unwrap().len(),
        length
    );
    let partial = page(length - 1).unwrap();
    assert_eq!(partial.versions.len(), 1);
    assert!(partial.has_more);
    assert!(serde_json::to_vec(&partial).unwrap().len() < length);
    let one_length = serde_json::to_vec(&partial).unwrap().len();
    assert_eq!(
        serde_json::to_vec(&page(one_length).unwrap())
            .unwrap()
            .len(),
        one_length
    );
    assert!(page(one_length - 1).is_err());
    assert!(page(1).is_err());
}

#[test]
fn oversized_snapshot_cannot_bypass_the_page_budget() {
    use crate::ledger::{compute_delta_paged, frontier_key};
    use std::collections::BTreeSet;
    let db = Database::open_in_memory().unwrap();
    let f = Fixture::new();
    let first = f.list(1, "list-a");
    let next = f.list(3, "list-a");
    f.apply(db.test_connection(), &[first.clone(), next])
        .unwrap();
    let key = frontier_key(&f.network.public_b64(), "list:list-a");
    db.test_connection()
        .execute(
            "INSERT INTO settings(key,value) VALUES(?1,?2)",
            params![
                format!("compaction_floor:{key}"),
                Uuid::from_u128(f.base + 2).to_string()
            ],
        )
        .unwrap();
    let peer = BTreeMap::from([(key, first.version_id)]);
    let allowed = BTreeSet::from(["list:list-a".into()]);
    let full =
        compute_delta_paged(db.test_connection(), &peer, &[], &allowed, 1024 * 1024).unwrap();
    assert_eq!(full.snapshot_versions.len(), 1);
    let length = serde_json::to_vec(&full).unwrap().len();
    assert!(compute_delta_paged(db.test_connection(), &peer, &[], &allowed, length - 1).is_err());
    assert!(compute_delta_paged(db.test_connection(), &peer, &[], &allowed, length).is_ok());
    // A new device and an old-hole request below the floor need recovery too,
    // even if MAX is absent or already above the compaction floor.
    let fresh = compute_delta_paged(
        db.test_connection(),
        &BTreeMap::new(),
        &[],
        &allowed,
        1024 * 1024,
    )
    .unwrap();
    assert_eq!(fresh.snapshot_versions.len(), 1);
    assert!(fresh.covered_ranges.is_empty());
    let last_id = fresh.snapshot_versions[0].version_id.clone();
    let (key, old_id) = peer.first_key_value().unwrap();
    let high_frontier = BTreeMap::from([(key.clone(), last_id.clone())]);
    let hole = crate::ledger::HoleRecord {
        author_node_id: f.network.public_b64(),
        channel: "list:list-a".into(),
        from_version_id: old_id.clone(),
        to_version_id: last_id,
    };
    let recovery = compute_delta_paged(
        db.test_connection(),
        &high_frontier,
        &[hole],
        &allowed,
        1024 * 1024,
    )
    .unwrap();
    assert_eq!(recovery.snapshot_versions.len(), 1);
    assert_eq!(recovery.snapshot_reset_keys, vec![key.clone()]);
    assert!(recovery.covered_ranges.is_empty());
}

#[test]
fn old_hole_replays_across_pages_even_when_peer_has_the_highest_uuid() {
    use crate::ledger::{
        VersionRange, compute_delta_paged, frontier_key, get_frontier, list_holes,
    };
    use std::collections::BTreeSet;
    let source = Database::open_in_memory().unwrap();
    let destination = Database::open_in_memory().unwrap();
    let f = Fixture::new();
    let peer = f.network.public_b64();
    let key = frontier_key(&peer, "list:list-a");
    let mut versions = vec![f.list(1, "list-a")];
    versions.extend((2..=12).map(|seq| f.item(seq, seq % 2 == 0)));
    let coverage = BTreeMap::from([(
        key.clone(),
        VersionRange {
            from_id: String::new(),
            to_id: versions.last().unwrap().version_id.clone(),
        },
    )]);
    let admit =
        |db: &Database, versions: &[Version], holes: &BTreeMap<_, _>, covered: &BTreeMap<_, _>| {
            apply_remote_versions(
                db.test_connection(),
                ApplyContext {
                    network_pub: &f.network.public,
                    peer_key: &peer,
                    recipient_b64: Some(&peer),
                    now_unix: f.now,
                },
                versions,
                holes,
                covered,
            )
            .unwrap()
        };
    assert!(
        admit(&source, &versions, &BTreeMap::new(), &coverage)
            .rejected
            .is_empty()
    );
    f.apply(destination.test_connection(), &versions[11..])
        .unwrap();
    let allowed = BTreeSet::from(["list:list-a".to_owned()]);
    let mut pages = 0;
    for _ in 0..20 {
        let batch = compute_delta_paged(
            source.test_connection(),
            &get_frontier(destination.test_connection()).unwrap(),
            &list_holes(destination.test_connection()).unwrap(),
            &allowed,
            1400,
        )
        .unwrap();
        assert!(serde_json::to_vec(&batch).unwrap().len() <= 1400);
        if batch.versions.is_empty() {
            break;
        }
        pages += 1;
        assert!(
            admit(
                &destination,
                &batch.versions,
                &batch.sender_holes,
                &batch.covered_ranges
            )
            .rejected
            .is_empty()
        );
        if !batch.has_more {
            break;
        }
    }
    assert!(pages > 1);
    assert!(
        list_holes(destination.test_connection())
            .unwrap()
            .is_empty()
    );
    let received: i64 = destination
        .test_connection()
        .query_row(
            "SELECT COUNT(*) FROM record_versions WHERE channel='list:list-a'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(received, versions.len() as i64);
    assert_eq!(
        state(source.test_connection()),
        state(destination.test_connection())
    );
}

#[test]
fn sender_hole_is_not_closed_by_its_own_coverage_claim_and_ack_failure_rolls_back() {
    use crate::ledger::{VersionRange, frontier_key, get_confirmed_frontier, list_holes};
    let db = Database::open_in_memory().unwrap();
    let f = Fixture::new();
    let peer = f.network.public_b64();
    let key = frontier_key(&peer, "list:list-a");
    let version = f.list(1, "list-a");
    let coverage = BTreeMap::from([(
        key.clone(),
        VersionRange {
            from_id: String::new(),
            to_id: version.version_id.clone(),
        },
    )]);
    let apply = |holes: &BTreeMap<_, _>| {
        apply_remote_versions(
            db.test_connection(),
            ApplyContext {
                network_pub: &f.network.public,
                peer_key: &peer,
                recipient_b64: Some(&peer),
                now_unix: f.now,
            },
            std::slice::from_ref(&version),
            holes,
            &coverage,
        )
    };
    assert!(apply(&coverage).unwrap().rejected.is_empty());
    assert_eq!(
        get_confirmed_frontier(db.test_connection(), &peer).unwrap()[&key],
        ""
    );
    let before = list_holes(db.test_connection()).unwrap();
    db.test_connection().execute_batch("CREATE TEMP TRIGGER fail_ack BEFORE INSERT ON sync_peer_state BEGIN SELECT RAISE(ABORT,'injected ACK failure'); END;").unwrap();
    assert!(apply(&BTreeMap::new()).is_err());
    assert_eq!(list_holes(db.test_connection()).unwrap(), before);
    assert_eq!(
        get_confirmed_frontier(db.test_connection(), &peer).unwrap()[&key],
        ""
    );
    let next = f.item(2, true);
    let next_coverage = BTreeMap::from([(
        key.clone(),
        VersionRange {
            from_id: String::new(),
            to_id: next.version_id.clone(),
        },
    )]);
    assert!(
        apply_remote_versions(
            db.test_connection(),
            ApplyContext {
                network_pub: &f.network.public,
                peer_key: &peer,
                recipient_b64: Some(&peer),
                now_unix: f.now
            },
            &[version.clone(), next],
            &BTreeMap::new(),
            &next_coverage
        )
        .is_err()
    );
    assert!(db.list_shopping_lists().unwrap()[0].items.is_empty());
    assert_eq!(list_holes(db.test_connection()).unwrap(), before);
    db.test_connection()
        .execute_batch("DROP TRIGGER fail_ack")
        .unwrap();
    assert!(apply(&BTreeMap::new()).unwrap().rejected.is_empty());
    assert!(list_holes(db.test_connection()).unwrap().is_empty());
}

#[test]
fn late_rejection_reopens_an_observed_stream_without_blocking_another_channel() {
    use crate::ledger::{VersionRange, frontier_key, get_confirmed_frontier, list_holes};
    let db = Database::open_in_memory().unwrap();
    let f = Fixture::new();
    let peer = f.network.public_b64();
    let key = frontier_key(&peer, "list:list-a");
    let first = f.list(1, "list-a");
    let last = f.item(3, true);
    let ranges = BTreeMap::from([(
        key.clone(),
        VersionRange {
            from_id: String::new(),
            to_id: last.version_id.clone(),
        },
    )]);
    let admit = |versions: &[Version], ranges: &BTreeMap<_, _>| {
        apply_remote_versions(
            db.test_connection(),
            ApplyContext {
                network_pub: &f.network.public,
                peer_key: &peer,
                recipient_b64: Some(&peer),
                now_unix: f.now,
            },
            versions,
            &BTreeMap::new(),
            ranges,
        )
        .unwrap()
    };
    admit(&[first.clone(), last.clone()], &ranges);
    assert!(list_holes(db.test_connection()).unwrap().is_empty());
    let mut bad = f.item(2, false);
    bad.signature.clear();
    let other = f.list(4, "list-b");
    let other_key = frontier_key(&peer, "list:list-b");
    let other_range = BTreeMap::from([(
        other_key.clone(),
        VersionRange {
            from_id: String::new(),
            to_id: other.version_id.clone(),
        },
    )]);
    assert_eq!(admit(&[bad, other.clone()], &other_range).rejected.len(), 1);
    let confirmed = get_confirmed_frontier(db.test_connection(), &peer).unwrap();
    assert_eq!(confirmed[&key], first.version_id);
    assert_eq!(confirmed[&other_key], other.version_id);
    admit(&[f.item(2, false), last], &ranges);
    assert!(list_holes(db.test_connection()).unwrap().is_empty());
}

#[test]
fn receipt_manifest_detects_missing_zero_price_items_and_inconsistent_headers() {
    let db = Database::open_in_memory().unwrap();
    let f = Fixture::new();
    let store = f.version(1,"stores","receipt-store","private:test",serde_json::json!({
        "sync_id":"receipt-store","name":"Receipt store","location":"","color":"#000000","updated_at":f.created
    }).to_string());
    let header = |seq, total, ids: Vec<&str>| {
        f.version(seq,"receipts","receipt","private:test",serde_json::json!({
        "sync_id":"receipt","store_sync_id":"receipt-store","store_name":"Receipt store","created_at":f.created,
        "cart_total_cents":total,"receipt_total_cents":100,"difference_cents":total-100,"item_ids":ids
    }).to_string())
    };
    let item = |seq, id: &str, price| {
        f.version(seq,"receipt_items",id,"private:test",serde_json::json!({
        "sync_id":id,"receipt_sync_id":"receipt","barcode":"code","name":"Item","qty_millis":1000,
        "unit":"kom","unit_price_cents":price,"total_cents":price,"updated_at":f.created
    }).to_string())
    };
    f.apply(
        db.test_connection(),
        &[
            store,
            header(2, 100, vec!["paid", "free"]),
            item(3, "paid", 100),
        ],
    )
    .unwrap();
    assert!(db.list_receipts().unwrap().is_empty());
    assert_eq!(db.incomplete_receipt_count().unwrap(), 1);
    f.apply(db.test_connection(), &[item(4, "free", 0)])
        .unwrap();
    assert_eq!(db.list_receipts().unwrap().len(), 1);
    f.apply(
        db.test_connection(),
        &[header(5, 200, vec!["paid", "free"])],
    )
    .unwrap();
    assert!(db.list_receipts().unwrap().is_empty());
    f.apply(
        db.test_connection(),
        &[header(6, 100, vec!["paid", "free"])],
    )
    .unwrap();
    assert_eq!(db.list_receipts().unwrap().len(), 1);
    let report = f
        .apply(
            db.test_connection(),
            &[
                header(7, 100, vec!["paid", "paid"]),
                f.list(8, "valid-after-bad-receipt"),
            ],
        )
        .unwrap();
    assert_eq!(report.rejected.len(), 1);
    assert_eq!(report.applied_versions, 1);
    assert_eq!(db.list_receipts().unwrap().len(), 1);
}

#[test]
fn signed_catalog_collisions_preserve_original_identity_and_allow_other_records() {
    let db = Database::open_in_memory().unwrap();
    let f = Fixture::new();
    let product = |seq, id: &str, barcode: &str| {
        f.version(
            seq,
            "products",
            id,
            "private:test",
            serde_json::json!({"sync_id":id,"barcode":barcode,"name":"Kruh","category":"Opce",
        "default_unit":"kom","last_price_cents":100,"last_store_name":"",
        "image_data_url":null,"updated_at":f.created,"archived":false})
            .to_string(),
        )
    };
    let store = |seq, id: &str| {
        f.version(seq, "stores", id, "private:test",
        serde_json::json!({"sync_id":id,"name":"Collision store","location":"","color":"#000000",
        "updated_at":f.created,"archived":false}).to_string())
    };
    f.apply(
        db.test_connection(),
        &[
            product(1, "product-a", "collision-code"),
            store(2, "store-a"),
        ],
    )
    .unwrap();
    let report = f
        .apply(
            db.test_connection(),
            &[
                product(3, "product-b", "collision-code"),
                product(4, "product-a", "changed-code"),
                store(5, "store-b"),
                f.list(6, "unrelated-list"),
            ],
        )
        .unwrap();
    assert_eq!(report.rejected.len(), 1);
    assert_eq!(report.applied_versions, 3);
    let identity: String = db
        .test_connection()
        .query_row(
            "SELECT sync_id FROM products WHERE barcode='collision-code'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(identity, "product-a");
    assert!(db.get_product("changed-code").unwrap().is_none());
    let identity: String = db
        .test_connection()
        .query_row(
            "SELECT sync_id FROM stores WHERE name='Collision store'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(identity, "store-a");
    let conflicts = db.identity_conflicts().unwrap();
    assert_eq!(conflicts.len(), 2);
    assert_eq!(conflicts[0].sync_ids, vec!["store-a", "store-b"]);
    assert_eq!(conflicts[1].sync_ids, vec!["product-a", "product-b"]);
    assert!(db.get_product("collision-code").is_err());
    assert!(db.get_product_store_prices("collision-code").is_err());
    let before = db.ledger_stats().unwrap();
    assert!(
        db.upsert_product(&crate::db::UpsertProductInput {
            barcode: "collision-code",
            name: "Ambiguous edit",
            category: "Opce",
            default_unit: Unit::Kom,
            price_cents: 200,
            store_name: "",
            image_data_url: None,
        })
        .is_err()
    );
    assert_eq!(
        serde_json::to_value(db.ledger_stats().unwrap()).unwrap(),
        serde_json::to_value(before).unwrap()
    );
    // A replica receiving separate packages in the opposite order preserves both IDs.
    let reverse = Database::open_in_memory().unwrap();
    for version in [
        store(5, "store-b"),
        product(3, "product-b", "collision-code"),
        store(2, "store-a"),
        product(1, "product-a", "collision-code"),
    ] {
        assert!(
            f.apply(reverse.test_connection(), &[version])
                .unwrap()
                .rejected
                .is_empty()
        );
    }
    assert_eq!(
        serde_json::to_value(reverse.identity_conflicts().unwrap()).unwrap(),
        serde_json::to_value(conflicts).unwrap()
    );
}

#[test]
fn pending_child_cannot_be_rebound_to_another_parent_before_projection() {
    let db = Database::open_in_memory().unwrap();
    let f = Fixture::new();
    let original = f.item(1, false);
    assert_eq!(
        f.apply(db.test_connection(), &[original])
            .unwrap()
            .deferred_children,
        1
    );
    let mut payload: PlannedItemSyncPayload =
        serde_json::from_str(&f.item(2, false).payload).unwrap();
    payload.list_sync_id = "other-list".into();
    let changed = f.version(
        2,
        "planned_items",
        "item-1",
        "list:list-a",
        serde_json::to_string(&payload).unwrap(),
    );
    assert_eq!(
        f.apply(db.test_connection(), &[changed])
            .unwrap()
            .rejected
            .len(),
        1
    );
    f.apply(db.test_connection(), &[f.list(3, "list-a")])
        .unwrap();
    assert_eq!(state(db.test_connection()), ("Kruh".into(), false, false));
}

#[test]
fn pending_receipt_and_price_bindings_cannot_change_across_packages() {
    let db = Database::open_in_memory().unwrap();
    let f = Fixture::new();
    let cases = [
        (
            "receipts",
            "store_sync_id",
            serde_json::json!({
                "sync_id":"bound-receipt", "store_sync_id":"missing-store", "store_name":"Store",
                "created_at":f.created,"cart_total_cents":100,"receipt_total_cents":100,"difference_cents":0
            }),
        ),
        (
            "receipt_items",
            "receipt_sync_id",
            serde_json::json!({
                "sync_id":"bound-item", "receipt_sync_id":"missing-receipt", "barcode":"code", "name":"Item",
                "qty_millis":1000,"unit":"kom","unit_price_cents":100,"total_cents":100,"updated_at":f.created
            }),
        ),
        (
            "price_history",
            "product_sync_id",
            serde_json::json!({
                "sync_id":"bound-price", "product_sync_id":"missing-product", "store_sync_id":"store",
                "barcode":"code","store_name":"Store","price_cents":100,"recorded_at":f.created,"updated_at":f.created
            }),
        ),
    ];
    for (index, (entity, field, payload)) in cases.into_iter().enumerate() {
        let id = payload["sync_id"].as_str().unwrap().to_owned();
        let seq = (index as u128) * 2 + 1;
        let original = f.version(seq, entity, &id, "private:test", payload.to_string());
        let report = f.apply(db.test_connection(), &[original]).unwrap();
        assert!(
            report.rejected.is_empty(),
            "{entity}: {:?}",
            report.rejected
        );
        assert_eq!(report.deferred_children, 1);
        let mut changed = payload;
        changed[field] = "different-parent".into();
        let changed = f.version(seq + 1, entity, &id, "private:test", changed.to_string());
        let report = f.apply(db.test_connection(), &[changed]).unwrap();
        assert_eq!(report.rejected.len(), 1, "{entity}");
        assert!(
            report.rejected[0].reason.contains("roditelja"),
            "{entity}: {:?}",
            report.rejected
        );
        assert_eq!(report.applied_versions, 0);
    }
}

#[test]
fn malformed_product_identity_and_unsafe_cents_are_rejected_without_losing_valid_records() {
    let db = Database::open_in_memory().unwrap();
    let f = Fixture::new();
    let payload = |id: &str, cents: i64| {
        serde_json::json!({
            "sync_id":id, "barcode":"rejected-product", "name":"Kruh", "category":"Opce",
            "default_unit":"kom", "last_price_cents":cents, "last_store_name":"",
            "image_data_url":null, "updated_at":f.created, "archived":false
        })
        .to_string()
    };
    let invalid_id = f.version(
        1,
        "products",
        "expected-id",
        "private:test",
        payload("different-id", 100),
    );
    let invalid_price = f.version(
        2,
        "products",
        "expected-id",
        "private:test",
        payload("expected-id", crate::calc::MAX_SAFE_CENTS + 1),
    );
    let good = f.list(3, "list-after-invalid-records");
    let report = f
        .apply(db.test_connection(), &[invalid_id, invalid_price, good])
        .unwrap();
    assert_eq!(report.rejected.len(), 2);
    assert_eq!(report.applied_versions, 1);
    assert!(db.get_product("rejected-product").unwrap().is_none());
    assert_eq!(db.list_shopping_lists().unwrap().len(), 1);
}

#[test]
fn three_replicas_converge_with_out_of_order_children_conflicts_and_duplicates() {
    let f = Fixture::new();
    let databases: Vec<_> = (0..3)
        .map(|_| Database::open_in_memory().unwrap())
        .collect();
    let list = f.list(2, "list-a");
    let older = f.item(1, false);
    let newer = f.item(3, true);
    let orders = [
        vec![list.clone(), older.clone(), newer.clone()],
        vec![newer.clone(), older.clone(), list.clone()],
        vec![older, list, newer],
    ];
    for (db, order) in databases.iter().zip(&orders) {
        for v in order {
            assert!(
                f.apply(db.test_connection(), std::slice::from_ref(v))
                    .unwrap()
                    .rejected
                    .is_empty()
            );
        }
        let replay = f.apply(db.test_connection(), order).unwrap();
        assert_eq!(replay.applied_versions, 0);
        assert_eq!(state(db.test_connection()), ("Kruh".into(), true, false));
        let pending: i64 = db
            .test_connection()
            .query_row("SELECT COUNT(*) FROM pending_projections", [], |r| r.get(0))
            .unwrap();
        assert_eq!(pending, 0);
    }
}

#[test]
fn projection_failure_rolls_back_ledger_and_replay_recovers() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.test_connection();
    let f = Fixture::new();
    f.apply(conn, &[f.list(1, "list-a")]).unwrap();
    conn.execute_batch("CREATE TRIGGER fail_projection BEFORE INSERT ON planned_items BEGIN SELECT RAISE(ABORT, 'injected failure'); END;").unwrap();
    let item = f.item(2, false);
    assert!(f.apply(conn, std::slice::from_ref(&item)).is_err());
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM record_versions WHERE version_id = ?1",
            [&item.version_id],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    conn.execute_batch("DROP TRIGGER fail_projection").unwrap();
    assert_eq!(f.apply(conn, &[item]).unwrap().projected_records, 1);
}

#[test]
fn grant_cannot_write_another_list_or_private_entities() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.test_connection();
    let f = Fixture::new();
    let family = SigningKey::from_bytes(&[8; 32]);
    let member = f
        .network
        .admit_member(
            "family",
            &family.verifying_key(),
            MemberRole::FamilyMember,
            "another-person",
            "root",
            Validity {
                issued_at: f.now - 10,
                valid_for_secs: 3600,
            },
        )
        .unwrap();
    store_control_record(
        conn,
        &f.network.public,
        &ControlPayload::Membership(member),
        f.now,
    )
    .unwrap();
    let grant = f
        .network
        .issue_list_grant(
            "grant-a",
            "list-a",
            &family.verifying_key(),
            ListPermission::Write,
            "root",
            Validity {
                issued_at: f.now - 10,
                valid_for_secs: 3600,
            },
        )
        .unwrap();
    store_control_record(
        conn,
        &f.network.public,
        &ControlPayload::ListGrant(grant),
        f.now,
    )
    .unwrap();
    f.apply(conn, &[f.list(1, "list-a"), f.list(2, "list-b")])
        .unwrap();
    // Valid signature and valid write grant, but the item tries to change list-b through list-a.
    let mut p: PlannedItemSyncPayload = serde_json::from_str(&f.item(3, false).payload).unwrap();
    p.list_sync_id = "list-b".into();
    let forged = f.signed(
        4,
        "planned_items",
        "item-1",
        "list:list-a",
        serde_json::to_string(&p).unwrap(),
        &family,
    );
    let report = f.apply(conn, &[forged]).unwrap();
    assert_eq!(report.rejected.len(), 1);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM planned_items", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let private = f.signed(5, "stores", "store-x", "list:list-a",
        r##"{"sync_id":"store-x","name":"Hidden","location":"","color":"#000000","updated_at":"2026-11-17T00:00:00Z"}"##.into(), &family);
    assert_eq!(f.apply(conn, &[private]).unwrap().rejected.len(), 1);
    let wrong_grant = f.signed(
        6,
        "shopping_lists",
        "list-b",
        "list:list-b",
        f.list(6, "list-b").payload,
        &family,
    );
    assert_eq!(f.apply(conn, &[wrong_grant]).unwrap().rejected.len(), 1);
}

#[test]
fn same_entity_cannot_switch_channel_and_signed_routing_cannot_be_modified() {
    let db = Database::open_in_memory().unwrap();
    let f = Fixture::new();
    let original = f.list(1, "list-a");
    f.apply(db.test_connection(), std::slice::from_ref(&original))
        .unwrap();
    let switched = f.version(
        2,
        "shopping_lists",
        "list-a",
        "private:someone",
        original.payload.clone(),
    );
    assert_eq!(
        f.apply(db.test_connection(), &[switched])
            .unwrap()
            .rejected
            .len(),
        1
    );
    let mut tampered = f.list(3, "list-a");
    tampered.node_id = "someone-else".into();
    assert_eq!(
        f.apply(db.test_connection(), &[tampered])
            .unwrap()
            .rejected
            .len(),
        1
    );
    assert_eq!(
        db.test_connection()
            .query_row(
                "SELECT COUNT(*) FROM record_versions WHERE entity_id = ?1 AND signature != ''",
                params!["list-a"],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        1
    );
}
