use super::*;
use crate::apply::{ApplyContext, apply_remote_versions};
use crate::ledger::compute_delta;
use std::collections::BTreeMap;

struct TestDir(std::path::PathBuf);
impl TestDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("list2cart-local-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn versions(db: &Database) -> Vec<Version> {
    compute_delta(&db.conn, &BTreeMap::new(), 10000).unwrap()
}
fn count(db: &Database) -> i64 {
    db.conn
        .query_row("SELECT COUNT(*) FROM record_versions", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn original_author_upgrades_legacy_receipt_manifest_once_on_reopen() {
    let dir = TestDir::new();
    let path = dir.0.join("old-receipt.sqlite");
    let db = Database::open(&path).unwrap();
    let store = db.list_stores().unwrap().remove(0);
    let receipt = db.save_receipt(store.id, &store.name, &[], 0).unwrap();
    let (payload, _) = snapshot(&db.conn, "receipts", &receipt.sync_id).unwrap();
    let mut payload: serde_json::Value = serde_json::from_str(&payload).unwrap();
    payload.as_object_mut().unwrap().remove("item_ids");
    record_signed_version(
        &db.conn,
        &db.node_id().unwrap(),
        &db.identity.key,
        "receipts",
        &receipt.sync_id,
        false,
        &payload.to_string(),
        &format!("private:{}", db.identity.principal),
        "",
    )
    .unwrap();
    let before = count(&db);
    drop(db);
    let db = Database::open(&path).unwrap();
    assert_eq!(count(&db), before + 1);
    assert_eq!(db.list_receipts().unwrap().len(), 1);
    drop(db);
    assert_eq!(count(&Database::open(&path).unwrap()), before + 1);
}

#[test]
fn interrupted_receipt_transfer_resumes_after_disk_reopen_without_partial_display() {
    let source = Database::open_in_memory().unwrap();
    let replica = join_for_test(&source);
    let store = source.list_stores().unwrap().remove(0);
    source
        .save_receipt(
            store.id,
            &store.name,
            &[CartItem {
                id: "draft".into(),
                barcode: "resume-receipt".into(),
                name: "Item".into(),
                quantity: 1.0,
                unit: Unit::Kom,
                unit_price_cents: 100,
                total_cents: 100,
                image_data_url: None,
            }],
            100,
        )
        .unwrap();
    let partial: Vec<_> = versions(&source)
        .into_iter()
        .filter(|v| v.entity != "receipt_items")
        .collect();
    let recipient = replica.sync_identity().device_public_key;
    apply_remote_versions(
        &replica.conn,
        ApplyContext {
            network_pub: &source.identity.network_public,
            peer_key: &source.sync_identity().device_public_key,
            recipient_b64: Some(&recipient),
            now_unix: Utc::now().timestamp(),
        },
        &partial,
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
    .unwrap();
    assert!(replica.list_receipts().unwrap().is_empty());
    assert_eq!(replica.incomplete_receipt_count().unwrap(), 1);
    let dir = TestDir::new();
    let path = dir.0.join("partial-receipt.sqlite");
    replica
        .conn
        .execute("VACUUM INTO ?1", [path.to_str().unwrap()])
        .unwrap();
    syncnet::identity::save_key_exclusive(&key_path(&path, "device"), &replica.identity.key)
        .unwrap();
    drop(replica);
    let reopened = Database::open(&path).unwrap();
    assert!(reopened.list_receipts().unwrap().is_empty());
    assert_eq!(reopened.incomplete_receipt_count().unwrap(), 1);
    assert!(transfer(&source, &reopened).rejected.is_empty());
    assert_eq!(reopened.list_receipts().unwrap().len(), 1);
    assert_eq!(reopened.incomplete_receipt_count().unwrap(), 0);
    assert_eq!(transfer(&source, &reopened).applied_versions, 0);
}

#[test]
fn receipt_packages_converge_on_three_replicas_without_exposing_partial_receipts() {
    let source = Database::open_in_memory().unwrap();
    let replicas: Vec<_> = (0..3).map(|_| join_for_test(&source)).collect();
    let store = source
        .add_store("Receipt packages", "Test", "#000000")
        .unwrap();
    let items: Vec<_> = [100, 0]
        .into_iter()
        .enumerate()
        .map(|(i, price)| CartItem {
            id: i.to_string(),
            barcode: format!("receipt-code-{i}"),
            name: format!("Item {i}"),
            quantity: 1.0,
            unit: Unit::Kom,
            unit_price_cents: price,
            total_cents: price,
            image_data_url: None,
        })
        .collect();
    source
        .save_receipt(store.id, &store.name, &items, 100)
        .unwrap();
    let all = versions(&source);
    for (index, replica) in replicas.iter().enumerate() {
        let receive = |records: &[Version]| {
            apply_remote_versions(
                &replica.conn,
                ApplyContext {
                    network_pub: &source.identity.network_public,
                    peer_key: &source.sync_identity().device_public_key,
                    recipient_b64: Some(&replica.sync_identity().device_public_key),
                    now_unix: Utc::now().timestamp(),
                },
                records,
                &BTreeMap::new(),
                &BTreeMap::new(),
            )
            .unwrap()
        };
        let controls: Vec<_> = all
            .iter()
            .filter(|v| v.channel == "control")
            .cloned()
            .collect();
        assert!(receive(&controls).rejected.is_empty());
        let mut domain: Vec<_> = all
            .iter()
            .filter(|v| v.channel != "control")
            .cloned()
            .collect();
        match index {
            0 => domain.sort_by_key(|v| v.entity != "receipts"),
            1 => domain.reverse(),
            _ => domain.sort_by_key(|v| v.entity != "receipt_items"),
        }
        for record in domain {
            assert!(receive(&[record]).rejected.is_empty());
            for receipt in replica.list_receipts().unwrap() {
                assert_eq!(receipt.items.len(), 2);
                assert_eq!(
                    receipt.items.iter().map(|i| i.total_cents).sum::<i64>(),
                    receipt.cart_total_cents
                );
            }
        }
        assert_eq!(replica.list_receipts().unwrap().len(), 1);
        assert_eq!(replica.incomplete_receipt_count().unwrap(), 0);
        assert_eq!(receive(&all).applied_versions, 0);
    }
}

#[test]
fn daily_maintenance_preserves_signed_tombstones_and_stale_replica_cannot_revive_them() {
    let dir = TestDir::new();
    let path = dir.0.join("maintenance.sqlite");
    let source = Database::open(&path).unwrap();
    let replica = join_for_test(&source);
    let list = source.create_shopping_list("Maintenance", None).unwrap();
    let item = source
        .add_planned_item(list.id, None, "Item", 1.0, Unit::Kom, 100)
        .unwrap();
    transfer(&source, &replica);
    for index in 0..10 {
        source.toggle_planned_item(item.id, index % 2 == 0).unwrap();
    }
    let stale = versions(&source);
    source
        .set_record_archived(LocalEntity::PlannedItem, &item.sync_id, true)
        .unwrap();
    let control_before: i64 = source
        .conn
        .query_row(
            "SELECT COUNT(*) FROM record_versions WHERE channel='control'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let removed = source.run_scheduled_maintenance().unwrap().unwrap();
    assert!(removed > 0);
    assert_eq!(source.run_scheduled_maintenance().unwrap(), None);
    assert_eq!(
        source
            .conn
            .query_row(
                "SELECT COUNT(*) FROM record_versions WHERE channel='control'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        control_before
    );
    let report = apply_remote_versions(
        &source.conn,
        ApplyContext {
            network_pub: &source.identity.network_public,
            peer_key: &replica.sync_identity().device_public_key,
            recipient_b64: Some(&source.sync_identity().device_public_key),
            now_unix: Utc::now().timestamp(),
        },
        &stale,
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
    .unwrap();
    assert!(report.rejected.is_empty(), "{:?}", report.rejected);
    assert!(source.list_shopping_lists().unwrap()[0].items.is_empty());
    let bundle = source
        .export_protected_bundle(
            &replica.sync_identity().device_public_key,
            &crate::ledger::get_frontier(&replica.conn).unwrap(),
            &[],
            512 * 1024,
        )
        .unwrap();
    let report = replica.import_protected_bundle(&bundle).unwrap();
    assert!(report.rejected.is_empty(), "{:?}", report.rejected);
    assert!(replica.list_shopping_lists().unwrap()[0].items.is_empty());
    assert_eq!(
        source
            .conn
            .query_row("PRAGMA freelist_count", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        0
    );
    drop(source);
    let reopened = Database::open(&path).unwrap();
    assert_eq!(reopened.run_scheduled_maintenance().unwrap(), None);
    assert!(reopened.list_shopping_lists().unwrap()[0].items.is_empty());
}

#[test]
fn failed_compaction_rolls_back_versions_floors_and_success_time() {
    let db = Database::open_in_memory().unwrap();
    let list = db.create_shopping_list("Atomic maintenance", None).unwrap();
    let item = db
        .add_planned_item(list.id, None, "Item", 1.0, Unit::Kom, 100)
        .unwrap();
    for i in 0..10 {
        db.toggle_planned_item(item.id, i % 2 == 0).unwrap();
    }
    let before = count(&db);
    db.conn.execute_batch("CREATE TRIGGER fail_floor BEFORE INSERT ON settings WHEN NEW.key LIKE 'compaction_floor:%' BEGIN SELECT RAISE(ABORT,'injected failure'); END;").unwrap();
    assert!(db.run_scheduled_maintenance().is_err());
    assert_eq!(count(&db), before);
    assert!(
        db.get_setting("maintenance_last_success")
            .unwrap()
            .is_none()
    );
    assert_eq!(
        db.conn
            .query_row(
                "SELECT COUNT(*) FROM settings WHERE key LIKE 'compaction_floor:%'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    db.conn.execute_batch("DROP TRIGGER fail_floor").unwrap();
    assert!(db.run_scheduled_maintenance().unwrap().unwrap() > 0);
}

#[test]
fn reviewed_conflict_resolution_is_signed_atomic_and_replicates_without_rewriting_history() {
    let source = Database::open_in_memory().unwrap();
    let replica = join_for_test(&source);
    let keep = source.add_store("Duplicate", "Keep", "#000000").unwrap();
    let other = source.add_store("Duplicate", "Other", "#000000").unwrap();
    let list = source
        .create_shopping_list("Historical reference", Some(other.id))
        .unwrap();
    transfer(&source, &replica);
    let ids = source
        .identity_conflicts()
        .unwrap()
        .into_iter()
        .find(|g| g.value == "duplicate")
        .unwrap()
        .sync_ids;
    let before = count(&source);
    assert!(
        source
            .resolve_identity_conflict("stores", &keep.sync_id, std::slice::from_ref(&keep.sync_id))
            .is_err()
    );
    source.conn.execute_batch("CREATE TRIGGER fail_resolution BEFORE INSERT ON record_versions WHEN NEW.archived=1 BEGIN SELECT RAISE(ABORT,'injected failure'); END;").unwrap();
    assert!(
        source
            .resolve_identity_conflict("stores", &keep.sync_id, &ids)
            .is_err()
    );
    assert_eq!(count(&source), before);
    assert_eq!(source.identity_conflicts().unwrap().len(), 1);
    source
        .conn
        .execute_batch("DROP TRIGGER fail_resolution")
        .unwrap();
    source
        .resolve_identity_conflict("stores", &keep.sync_id, &ids)
        .unwrap();
    assert_eq!(count(&source), before + 1);
    assert!(source.identity_conflicts().unwrap().is_empty());
    assert!(
        source
            .resolve_identity_conflict("stores", &keep.sync_id, &ids)
            .is_err()
    );
    let report = transfer(&source, &replica);
    assert!(report.rejected.is_empty(), "{:?}", report.rejected);
    assert!(replica.identity_conflicts().unwrap().is_empty());
    for db in [&source, &replica] {
        let reference: String = db
            .conn
            .query_row(
                "SELECT store_sync_id FROM shopping_lists WHERE sync_id=?1",
                [&list.sync_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(reference, other.sync_id);
        let archived: bool = db
            .conn
            .query_row(
                "SELECT archived FROM stores WHERE sync_id=?1",
                [&other.sync_id],
                |r| r.get(0),
            )
            .unwrap();
        assert!(archived);
    }
}

#[test]
fn product_resolution_allows_future_writes_only_to_the_chosen_identity() {
    let db = Database::open_in_memory().unwrap();
    let input = UpsertProductInput {
        barcode: "duplicate-code",
        name: "Chosen",
        category: "Opce",
        default_unit: Unit::Kom,
        price_cents: 100,
        store_name: "",
        image_data_url: None,
    };
    let keep = db.upsert_product(&input).unwrap();
    db.conn.execute("INSERT INTO products(barcode,sync_id,name,updated_at) VALUES ('duplicate-code','other-product','Other','now')",[]).unwrap();
    db.record_snapshot(&db.conn, "products", "other-product")
        .unwrap();
    let group = db.identity_conflicts().unwrap().pop().unwrap();
    db.resolve_identity_conflict("products", &keep.sync_id, &group.sync_ids)
        .unwrap();
    assert_eq!(
        db.get_product("duplicate-code").unwrap().unwrap().sync_id,
        keep.sync_id
    );
    assert!(
        db.get_product_store_prices("duplicate-code")
            .unwrap()
            .is_empty()
    );
    assert_eq!(db.upsert_product(&input).unwrap().sync_id, keep.sync_id);
    let other: (String, bool) = db
        .conn
        .query_row(
            "SELECT name,archived FROM products WHERE sync_id='other-product'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(other, ("Other".into(), true));
}

fn join_for_test(source: &Database) -> Database {
    let key = generate_signing_key();
    let root = source.identity.network_key.as_ref().unwrap();
    let net = NetworkKey::from_signing_key("test", root.clone());
    let validity = Validity {
        issued_at: Utc::now().timestamp(),
        valid_for_secs: 3600,
    };
    let dev_b64 = verifying_key_to_base64(&key.verifying_key());
    let membership = net
        .admit_member(&dev_b64, &key.verifying_key(), "root", validity)
        .unwrap();
    let role_rec = crate::acl_types::issue_member_role(
        &net,
        &dev_b64,
        &key.verifying_key(),
        crate::acl_types::MemberRole::OwnDevice,
        &source.identity.principal,
        "root",
        validity,
    )
    .unwrap();
    let db = Database {
        conn: Connection::open_in_memory().unwrap(),
        identity: LocalIdentity {
            key,
            network_public: source.identity.network_public,
            network_key: None,
            principal: source.identity.principal.clone(),
            db_path: None,
        },
    };
    db.init_schema().unwrap();
    let control = ControlPayload::Membership(membership);
    let role_control = ControlPayload::MemberRole(role_rec);
    for ctrl in [&control, &role_control] {
        store_control_record(
            &db.conn,
            &source.identity.network_public,
            ctrl,
            Utc::now().timestamp(),
        )
        .unwrap();
        store_control_record(
            &source.conn,
            &source.identity.network_public,
            ctrl,
            Utc::now().timestamp(),
        )
        .unwrap();
    }
    record_signed_version(
        &source.conn,
        &source.sync_identity().network_public_key,
        root,
        "membership",
        &format!("membership:{}", db.sync_identity().device_public_key),
        false,
        &serde_json::to_string(&control).unwrap(),
        "control",
        "",
    )
    .unwrap();
    record_signed_version(
        &source.conn,
        &source.sync_identity().network_public_key,
        root,
        "member_role",
        &format!("member_role:{}", db.sync_identity().device_public_key),
        false,
        &serde_json::to_string(&role_control).unwrap(),
        "control",
        "",
    )
    .unwrap();
    db.initialize_local_identity().unwrap();
    db
}

fn copy_membership_and_role(from: &Database, to: &Database, subject_key: &str) {
    for kind in ["membership", "member_role"] {
        if let Some(raw) = from
            .conn
            .query_row(
                "SELECT raw_json FROM sync_trust_records WHERE kind=?1 AND subject_key=?2",
                params![kind, subject_key],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .unwrap()
        {
            store_control_record(
                &to.conn,
                &from.identity.network_public,
                &serde_json::from_str::<ControlPayload>(&raw).unwrap(),
                Utc::now().timestamp(),
            )
            .unwrap();
        }
    }
}

fn transfer(source: &Database, destination: &Database) -> crate::apply::ApplyReport {
    let recipient = destination.sync_identity().device_public_key;
    apply_remote_versions(
        &destination.conn,
        ApplyContext {
            network_pub: &source.identity.network_public,
            peer_key: &source.sync_identity().device_public_key,
            recipient_b64: Some(&recipient),
            now_unix: Utc::now().timestamp(),
        },
        &versions(source),
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
    .unwrap()
}

#[test]
fn relay_with_a_lower_local_max_still_sends_an_arrival_below_the_recipients_max() {
    use crate::ledger::{VersionRange, frontier_key};
    let original = Database::open_in_memory().unwrap();
    let relay = join_for_test(&original);
    let destination = join_for_test(&original);
    let author = original.sync_identity().device_public_key;
    let relay_key = relay.sync_identity().device_public_key;
    let destination_key = destination.sync_identity().device_public_key;
    for db in [&relay, &destination] {
        for key in [&author, &relay_key, &destination_key] {
            copy_membership_and_role(&original, db, key);
        }
    }
    let list = original
        .create_shopping_list("Different maxima", None)
        .unwrap();
    let low = original
        .add_planned_item(list.id, None, "Low", 1.0, Unit::Kom, 100)
        .unwrap();
    let middle = original
        .add_planned_item(list.id, None, "Middle", 1.0, Unit::Kom, 100)
        .unwrap();
    let high = original
        .add_planned_item(list.id, None, "High", 1.0, Unit::Kom, 100)
        .unwrap();
    let all = versions(&original);
    let middle_version = all
        .iter()
        .find(|v| v.entity_id == middle.sync_id)
        .unwrap()
        .clone();
    let low_version = all
        .iter()
        .find(|v| v.entity_id == low.sync_id)
        .unwrap()
        .clone();
    let admit = |db: &Database, rows: &[Version], covered: &BTreeMap<_, _>| {
        let recipient = db.sync_identity().device_public_key;
        apply_remote_versions(
            &db.conn,
            ApplyContext {
                network_pub: &original.identity.network_public,
                peer_key: &author,
                recipient_b64: Some(&recipient),
                now_unix: Utc::now().timestamp(),
            },
            rows,
            &BTreeMap::new(),
            covered,
        )
        .unwrap()
    };
    let complete = |rows: &[Version]| {
        let mut ranges: BTreeMap<String, VersionRange> = BTreeMap::new();
        for v in rows {
            ranges
                .entry(frontier_key(&v.node_id, &v.channel))
                .and_modify(|r| r.to_id = r.to_id.clone().max(v.version_id.clone()))
                .or_insert(VersionRange {
                    from_id: String::new(),
                    to_id: v.version_id.clone(),
                });
        }
        ranges
    };
    let relay_rows: Vec<_> = all
        .iter()
        .filter(|v| v.entity_id != middle.sync_id && v.entity_id != high.sync_id)
        .cloned()
        .collect();
    let destination_rows: Vec<_> = all
        .iter()
        .filter(|v| v.entity_id != middle.sync_id)
        .cloned()
        .collect();
    assert!(
        admit(&relay, &relay_rows, &complete(&relay_rows))
            .rejected
            .is_empty()
    );
    assert!(
        admit(
            &destination,
            &destination_rows,
            &complete(&destination_rows)
        )
        .rejected
        .is_empty()
    );
    let page = relay
        .sync_reply(&destination.sync_request(&relay_key).unwrap(), 1024 * 1024)
        .unwrap();
    destination.import_sync_page(&page).unwrap();
    assert!(
        !destination.list_shopping_lists().unwrap()[0]
            .items
            .iter()
            .any(|i| i.name == "Middle")
    );
    let key = frontier_key(&author, &middle_version.channel);
    let range = BTreeMap::from([(
        key,
        VersionRange {
            from_id: low_version.version_id,
            to_id: middle_version.version_id.clone(),
        },
    )]);
    assert_eq!(admit(&relay, &[middle_version], &range).applied_versions, 1);
    let page = relay
        .sync_reply(&destination.sync_request(&relay_key).unwrap(), 1024 * 1024)
        .unwrap();
    destination.import_sync_page(&page).unwrap();
    let lists = destination.list_shopping_lists().unwrap();
    assert!(lists[0].items.iter().any(|i| i.name == "Middle"));
    assert!(lists[0].items.iter().any(|i| i.name == "High"));
}

#[test]
fn late_older_record_reaches_a_third_device_even_after_a_high_frontier_was_confirmed() {
    use crate::exchange::SyncReceiveOutcome;
    use crate::ledger::{VersionRange, frontier_key, get_frontier};
    let original = Database::open_in_memory().unwrap();
    let relay = join_for_test(&original);
    let destination = join_for_test(&original);
    let original_key = original.sync_identity().device_public_key;
    let relay_key = relay.sync_identity().device_public_key;
    let destination_key = destination.sync_identity().device_public_key;
    // Pairing has already established the three synthetic memberships.
    for db in [&relay, &destination] {
        for key in [&original_key, &relay_key, &destination_key] {
            copy_membership_and_role(&original, db, key);
        }
    }
    let list = original
        .create_shopping_list("Late independent record", None)
        .unwrap();
    let older = original
        .add_planned_item(
            list.id,
            None,
            "Later discovered older item",
            1.0,
            Unit::Kom,
            100,
        )
        .unwrap();
    let second_older = original
        .add_planned_item(
            list.id,
            None,
            "Older item during frozen snapshot",
            1.0,
            Unit::Kom,
            100,
        )
        .unwrap();
    for i in 0..12 {
        original
            .add_planned_item(
                list.id,
                None,
                &format!("Known newer item {i}"),
                1.0,
                Unit::Kom,
                100,
            )
            .unwrap();
    }
    let all = versions(&original);
    let late = all
        .iter()
        .find(|v| v.entity_id == older.sync_id)
        .unwrap()
        .clone();
    let late_second = all
        .iter()
        .find(|v| v.entity_id == second_older.sync_id)
        .unwrap()
        .clone();
    // The donor's previous complete view did not yet include the older item
    // later recovered from another source/archive. No supersedes link reveals it.
    let known: Vec<_> = all
        .into_iter()
        .filter(|v| v.version_id != late.version_id && v.version_id != late_second.version_id)
        .collect();
    let coverage = get_frontier(&original.conn)
        .unwrap()
        .into_iter()
        .map(|(k, to_id)| {
            (
                k,
                VersionRange {
                    from_id: String::new(),
                    to_id,
                },
            )
        })
        .collect();
    let admit = |rows: &[Version], covered: &BTreeMap<_, _>| {
        apply_remote_versions(
            &relay.conn,
            ApplyContext {
                network_pub: &original.identity.network_public,
                peer_key: &original_key,
                recipient_b64: Some(&relay_key),
                now_unix: Utc::now().timestamp(),
            },
            rows,
            &BTreeMap::new(),
            covered,
        )
    };
    assert!(admit(&known, &coverage).unwrap().rejected.is_empty());
    let first = relay
        .sync_reply(&destination.sync_request(&relay_key).unwrap(), 1024 * 1024)
        .unwrap();
    destination.import_sync_page(&first).unwrap();
    assert_eq!(
        destination.list_shopping_lists().unwrap()[0].items.len(),
        12
    );
    let acknowledged_before = destination
        .sync_request(&relay_key)
        .unwrap()
        .backfill_revisions;
    let before = get_frontier(&relay.conn).unwrap();
    let before_count = count(&relay);
    relay.conn.execute_batch("CREATE TEMP TRIGGER fail_backfill_marker BEFORE INSERT ON sync_backfill_revisions BEGIN SELECT RAISE(ABORT,'injected backfill marker failure'); END;").unwrap();
    assert!(admit(std::slice::from_ref(&late), &BTreeMap::new()).is_err());
    assert_eq!(count(&relay), before_count);
    relay
        .conn
        .execute_batch("DROP TRIGGER fail_backfill_marker")
        .unwrap();
    assert_eq!(
        admit(std::slice::from_ref(&late), &BTreeMap::new())
            .unwrap()
            .applied_versions,
        1
    );
    assert_eq!(
        get_frontier(&relay.conn).unwrap(),
        before,
        "MAX is unchanged by the late insertion"
    );
    let frozen = relay
        .sync_reply(&destination.sync_request(&relay_key).unwrap(), 4096)
        .unwrap();
    let channel = format!("private:{}", original.identity.principal);
    let revision_before: String = relay
        .conn
        .query_row(
            "SELECT revision FROM sync_backfill_revisions WHERE channel=?1",
            [&channel],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        admit(std::slice::from_ref(&late_second), &BTreeMap::new())
            .unwrap()
            .applied_versions,
        1
    );
    let revision_after: String = relay
        .conn
        .query_row(
            "SELECT revision FROM sync_backfill_revisions WHERE channel=?1",
            [&channel],
            |r| r.get(0),
        )
        .unwrap();
    assert_ne!(revision_before, revision_after);
    assert_eq!(
        admit(std::slice::from_ref(&late), &BTreeMap::new())
            .unwrap()
            .applied_versions,
        0
    );
    let after_duplicate: String = relay
        .conn
        .query_row(
            "SELECT revision FROM sync_backfill_revisions WHERE channel=?1",
            [&channel],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(after_duplicate, revision_after);
    // Both the marker and the already frozen outgoing snapshot survive restart.
    let dir = TestDir::new();
    let path = dir.0.join("backfilled-relay.sqlite");
    relay
        .conn
        .execute("VACUUM INTO ?1", [path.to_str().unwrap()])
        .unwrap();
    syncnet::identity::save_key_exclusive(&key_path(&path, "device"), &relay.identity.key).unwrap();
    drop(relay);
    let relay = Database::open(&path).unwrap();
    let SyncReceiveOutcome::Snapshot { progress: first } =
        destination.import_sync_page(&frozen).unwrap()
    else {
        panic!("a changed backfill revision requires recovery")
    };
    assert!(!first.complete);
    let mut complete = false;
    let mut failed_ack_checked = false;
    for _ in 0..100 {
        let request = destination.sync_request(&relay_key).unwrap();
        let page = relay.sync_reply(&request, 4096).unwrap();
        assert!(
            page.open(crate::apply::PROTOCOL_NAME, &destination.identity.key)
                .unwrap()
                .0
                .len()
                <= 4096
        );
        let (bytes, _) = page
            .open(crate::apply::PROTOCOL_NAME, &destination.identity.key)
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        if json["snapshot_page"]["done"] == true && !failed_ack_checked {
            destination.conn.execute_batch("CREATE TEMP TRIGGER fail_backfill_ack BEFORE INSERT ON sync_backfill_ack BEGIN SELECT RAISE(ABORT,'injected backfill ACK failure'); END;").unwrap();
            assert!(destination.import_sync_page(&page).is_err());
            assert!(
                destination
                    .sync_request(&relay_key)
                    .unwrap()
                    .backfill_revisions
                    == acknowledged_before
            );
            assert_eq!(
                destination.list_shopping_lists().unwrap()[0].items.len(),
                12
            );
            destination
                .conn
                .execute_batch("DROP TRIGGER fail_backfill_ack")
                .unwrap();
            failed_ack_checked = true;
        }
        let SyncReceiveOutcome::Snapshot { progress } =
            destination.import_sync_page(&page).unwrap()
        else {
            panic!("expected frozen continuation")
        };
        assert_eq!(progress.snapshot_id, first.snapshot_id);
        if progress.complete {
            complete = true;
            break;
        }
    }
    assert!(complete && failed_ack_checked);
    let items = &destination.list_shopping_lists().unwrap()[0].items;
    assert!(
        items
            .iter()
            .any(|i| i.name == "Later discovered older item")
    );
    assert!(
        !items
            .iter()
            .any(|i| i.name == "Older item during frozen snapshot")
    );
    assert_eq!(
        destination
            .sync_request(&relay_key)
            .unwrap()
            .backfill_revisions[&channel],
        revision_before,
        "completion must acknowledge the frozen revision, not the newer one"
    );
    let replacement = relay
        .sync_reply(&destination.sync_request(&relay_key).unwrap(), 4096)
        .unwrap();
    let SyncReceiveOutcome::Snapshot { progress: second } =
        destination.import_sync_page(&replacement).unwrap()
    else {
        panic!("the second late record requires another revision")
    };
    assert_ne!(second.snapshot_id, first.snapshot_id);
    let mut complete = second.complete;
    for _ in 0..100 {
        if complete {
            break;
        }
        let page = relay
            .sync_reply(&destination.sync_request(&relay_key).unwrap(), 4096)
            .unwrap();
        let SyncReceiveOutcome::Snapshot { progress } =
            destination.import_sync_page(&page).unwrap()
        else {
            panic!("expected second continuation")
        };
        complete = progress.complete;
    }
    assert!(complete);
    assert!(
        destination.list_shopping_lists().unwrap()[0]
            .items
            .iter()
            .any(|i| i.name == "Older item during frozen snapshot")
    );
    assert_eq!(
        destination
            .sync_request(&relay_key)
            .unwrap()
            .backfill_revisions[&channel],
        revision_after
    );
    let settled = relay
        .sync_reply(&destination.sync_request(&relay_key).unwrap(), 4096)
        .unwrap();
    assert!(
        matches!(
            destination.import_sync_page(&settled).unwrap(),
            SyncReceiveOutcome::Delta { .. }
        ),
        "duplicate arrivals must not cause endless snapshots"
    );
    let stream = frontier_key(
        &original_key,
        &format!("private:{}", original.identity.principal),
    );
    assert_eq!(
        get_frontier(&destination.conn).unwrap()[&stream],
        before[&stream]
    );
}

#[test]
fn automatic_exchange_rejects_forged_requests_and_unrelated_failures_do_not_start_recovery() {
    let source = Database::open_in_memory().unwrap();
    let destination = join_for_test(&source);
    let sender = source.sync_identity().device_public_key;
    let recipient = destination.sync_identity().device_public_key;
    copy_membership_and_role(&source, &destination, &sender);
    let request = destination.sync_request(&sender).unwrap();
    let mut forged = request.clone();
    forged
        .frontier
        .insert("invented|private:test".into(), Uuid::now_v7().to_string());
    assert!(source.sync_reply(&forged, 4096).is_err());
    let mut forged = request.clone();
    forged.signature.clear();
    assert!(source.sync_reply(&forged, 4096).is_err());
    let mut forged = request.clone();
    forged
        .backfill_revisions
        .insert("control".into(), Uuid::new_v4().to_string());
    assert!(source.sync_reply(&forged, 4096).is_err());
    let mut legacy = request.clone();
    legacy.format = 1;
    assert!(source.sync_reply(&legacy, 4096).is_err());
    let mut forged = request.clone();
    forged.recovery = Some(crate::recovery::SnapshotProgress {
        snapshot_id: Uuid::new_v4().to_string(),
        next_offset: 1,
        total_versions: 1,
        complete: true,
    });
    assert!(source.sync_reply(&forged, 4096).is_err());
    let mut forged = request.clone();
    forged.created_at = i64::MIN;
    assert!(source.sync_reply(&forged, 4096).is_err());
    let mut forged = request.clone();
    forged.recipient = recipient.clone();
    assert!(source.sync_reply(&forged, 4096).is_err());
    let stranger = Database::open_in_memory().unwrap();
    assert!(stranger.sync_reply(&request, 4096).is_err());
    assert!(source.sync_request(&sender).is_err());
    // A too-small ordinary page is an error, not a reason to allocate a snapshot.
    assert!(source.sync_reply(&request, 1).is_err());
    let sessions: i64 = source
        .conn
        .query_row(
            "SELECT COUNT(*) FROM sync_snapshot_sessions WHERE direction='out'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(sessions, 0);
    let legitimate = source.sync_reply(&request, 1024 * 1024).unwrap();
    let (plaintext, _) = legitimate
        .open(crate::apply::PROTOCOL_NAME, &destination.identity.key)
        .unwrap();
    let mut legacy_reset: serde_json::Value = serde_json::from_slice(&plaintext).unwrap();
    legacy_reset["batch"]["snapshot_reset_keys"] = serde_json::json!(["not-a-paged-manifest"]);
    let legacy_reset = syncnet::wire::bundle_v2::ProtectedBundleV2::seal(
        crate::apply::PROTOCOL_NAME,
        &source.identity.key,
        destination.identity.key.verifying_key().as_bytes(),
        &serde_json::to_vec(&legacy_reset).unwrap(),
    )
    .unwrap();
    let before = count(&destination);
    assert!(destination.import_sync_page(&legacy_reset).is_err());
    assert_eq!(count(&destination), before);
    source
        .conn
        .execute(
            "UPDATE sync_trust_records SET revoked=1 WHERE kind='membership' AND subject_key=?1",
            [&recipient],
        )
        .unwrap();
    assert!(source.sync_reply(&request, 4096).is_err());
}

#[test]
fn automatic_exchange_chooses_recovery_resumes_after_restart_and_returns_to_delta() {
    use crate::exchange::SyncReceiveOutcome;
    use syncnet::identity::save_key_exclusive;
    let dir = TestDir::new();
    let source_path = dir.0.join("auto-source.sqlite");
    let destination_path = dir.0.join("auto-destination.sqlite");
    let source = Database::open(&source_path).unwrap();
    let destination = join_for_test(&source);
    let sender = source.sync_identity().device_public_key;
    let recipient = destination.sync_identity().device_public_key;
    copy_membership_and_role(&source, &destination, &sender);
    destination
        .conn
        .execute("VACUUM INTO ?1", [destination_path.to_str().unwrap()])
        .unwrap();
    save_key_exclusive(
        &key_path(&destination_path, "device"),
        &destination.identity.key,
    )
    .unwrap();
    drop(destination);
    let destination = Database::open(&destination_path).unwrap();
    let list = source
        .create_shopping_list("Automatic recovery", None)
        .unwrap();
    let item = source
        .add_planned_item(list.id, None, "Kruh", 1.0, Unit::Kom, 100)
        .unwrap();
    // Ordinary changes use deltas and do not allocate recovery sessions.
    let request = destination.sync_request(&sender).unwrap();
    let page = source.sync_reply(&request, 1024 * 1024).unwrap();
    assert!(matches!(
        destination.import_sync_page(&page).unwrap(),
        SyncReceiveOutcome::Delta {
            has_more: false,
            ..
        }
    ));
    for i in 0..20 {
        source
            .add_planned_item(
                list.id,
                None,
                &format!("Artikl {i} {}", "Čokolada".repeat(8)),
                1.0,
                Unit::Kom,
                100,
            )
            .unwrap();
    }
    for i in 0..8 {
        source.toggle_planned_item(item.id, i % 2 == 0).unwrap();
    }
    source
        .set_record_archived(LocalEntity::PlannedItem, &item.sync_id, true)
        .unwrap();
    assert!(source.run_scheduled_maintenance().unwrap().unwrap() > 0);
    let request = destination.sync_request(&sender).unwrap();
    // Lose the first response. Its durable outgoing snapshot must be reused.
    let lost = source.sync_reply(&request, 4096).unwrap();
    let (lost_data, _) = lost
        .open(crate::apply::PROTOCOL_NAME, &destination.identity.key)
        .unwrap();
    let first_id: serde_json::Value = serde_json::from_slice(&lost_data).unwrap();
    let first_id = first_id["snapshot_page"]["manifest"]["snapshot_id"]
        .as_str()
        .unwrap()
        .to_owned();
    drop(source);
    let source = Database::open(&source_path).unwrap();
    let first = source.sync_reply(&request, 4096).unwrap();
    let SyncReceiveOutcome::Snapshot { progress } = destination.import_sync_page(&first).unwrap()
    else {
        panic!("expected automatic recovery")
    };
    assert_eq!(progress.snapshot_id, first_id);
    assert!(!progress.complete);
    source
        .create_shopping_list("After automatic snapshot", None)
        .unwrap();
    drop(source);
    drop(destination);
    let source = Database::open(&source_path).unwrap();
    let destination = Database::open(&destination_path).unwrap();
    let resumed = destination.sync_request(&sender).unwrap();
    assert_eq!(resumed.recovery.as_ref().unwrap(), &progress);
    let mut complete = false;
    let mut pages = 0;
    for _ in 0..100 {
        let request = destination.sync_request(&sender).unwrap();
        let page = source.sync_reply(&request, 4096).unwrap();
        assert!(
            page.open(crate::apply::PROTOCOL_NAME, &destination.identity.key)
                .unwrap()
                .0
                .len()
                <= 4096
        );
        match destination.import_sync_page(&page).unwrap() {
            SyncReceiveOutcome::Snapshot { progress: next } => {
                assert_eq!(next.snapshot_id, first_id);
                pages += 1;
                if next.complete {
                    complete = true;
                    break;
                }
            }
            other => panic!("premature delta: {other:?}"),
        }
    }
    assert!(complete && pages > 1);
    assert_eq!(
        destination.list_shopping_lists().unwrap()[0].items.len(),
        20
    );
    let before: i64 = source
        .conn
        .query_row(
            "SELECT COUNT(*) FROM sync_snapshot_items WHERE direction='out'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(before > 0, "exporting last page is not an ACK");
    let receipt = destination.sync_request(&sender).unwrap();
    assert!(receipt.recovery.as_ref().unwrap().complete);
    source.conn.execute_batch("CREATE TEMP TRIGGER fail_recovery_ack BEFORE UPDATE ON sync_snapshot_sessions WHEN NEW.direction='out' AND NEW.complete=1 BEGIN SELECT RAISE(ABORT,'injected recovery ACK failure'); END;").unwrap();
    assert!(source.sync_reply(&receipt, 4096).is_err());
    let retained: i64 = source
        .conn
        .query_row(
            "SELECT COUNT(*) FROM sync_snapshot_items WHERE direction='out' AND peer_key=?1",
            [&recipient],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(retained, before);
    source
        .conn
        .execute_batch("DROP TRIGGER fail_recovery_ack")
        .unwrap();
    let page = source.sync_reply(&receipt, 4096).unwrap();
    assert!(matches!(
        destination.import_sync_page(&page).unwrap(),
        SyncReceiveOutcome::Delta { .. }
    ));
    assert!(
        destination
            .list_shopping_lists()
            .unwrap()
            .iter()
            .any(|l| l.name == "After automatic snapshot")
    );
    let remaining: i64 = source
        .conn
        .query_row(
            "SELECT COUNT(*) FROM sync_snapshot_items WHERE direction='out'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(remaining, 0);
    let next = destination.sync_request(&sender).unwrap();
    let page = source.sync_reply(&next, 4096).unwrap();
    let SyncReceiveOutcome::Delta { report, has_more } =
        destination.import_sync_page(&page).unwrap()
    else {
        panic!("expected delta after ACK")
    };
    assert_eq!(report.applied_versions, 0);
    assert!(!has_more);
}

#[test]
fn automatic_exchange_replaces_lost_outgoing_state_without_resuming_an_abandoned_input() {
    use crate::exchange::SyncReceiveOutcome;
    let source = Database::open_in_memory().unwrap();
    let destination = join_for_test(&source);
    let sender = source.sync_identity().device_public_key;
    let recipient = destination.sync_identity().device_public_key;
    copy_membership_and_role(&source, &destination, &sender);
    let list = source
        .create_shopping_list("Replace interrupted snapshot", None)
        .unwrap();
    let item = source
        .add_planned_item(list.id, None, "Kruh", 1.0, Unit::Kom, 100)
        .unwrap();
    for i in 0..15 {
        source
            .add_planned_item(list.id, None, &format!("Item {i}"), 1.0, Unit::Kom, 100)
            .unwrap();
    }
    for i in 0..8 {
        source.toggle_planned_item(item.id, i % 2 == 0).unwrap();
    }
    source.run_scheduled_maintenance().unwrap();
    let page = source
        .sync_reply(&destination.sync_request(&sender).unwrap(), 4096)
        .unwrap();
    let SyncReceiveOutcome::Snapshot { progress: old } =
        destination.import_sync_page(&page).unwrap()
    else {
        panic!("expected recovery")
    };
    assert!(!old.complete);
    source
        .discard_snapshot_transfer(&old.snapshot_id, &recipient, true)
        .unwrap();
    let page = source
        .sync_reply(&destination.sync_request(&sender).unwrap(), 4096)
        .unwrap();
    let SyncReceiveOutcome::Snapshot { progress: new } =
        destination.import_sync_page(&page).unwrap()
    else {
        panic!("expected replacement")
    };
    assert_ne!(old.snapshot_id, new.snapshot_id);
    let mut complete = new.complete;
    for _ in 0..100 {
        if complete {
            break;
        }
        let request = destination.sync_request(&sender).unwrap();
        assert_eq!(
            request.recovery.as_ref().unwrap().snapshot_id,
            new.snapshot_id
        );
        let page = source.sync_reply(&request, 4096).unwrap();
        let SyncReceiveOutcome::Snapshot { progress } =
            destination.import_sync_page(&page).unwrap()
        else {
            panic!("expected replacement continuation")
        };
        complete = progress.complete;
    }
    assert!(complete);
    // The older partial input is retained for explicit cleanup, but never wins routing.
    assert!(
        !destination
            .snapshot_progress(&sender, &old.snapshot_id)
            .unwrap()
            .unwrap()
            .complete
    );
    let request = destination.sync_request(&sender).unwrap();
    assert_eq!(
        request.recovery.as_ref().unwrap().snapshot_id,
        new.snapshot_id
    );
    let page = source.sync_reply(&request, 4096).unwrap();
    assert!(matches!(
        destination.import_sync_page(&page).unwrap(),
        SyncReceiveOutcome::Delta { .. }
    ));
    assert_eq!(
        destination.list_shopping_lists().unwrap()[0].items.len(),
        16
    );
}

#[test]
fn snapshot_preserves_newer_local_edits_and_rechecks_own_device_permissions() {
    let source = Database::open_in_memory().unwrap();
    let destination = join_for_test(&source);
    let recipient = destination.sync_identity().device_public_key;
    let sender = source.sync_identity().device_public_key;
    copy_membership_and_role(&source, &destination, &sender);
    let list = source
        .create_shopping_list("Keep newer edit", None)
        .unwrap();
    let item = source
        .add_planned_item(list.id, None, "Kruh", 1.0, Unit::Kom, 100)
        .unwrap();
    let manifest = source.start_snapshot_recovery(&recipient).unwrap();
    transfer(&source, &destination);
    assert!(
        destination.start_snapshot_recovery(&sender).is_err(),
        "a replica with unconfirmed holes must not certify a snapshot"
    );
    let local_item = destination.list_shopping_lists().unwrap()[0].items[0].id;
    destination.toggle_planned_item(local_item, true).unwrap();
    let page = source
        .export_snapshot_page(&manifest.snapshot_id, &recipient, 0, 1024 * 1024)
        .unwrap();
    // A matching content hash and valid outer signature cannot replace each
    // original version's signature. The entire final transaction must fail.
    let (plaintext, _) = page
        .open(crate::apply::PROTOCOL_NAME, &destination.identity.key)
        .unwrap();
    let mut malformed: serde_json::Value = serde_json::from_slice(&plaintext).unwrap();
    malformed["snapshot_page"]["versions"][0]["signature"] = "".into();
    let altered: Vec<Version> =
        serde_json::from_value(malformed["snapshot_page"]["versions"].clone()).unwrap();
    let data = serde_json::to_vec(&altered).unwrap();
    use sha2::{Digest, Sha256};
    malformed["snapshot_page"]["manifest"]["sha256"] = hex::encode(Sha256::digest(&data)).into();
    malformed["snapshot_page"]["manifest"]["total_bytes"] = data.len().into();
    let invalid = syncnet::wire::bundle_v2::ProtectedBundleV2::seal(
        crate::apply::PROTOCOL_NAME,
        &source.identity.key,
        destination.identity.key.verifying_key().as_bytes(),
        &serde_json::to_vec(&malformed).unwrap(),
    )
    .unwrap();
    assert!(destination.import_snapshot_page(&invalid).is_err());
    assert!(
        destination
            .snapshot_progress(&sender, &manifest.snapshot_id)
            .unwrap()
            .is_none()
    );
    let stranger = Database::open_in_memory().unwrap();
    assert!(stranger.import_snapshot_page(&page).is_err());
    assert!(stranger.start_snapshot_recovery(&recipient).is_err());
    destination
        .conn
        .execute(
            "UPDATE sync_trust_records SET revoked=1 WHERE kind='membership' AND subject_key=?1",
            [&sender],
        )
        .unwrap();
    assert!(destination.import_snapshot_page(&page).is_err());
    assert!(
        destination
            .snapshot_progress(&sender, &manifest.snapshot_id)
            .unwrap()
            .is_none()
    );
    destination
        .conn
        .execute(
            "UPDATE sync_trust_records SET revoked=0 WHERE kind='membership' AND subject_key=?1",
            [&sender],
        )
        .unwrap();
    assert!(destination.import_snapshot_page(&page).unwrap().complete);
    assert!(destination.list_shopping_lists().unwrap()[0].items[0].checked);
    assert!(!source.list_shopping_lists().unwrap()[0].items[0].checked);
    source
        .conn
        .execute(
            "UPDATE sync_trust_records SET revoked=1 WHERE kind='membership' AND subject_key=?1",
            [&recipient],
        )
        .unwrap();
    assert!(
        source
            .export_snapshot_page(&manifest.snapshot_id, &recipient, 0, 4096)
            .is_err()
    );
    assert!(source.start_snapshot_recovery(&recipient).is_err());
    assert!(source.toggle_planned_item(item.id, true).is_ok());
}

#[test]
fn snapshot_preparation_rolls_back_and_transfer_quotas_are_recoverable() {
    let source = Database::open_in_memory().unwrap();
    let destination = join_for_test(&source);
    let recipient = destination.sync_identity().device_public_key;
    let before = count(&source);
    source.conn.execute_batch("CREATE TEMP TRIGGER fail_snapshot_stage BEFORE INSERT ON sync_snapshot_items BEGIN SELECT RAISE(ABORT,'injected stage failure'); END;").unwrap();
    assert!(source.start_snapshot_recovery(&recipient).is_err());
    let sessions: i64 = source
        .conn
        .query_row("SELECT COUNT(*) FROM sync_snapshot_sessions", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(sessions, 0);
    source
        .conn
        .execute_batch("DROP TRIGGER fail_snapshot_stage")
        .unwrap();
    let mut manifests = Vec::new();
    for _ in 0..8 {
        manifests.push(source.start_snapshot_recovery(&recipient).unwrap());
    }
    assert!(source.start_snapshot_recovery(&recipient).is_err());
    assert!(
        source
            .export_snapshot_page(&manifests[0].snapshot_id, &recipient, usize::MAX, 4096)
            .is_err()
    );
    assert!(
        source
            .export_snapshot_page(&manifests[0].snapshot_id, &recipient, 0, 1)
            .is_err()
    );
    source
        .discard_snapshot_transfer(&manifests[0].snapshot_id, &recipient, true)
        .unwrap();
    assert!(source.start_snapshot_recovery(&recipient).is_ok());
    assert_eq!(count(&source), before);
}

#[test]
fn snapshot_pages_resume_after_both_databases_reopen_and_commit_only_at_completion() {
    use syncnet::identity::save_key_exclusive;
    let dir = TestDir::new();
    let source_path = dir.0.join("snapshot-source.sqlite");
    let destination_path = dir.0.join("snapshot-recipient.sqlite");
    let source = Database::open(&source_path).unwrap();
    let destination = join_for_test(&source);
    copy_membership_and_role(
        &source,
        &destination,
        &source.sync_identity().device_public_key,
    );
    destination
        .conn
        .execute("VACUUM INTO ?1", [destination_path.to_str().unwrap()])
        .unwrap();
    save_key_exclusive(
        &key_path(&destination_path, "device"),
        &destination.identity.key,
    )
    .unwrap();
    drop(destination);
    let destination = Database::open(&destination_path).unwrap();
    let list = source.create_shopping_list("Snapshot test", None).unwrap();
    let mut first_item = None;
    for i in 0..24 {
        let item = source
            .add_planned_item(
                list.id,
                None,
                &format!("Item {i} {}", "Čokolada \\\"".repeat(5)),
                1.0,
                Unit::Kom,
                100,
            )
            .unwrap();
        if i == 0 {
            first_item = Some(item);
        }
    }
    let item = first_item.unwrap();
    for i in 0..8 {
        source.toggle_planned_item(item.id, i % 2 == 0).unwrap();
    }
    source
        .set_record_archived(LocalEntity::PlannedItem, &item.sync_id, true)
        .unwrap();
    assert!(source.run_scheduled_maintenance().unwrap().unwrap() > 0);
    let recipient = destination.sync_identity().device_public_key;
    let sender = source.sync_identity().device_public_key;
    let manifest = source.start_snapshot_recovery(&recipient).unwrap();
    let first = source
        .export_snapshot_page(&manifest.snapshot_id, &recipient, 0, 4096)
        .unwrap();
    assert!(
        first
            .open(crate::apply::PROTOCOL_NAME, &destination.identity.key)
            .unwrap()
            .0
            .len()
            <= 4096
    );
    let initial = destination.import_snapshot_page(&first).unwrap();
    assert!(!initial.complete);
    assert!(destination.list_shopping_lists().unwrap().is_empty());
    source
        .create_shopping_list("After frozen snapshot", None)
        .unwrap();
    drop(source);
    drop(destination);
    let source = Database::open(&source_path).unwrap();
    let destination = Database::open(&destination_path).unwrap();
    assert_eq!(
        destination
            .snapshot_progress(&sender, &manifest.snapshot_id)
            .unwrap()
            .unwrap(),
        initial
    );
    assert_eq!(destination.import_snapshot_page(&first).unwrap(), initial);
    let mut next = initial.next_offset;
    let skip = source
        .export_snapshot_page(&manifest.snapshot_id, &recipient, next + 1, 4096)
        .unwrap();
    assert!(destination.import_snapshot_page(&skip).is_err());
    let before = count(&destination);
    let mut pages = 1;
    loop {
        let page = source
            .export_snapshot_page(&manifest.snapshot_id, &recipient, next, 4096)
            .unwrap();
        let (plaintext, _) = page
            .open(crate::apply::PROTOCOL_NAME, &destination.identity.key)
            .unwrap();
        assert!(plaintext.len() <= 4096);
        let json: serde_json::Value = serde_json::from_slice(&plaintext).unwrap();
        if json["snapshot_page"]["done"] == true {
            // A validly encrypted but changed final page cannot complete the manifest.
            let mut changed = json.clone();
            changed["snapshot_page"]["versions"][0]["payload"] = "corrupt".into();
            let corrupted = syncnet::wire::bundle_v2::ProtectedBundleV2::seal(
                crate::apply::PROTOCOL_NAME,
                &source.identity.key,
                destination.identity.key.verifying_key().as_bytes(),
                &serde_json::to_vec(&changed).unwrap(),
            )
            .unwrap();
            assert!(destination.import_snapshot_page(&corrupted).is_err());
            assert_eq!(
                destination
                    .snapshot_progress(&sender, &manifest.snapshot_id)
                    .unwrap()
                    .unwrap()
                    .next_offset,
                next
            );
            destination.conn.execute_batch("CREATE TEMP TRIGGER fail_snapshot_finish BEFORE UPDATE ON sync_snapshot_sessions WHEN NEW.complete=1 BEGIN SELECT RAISE(ABORT,'injected snapshot finish failure'); END;").unwrap();
            assert!(destination.import_snapshot_page(&page).is_err());
            assert_eq!(count(&destination), before);
            assert!(destination.list_shopping_lists().unwrap().is_empty());
            assert_eq!(
                destination
                    .snapshot_progress(&sender, &manifest.snapshot_id)
                    .unwrap()
                    .unwrap()
                    .next_offset,
                next
            );
            destination
                .conn
                .execute_batch("DROP TRIGGER fail_snapshot_finish")
                .unwrap();
            let progress = destination.import_snapshot_page(&page).unwrap();
            assert!(progress.complete);
            assert_eq!(progress.next_offset, manifest.total_versions);
            let after = count(&destination);
            assert_eq!(destination.import_snapshot_page(&page).unwrap(), progress);
            assert_eq!(count(&destination), after);
            break;
        }
        let progress = destination.import_snapshot_page(&page).unwrap();
        assert!(!progress.complete);
        assert!(progress.next_offset > next);
        next = progress.next_offset;
        pages += 1;
        assert!(destination.list_shopping_lists().unwrap().is_empty());
    }
    assert!(pages > 2);
    let lists = destination.list_shopping_lists().unwrap();
    assert_eq!(lists.len(), 1);
    assert_eq!(lists[0].name, "Snapshot test");
    assert_eq!(lists[0].items.len(), 23);
    assert!(
        crate::ledger::list_holes(&destination.conn)
            .unwrap()
            .is_empty()
    );
    let staged: i64 = destination
        .conn
        .query_row(
            "SELECT COUNT(*) FROM sync_snapshot_items WHERE direction='in'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(staged, 0);
    drop(destination);
    let destination = Database::open(&destination_path).unwrap();
    assert!(
        destination
            .snapshot_progress(&sender, &manifest.snapshot_id)
            .unwrap()
            .unwrap()
            .complete
    );
    assert_eq!(
        destination.list_shopping_lists().unwrap()[0].items.len(),
        23
    );
    let following = source
        .export_protected_bundle(
            &recipient,
            &crate::ledger::get_confirmed_frontier(&destination.conn, &sender).unwrap(),
            &crate::ledger::list_holes(&destination.conn).unwrap(),
            4096,
        )
        .unwrap();
    assert!(
        destination
            .import_protected_bundle(&following)
            .unwrap()
            .rejected
            .is_empty()
    );
    assert!(
        destination
            .list_shopping_lists()
            .unwrap()
            .iter()
            .any(|list| list.name == "After frozen snapshot")
    );
    source
        .discard_snapshot_transfer(&manifest.snapshot_id, &recipient, true)
        .unwrap();
    assert!(
        source
            .export_snapshot_page(&manifest.snapshot_id, &recipient, 0, 4096)
            .is_err()
    );
}

#[test]
fn protected_bundle_budget_includes_the_sender_wrapper() {
    let source = Database::open_in_memory().unwrap();
    let destination = join_for_test(&source);
    source
        .create_shopping_list("Čokolada \\\"\n", None)
        .unwrap();
    let recipient = destination.sync_identity().device_public_key;
    let full = source
        .export_protected_bundle(&recipient, &BTreeMap::new(), &[], 1024 * 1024)
        .unwrap();
    let (plaintext, _) = full
        .open(crate::apply::PROTOCOL_NAME, &destination.identity.key)
        .unwrap();
    let length = plaintext.len();
    let exact = source
        .export_protected_bundle(&recipient, &BTreeMap::new(), &[], length)
        .unwrap();
    assert_eq!(
        exact
            .open(crate::apply::PROTOCOL_NAME, &destination.identity.key)
            .unwrap()
            .0
            .len(),
        length
    );
    let smaller = source
        .export_protected_bundle(&recipient, &BTreeMap::new(), &[], length - 1)
        .unwrap();
    let (plaintext, _) = smaller
        .open(crate::apply::PROTOCOL_NAME, &destination.identity.key)
        .unwrap();
    assert!(plaintext.len() < length);
    let payload: crate::apply::ProtectedSyncPayload = serde_json::from_slice(&plaintext).unwrap();
    assert!(payload.batch.has_more);
    assert!(
        source
            .export_protected_bundle(&recipient, &BTreeMap::new(), &[], 1)
            .is_err()
    );
}

#[test]
fn database_protected_bundle_round_trip_replay_and_tamper_rejection() {
    let source = Database::open_in_memory().unwrap();
    let destination = join_for_test(&source);
    // Pairing must establish the sender's membership before private data admission.
    copy_membership_and_role(
        &source,
        &destination,
        &source.sync_identity().device_public_key,
    );
    source.create_shopping_list("Privatni popis", None).unwrap();
    let bundle = source
        .export_protected_bundle(
            &destination.sync_identity().device_public_key,
            &BTreeMap::new(),
            &[],
            512 * 1024,
        )
        .unwrap();
    let report = destination.import_protected_bundle(&bundle).unwrap();
    assert!(report.rejected.is_empty(), "{:?}", report.rejected);
    assert_eq!(destination.list_shopping_lists().unwrap().len(), 1);
    assert_eq!(
        destination
            .import_protected_bundle(&bundle)
            .unwrap()
            .applied_versions,
        0
    );
    let before = count(&destination);
    let mut corrupted = bundle.clone();
    corrupted.ciphertext[0] ^= 1;
    assert!(destination.import_protected_bundle(&corrupted).is_err());
    assert_eq!(count(&destination), before);
    let stranger = Database::open_in_memory().unwrap();
    let before = count(&stranger);
    assert!(stranger.import_protected_bundle(&bundle).is_err());
    assert_eq!(count(&stranger), before);
}

#[test]
fn actual_local_commands_produce_full_signed_snapshots_and_replicate() {
    let a = Database::open_in_memory().unwrap();
    let b = join_for_test(&a);
    let store = a.list_stores().unwrap().remove(0);
    let list = a.create_shopping_list("Tjedan", Some(store.id)).unwrap();
    let item = a
        .add_planned_item(list.id, Some("38501"), "Kruh", 1.5, Unit::Kom, 150)
        .unwrap();
    a.toggle_planned_item(item.id, true).unwrap();
    a.save_receipt(
        store.id,
        &store.name,
        &[CartItem {
            id: "draft".into(),
            barcode: "38501".into(),
            name: "Kruh".into(),
            quantity: 1.5,
            unit: Unit::Kom,
            unit_price_cents: 150,
            total_cents: 999,
            image_data_url: None,
        }],
        225,
    )
    .unwrap();
    let recorded = versions(&a);
    for v in &recorded {
        let env = syncnet::trust::SignedVersionEnvelope {
            version_id: v.version_id.clone(),
            entity: v.entity.clone(),
            entity_id: v.entity_id.clone(),
            node_id: v.node_id.clone(),
            supersedes: v.supersedes.clone(),
            archived: v.archived,
            channel: v.channel.clone(),
            created_at: v.created_at.clone(),
            created_at_unix: v.created_at_unix,
            schema_version: v.schema_version,
            author_pub_key: v.author_pub_key.clone(),
            grant_ref: v.grant_ref.clone(),
            payload: v.payload.clone(),
            signature: v.signature.clone(),
        };
        env.verify_signature(crate::apply::PROTOCOL_NAME).unwrap();
        if v.channel != "control" {
            assert_eq!(v.channel, format!("private:{}", a.identity.principal));
            validate_payload_syntax_only(v).unwrap();
        }
    }
    for table in ENTITIES {
        assert!(
            recorded.iter().any(|v| v.entity == table),
            "missing {table}"
        );
    }
    let report = transfer(&a, &b);
    assert!(report.rejected.is_empty(), "{:?}", report.rejected);
    assert_eq!(b.list_receipts().unwrap()[0].cart_total_cents, 225);
    assert_eq!(b.list_receipts().unwrap()[0].items[0].quantity, 1.5);
    let got = b.list_shopping_lists().unwrap().remove(0);
    assert_eq!(got.sync_id, list.sync_id);
    assert!(got.items[0].checked);
    assert_eq!(got.items[0].sync_id, item.sync_id);
    assert_eq!(
        b.get_product_store_prices("38501").unwrap()[0].price_cents,
        150
    );
    assert_eq!(transfer(&a, &b).applied_versions, 0);
}

#[test]
fn ledger_failure_rolls_back_every_local_mutator() {
    let db = Database::open_in_memory().unwrap();
    let store = db.list_stores().unwrap().remove(0);
    let list = db.create_shopping_list("test", None).unwrap();
    let item = db
        .add_planned_item(list.id, None, "Kruh", 1.0, Unit::Kom, 100)
        .unwrap();
    let before = count(&db);
    db.conn.execute_batch("CREATE TRIGGER fail_ledger BEFORE INSERT ON record_versions BEGIN SELECT RAISE(ABORT,'injected ledger failure'); END;").unwrap();
    assert!(db.add_store("Must rollback", "", "#123456").is_err());
    assert!(
        db.upsert_product(&UpsertProductInput {
            barcode: "bad",
            name: "x",
            category: "Opce",
            default_unit: Unit::Kom,
            price_cents: 100,
            store_name: &store.name,
            image_data_url: None
        })
        .is_err()
    );
    assert!(db.create_shopping_list("rollback", None).is_err());
    assert!(
        db.add_planned_item(list.id, None, "Rollback", 1.0, Unit::Kom, 100)
            .is_err()
    );
    assert!(db.toggle_planned_item(item.id, true).is_err());
    assert!(db.save_receipt(store.id, &store.name, &[], 0).is_err());
    assert!(
        db.set_record_archived(LocalEntity::ShoppingList, &list.sync_id, true)
            .is_err()
    );
    assert_eq!(count(&db), before);
    assert_eq!(db.list_stores().unwrap().len(), 5);
    assert!(db.get_product("bad").unwrap().is_none());
    assert!(db.list_receipts().unwrap().is_empty());
    let lists = db.list_shopping_lists().unwrap();
    assert_eq!(lists.len(), 1);
    assert_eq!(lists[0].items.len(), 1);
    assert!(!lists[0].items[0].checked);
}

#[test]
fn archive_restore_and_stale_replay_preserve_tombstones() {
    let a = Database::open_in_memory().unwrap();
    let b = join_for_test(&a);
    let list = a.create_shopping_list("test", None).unwrap();
    let item = a
        .add_planned_item(list.id, None, "Kruh", 1.0, Unit::Kom, 100)
        .unwrap();
    let stale = versions(&a);
    a.set_record_archived(LocalEntity::PlannedItem, &item.sync_id, true)
        .unwrap();
    assert!(a.list_shopping_lists().unwrap()[0].items.is_empty());
    assert!(a.toggle_planned_item(item.id, true).is_err());
    assert!(transfer(&a, &b).rejected.is_empty());
    assert!(b.list_shopping_lists().unwrap()[0].items.is_empty());
    let recipient = b.sync_identity().device_public_key;
    apply_remote_versions(
        &b.conn,
        ApplyContext {
            network_pub: &a.identity.network_public,
            peer_key: "stale",
            recipient_b64: Some(&recipient),
            now_unix: Utc::now().timestamp(),
        },
        &stale,
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
    .unwrap();
    assert!(b.list_shopping_lists().unwrap()[0].items.is_empty());
    a.set_record_archived(LocalEntity::PlannedItem, &item.sync_id, false)
        .unwrap();
    a.set_record_archived(LocalEntity::ShoppingList, &list.sync_id, true)
        .unwrap();
    assert!(a.list_shopping_lists().unwrap().is_empty());
    assert!(
        a.add_planned_item(list.id, None, "hidden", 1.0, Unit::Kom, 1)
            .is_err()
    );
    a.set_record_archived(LocalEntity::ShoppingList, &list.sync_id, false)
        .unwrap();
    assert!(transfer(&a, &b).rejected.is_empty());
    assert_eq!(b.list_shopping_lists().unwrap()[0].items.len(), 1);
}

#[test]
fn disk_identity_is_stable_separate_and_missing_key_fails_without_replacement() {
    let dir = TestDir::new();
    let path = dir.0.join("test.sqlite");
    let db = Database::open(&path).unwrap();
    db.create_shopping_list("persisted", None).unwrap();
    let identity = db.sync_identity();
    let before = count(&db);
    drop(db);
    let pem = std::fs::read(key_path(&path, "device")).unwrap();
    let reopened = Database::open(&path).unwrap();
    assert_eq!(
        reopened.sync_identity().device_public_key,
        identity.device_public_key
    );
    assert_eq!(count(&reopened), before);
    assert_eq!(reopened.list_shopping_lists().unwrap()[0].name, "persisted");
    drop(reopened);
    assert!(
        !std::fs::read(&path)
            .unwrap()
            .windows(pem.len())
            .any(|s| s == pem)
    );
    std::fs::rename(key_path(&path, "device"), path.with_extension("saved.key")).unwrap();
    assert!(Database::open(&path).is_err());
    assert!(!key_path(&path, "device").exists());
    std::fs::rename(path.with_extension("saved.key"), key_path(&path, "device")).unwrap();
    assert_eq!(count(&Database::open(&path).unwrap()), before);
    let other = Database::open(&path.with_extension("db")).unwrap();
    assert_ne!(
        other.sync_identity().device_public_key,
        identity.device_public_key
    );
    assert_ne!(
        other.sync_identity().network_public_key,
        identity.network_public_key
    );
}

#[test]
fn invalid_input_and_unknown_items_do_not_leave_partial_versions() {
    let db = Database::open_in_memory().unwrap();
    let list = db.create_shopping_list("valid", None).unwrap();
    let before = count(&db);
    for q in [f64::NAN, f64::INFINITY, -1.0, 0.0, 0.0001, 1.0001] {
        assert!(
            db.add_planned_item(list.id, None, "x", q, Unit::Kom, 100)
                .is_err()
        );
    }
    assert!(db.toggle_planned_item(99999, true).is_err());
    assert!(db.create_shopping_list(" ", None).is_err());
    assert_eq!(count(&db), before);
    assert!(db.list_shopping_lists().unwrap()[0].items.is_empty());
}

#[test]
fn reversed_entity_delivery_resolves_store_receipt_item_and_product_price_dependencies() {
    let a = Database::open_in_memory().unwrap();
    let b = join_for_test(&a);
    let store = a
        .add_store("Neovisna trgovina", "kvart", "#123456")
        .unwrap();
    a.save_receipt(
        store.id,
        &store.name,
        &[CartItem {
            id: "draft".into(),
            barcode: "38599".into(),
            name: "Brasno".into(),
            quantity: 1.25,
            unit: Unit::Kg,
            unit_price_cents: 130,
            total_cents: 0,
            image_data_url: None,
        }],
        163,
    )
    .unwrap();
    let all = versions(&a);
    let recipient = b.sync_identity().device_public_key;
    let receive = |v: &Version| {
        apply_remote_versions(
            &b.conn,
            ApplyContext {
                network_pub: &a.identity.network_public,
                peer_key: &a.sync_identity().device_public_key,
                recipient_b64: Some(&recipient),
                now_unix: Utc::now().timestamp(),
            },
            std::slice::from_ref(v),
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .unwrap()
    };
    for v in all.iter().filter(|v| v.channel == "control") {
        assert!(receive(v).rejected.is_empty());
    }
    for v in all.iter().rev().filter(|v| v.channel != "control") {
        let report = receive(v);
        assert!(report.rejected.is_empty(), "{:?}", report.rejected);
    }
    assert_eq!(
        b.conn
            .query_row("SELECT COUNT(*) FROM pending_projections", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let receipt = b.list_receipts().unwrap().remove(0);
    assert_eq!(receipt.items.len(), 1);
    assert_eq!(receipt.items[0].total_cents, 163);
    let received_store = b
        .list_stores()
        .unwrap()
        .into_iter()
        .find(|s| s.id == receipt.store_id)
        .unwrap();
    assert_eq!(received_store.sync_id, store.sync_id);
    assert_eq!(
        b.get_product_store_prices("38599").unwrap()[0].price_cents,
        130
    );
}

#[test]
fn late_receipt_failure_rolls_back_header_items_catalog_and_history() {
    let db = Database::open_in_memory().unwrap();
    let store = db.list_stores().unwrap().remove(0);
    let before = count(&db);
    db.conn.execute_batch("CREATE TRIGGER fail_last_record BEFORE INSERT ON record_versions WHEN NEW.entity='price_history' BEGIN SELECT RAISE(ABORT,'late failure'); END;").unwrap();
    assert!(
        db.save_receipt(
            store.id,
            &store.name,
            &[CartItem {
                id: "draft".into(),
                barcode: "will-rollback".into(),
                name: "Kruh".into(),
                quantity: 1.0,
                unit: Unit::Kom,
                unit_price_cents: 100,
                total_cents: 100,
                image_data_url: None
            }],
            100
        )
        .is_err()
    );
    assert!(db.list_receipts().unwrap().is_empty());
    assert!(db.get_product("will-rollback").unwrap().is_none());
    assert_eq!(
        db.conn
            .query_row("SELECT COUNT(*) FROM receipt_items", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(
        db.conn
            .query_row("SELECT COUNT(*) FROM price_history", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert_eq!(count(&db), before);
}

#[test]
fn legacy_import_does_not_admit_supplied_signatures_or_change_trust() {
    let source = Database::open_in_memory().unwrap();
    let destination = Database::open_in_memory().unwrap();
    source.create_shopping_list("legacy", None).unwrap();
    let mut bundle = source.export_sync_bundle().unwrap();
    let mut forged = bundle.versions[0].clone();
    forged.version_id = Uuid::now_v7().to_string();
    forged.entity = "revocation".into();
    forged.payload = "untrusted".into();
    let forged_id = forged.version_id.clone();
    bundle.versions.push(forged);
    let before: i64 = destination
        .conn
        .query_row("SELECT COUNT(*) FROM sync_trust_records", [], |r| r.get(0))
        .unwrap();
    destination.import_sync_bundle(&bundle).unwrap();
    assert_eq!(
        destination
            .conn
            .query_row("SELECT COUNT(*) FROM sync_trust_records", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        before
    );
    assert_eq!(
        destination
            .conn
            .query_row(
                "SELECT COUNT(*) FROM record_versions WHERE version_id=?1",
                [forged_id],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    let adoption = versions(&destination)
        .into_iter()
        .find(|v| v.entity == "shopping_lists")
        .unwrap();
    assert_eq!(
        adoption.author_pub_key,
        destination.sync_identity().device_public_key
    );
    assert_eq!(
        adoption.channel,
        format!("private:{}", destination.identity.principal)
    );
}

#[test]
fn aggregate_overflow_is_rejected_atomically_for_lists_and_receipts() {
    let db = Database::open_in_memory().unwrap();
    let store = db.list_stores().unwrap().remove(0);
    let list = db.create_shopping_list("limit", None).unwrap();
    db.add_planned_item(
        list.id,
        None,
        "max",
        1.0,
        Unit::Kom,
        crate::calc::MAX_SAFE_CENTS,
    )
    .unwrap();
    let before = count(&db);
    assert!(
        db.add_planned_item(list.id, None, "one too many", 1.0, Unit::Kom, 1)
            .is_err()
    );
    assert_eq!(count(&db), before);
    assert_eq!(db.list_shopping_lists().unwrap()[0].items.len(), 1);
    let item = CartItem {
        id: "draft".into(),
        barcode: String::new(),
        name: "max".into(),
        quantity: 1.0,
        unit: Unit::Kom,
        unit_price_cents: crate::calc::MAX_SAFE_CENTS,
        total_cents: 0,
        image_data_url: None,
    };
    assert!(
        db.save_receipt(store.id, &store.name, &[item.clone(), item], 0)
            .is_err()
    );
    assert!(db.list_receipts().unwrap().is_empty());
    assert_eq!(count(&db), before);
}

#[test]
fn every_entity_archive_and_restore_replicates_with_receipt_totals() {
    let a = Database::open_in_memory().unwrap();
    let b = join_for_test(&a);
    let store = a.add_store("Archive test", "", "#123456").unwrap();
    let receipt = a
        .save_receipt(
            store.id,
            &store.name,
            &[CartItem {
                id: "draft".into(),
                barcode: "archive-product".into(),
                name: "Kruh".into(),
                quantity: 1.0,
                unit: Unit::Kom,
                unit_price_cents: 100,
                total_cents: 100,
                image_data_url: None,
            }],
            100,
        )
        .unwrap();
    let list = a
        .create_shopping_list("Archive list", Some(store.id))
        .unwrap();
    let item = a
        .add_planned_item(list.id, None, "Kruh", 1.0, Unit::Kom, 100)
        .unwrap();
    let product = a.get_product("archive-product").unwrap().unwrap();
    let receipt_item: String = a
        .conn
        .query_row(
            "SELECT sync_id FROM receipt_items WHERE receipt_id=?1",
            [receipt.id],
            |r| r.get(0),
        )
        .unwrap();
    let price: String = a
        .conn
        .query_row(
            "SELECT sync_id FROM price_history WHERE barcode=?1",
            [&product.barcode],
            |r| r.get(0),
        )
        .unwrap();
    assert!(transfer(&a, &b).rejected.is_empty());
    for (entity, id) in [
        (LocalEntity::Store, store.sync_id),
        (LocalEntity::Product, product.sync_id),
        (LocalEntity::Price, price),
        (LocalEntity::Receipt, receipt.sync_id),
        (LocalEntity::ReceiptItem, receipt_item),
        (LocalEntity::ShoppingList, list.sync_id),
        (LocalEntity::PlannedItem, item.sync_id),
    ] {
        for archived in [true, false] {
            a.set_record_archived(entity, &id, archived).unwrap();
            assert!(transfer(&a, &b).rejected.is_empty());
            for db in [&a, &b] {
                let got: bool = db
                    .conn
                    .query_row(
                        &format!("SELECT archived FROM {} WHERE sync_id=?1", entity.table()),
                        [&id],
                        |r| r.get(0),
                    )
                    .unwrap();
                assert_eq!(got, archived, "{}", entity.table());
                if matches!(entity, LocalEntity::ReceiptItem) {
                    let receipt = db.list_receipts().unwrap().remove(0);
                    assert_eq!(receipt.items.len(), usize::from(!archived));
                    assert_eq!(receipt.cart_total_cents, if archived { 0 } else { 100 });
                    assert_eq!(receipt.difference_cents, if archived { -100 } else { 0 });
                }
            }
        }
    }
}

#[test]
fn reopened_replica_does_not_resign_received_data_or_seed_a_new_catalog() {
    let a = Database::open_in_memory().unwrap();
    let b = join_for_test(&a);
    a.create_shopping_list("From another device", None).unwrap();
    assert!(transfer(&a, &b).rejected.is_empty());
    let before = versions(&b);
    let dir = TestDir::new();
    let path = dir.0.join("replica.sqlite");
    b.conn
        .execute("VACUUM INTO ?1", [path.to_str().unwrap()])
        .unwrap();
    syncnet::identity::save_key_exclusive(&key_path(&path, "device"), &b.identity.key).unwrap();
    let reopened = Database::open(&path).unwrap();
    assert_eq!(count(&reopened) as usize, before.len());
    assert_eq!(
        reopened.list_stores().unwrap().len(),
        a.list_stores().unwrap().len()
    );
    let latest = versions(&reopened)
        .into_iter()
        .rfind(|v| v.entity == "shopping_lists")
        .unwrap();
    assert_eq!(latest.author_pub_key, a.sync_identity().device_public_key);
    assert_eq!(
        reopened.sync_identity().device_public_key,
        b.sync_identity().device_public_key
    );
    assert!(!key_path(&path, "network").exists());
}

#[test]
fn legacy_database_is_adopted_once_without_forging_the_historical_author() {
    let dir = TestDir::new();
    let path = dir.0.join("legacy.sqlite");
    let legacy = Connection::open(&path).unwrap();
    legacy
        .execute_batch(include_str!("../../../testdata/legacy-v1.sql"))
        .unwrap();
    drop(legacy);
    let a = Database::open(&path).unwrap();
    assert_eq!(a.list_stores().unwrap().len(), 1);
    assert_eq!(a.list_stores().unwrap()[0].id, 7);
    let receipt = a.list_receipts().unwrap().remove(0);
    assert_eq!(
        (receipt.cart_total_cents, receipt.difference_cents),
        (225, -5)
    );
    assert_eq!(receipt.items[0].quantity, 1.5);
    let list = a.list_shopping_lists().unwrap().remove(0);
    assert_eq!(list.items[0].sync_id, "legacy-planned");
    assert!(list.items[0].checked);
    let all = versions(&a);
    let old = all
        .iter()
        .find(|v| v.node_id == "old-claimed-author")
        .unwrap();
    assert!(old.signature.is_empty());
    assert!(old.author_pub_key.is_empty());
    assert_eq!(old.payload, "{\"name\":\"historical snapshot\"}");
    let signed: Vec<Version> = all
        .iter()
        .filter(|v| !v.signature.is_empty())
        .cloned()
        .collect();
    assert_eq!(signed.iter().filter(|v| v.channel != "control").count(), 7);
    assert!(signed.iter().all(|v| v.supersedes != old.version_id));
    let b = join_for_test(&a);
    let recipient = b.sync_identity().device_public_key;
    let report = apply_remote_versions(
        &b.conn,
        ApplyContext {
            network_pub: &a.identity.network_public,
            peer_key: &a.sync_identity().device_public_key,
            recipient_b64: Some(&recipient),
            now_unix: Utc::now().timestamp(),
        },
        &signed,
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
    .unwrap();
    assert!(report.rejected.is_empty(), "{:?}", report.rejected);
    assert_eq!(b.list_receipts().unwrap()[0].items[0].total_cents, 225);
    assert!(b.list_shopping_lists().unwrap()[0].items[0].checked);
    let before = count(&a);
    let identity = a.sync_identity();
    drop(a);
    let reopened = Database::open(&path).unwrap();
    assert_eq!(count(&reopened), before);
    assert_eq!(
        reopened.sync_identity().device_public_key,
        identity.device_public_key
    );
    assert_eq!(reopened.list_stores().unwrap()[0].sync_id, "legacy-store");
}

#[test]
fn local_writes_after_a_future_peer_keep_order_time_and_independent_ids() {
    use syncnet::trust::SignedVersionEnvelope;
    let a = Database::open_in_memory().unwrap();
    let b = join_for_test(&a);
    let list = a.create_shopping_list("Clock test", None).unwrap();
    assert!(transfer(&a, &b).rejected.is_empty());
    let original = versions(&a)
        .into_iter()
        .rfind(|v| v.entity == "shopping_lists")
        .unwrap();
    let mut env: SignedVersionEnvelope =
        serde_json::from_value(serde_json::to_value(&original).unwrap()).unwrap();
    let future = Utc::now() + chrono::Duration::minutes(30);
    let mut bytes = *Uuid::parse_str(&env.version_id).unwrap().as_bytes();
    bytes[..6].copy_from_slice(&(future.timestamp_millis() as u64).to_be_bytes()[2..]);
    env.version_id = Uuid::from_bytes(bytes).to_string();
    env.created_at = future.to_rfc3339();
    env.created_at_unix = future.timestamp();
    let v: Version = serde_json::from_value(
        serde_json::to_value(env.sign(crate::apply::PROTOCOL_NAME, &a.identity.key)).unwrap(),
    )
    .unwrap();
    for db in [&a, &b] {
        let recipient = db.sync_identity().device_public_key;
        let report = apply_remote_versions(
            &db.conn,
            ApplyContext {
                network_pub: &a.identity.network_public,
                peer_key: &a.sync_identity().device_public_key,
                recipient_b64: Some(&recipient),
                now_unix: Utc::now().timestamp(),
            },
            std::slice::from_ref(&v),
            &BTreeMap::new(),
            &BTreeMap::new(),
        )
        .unwrap();
        assert!(report.rejected.is_empty(), "{:?}", report.rejected);
    }
    let b_list = b.list_shopping_lists().unwrap().remove(0);
    a.add_planned_item(list.id, None, "On A", 1.0, Unit::Kom, 100)
        .unwrap();
    b.add_planned_item(b_list.id, None, "On B", 1.0, Unit::Kom, 100)
        .unwrap();
    let latest = |db: &Database| {
        versions(db)
            .into_iter()
            .rfind(|v| v.entity == "shopping_lists")
            .unwrap()
    };
    assert!(latest(&a).version_id > v.version_id);
    assert!(latest(&b).version_id > v.version_id);
    assert_ne!(latest(&a).version_id, latest(&b).version_id);
    assert!(transfer(&a, &b).rejected.is_empty());
    assert!(transfer(&b, &a).rejected.is_empty());
    assert_eq!(a.list_shopping_lists().unwrap()[0].items.len(), 2);
    assert_eq!(b.list_shopping_lists().unwrap()[0].items.len(), 2);
}

#[test]
fn unsigned_legacy_future_uuid_cannot_poison_the_signed_clock() {
    let db = Database::open_in_memory().unwrap();
    let id = Uuid::from_bytes([
        255, 255, 255, 255, 255, 255, 0x7f, 255, 0xbf, 255, 255, 255, 255, 255, 255, 255,
    ])
    .to_string();
    db.conn.execute("INSERT INTO record_versions(version_id,entity,entity_id,node_id,payload,created_at,legacy_imported) VALUES (?1,'shopping_lists','old','old','{}','2025-01-01T00:00:00Z',1)",[id]).unwrap();
    let list = db.create_shopping_list("Still editable", None).unwrap();
    assert_eq!(db.list_shopping_lists().unwrap()[0].sync_id, list.sync_id);
}

#[test]
fn local_shared_list_mutation_requires_write_grant_and_inherits_the_channel() {
    use crate::acl_types::ListPermission;
    let db = Database::open_in_memory().unwrap();
    let scratch = Database::open_in_memory().unwrap();
    let network =
        NetworkKey::from_signing_key("test", db.identity.network_key.as_ref().unwrap().clone());
    let public = db.sync_identity();
    let id = "shared-test-list";
    let channel = format!("list:{id}");
    let grant = |permission, grant_id| {
        let g = crate::acl_types::issue_list_grant(
            &network,
            grant_id,
            id,
            &db.identity.key.verifying_key(),
            permission,
            "root",
            Validity {
                issued_at: Utc::now().timestamp(),
                valid_for_secs: 3600,
            },
        )
        .unwrap();
        store_control_record(
            &db.conn,
            &network.public,
            &ControlPayload::ListGrant(g),
            Utc::now().timestamp(),
        )
        .unwrap();
    };
    grant(ListPermission::Read, "read-grant");
    let payload = serde_json::json!({"sync_id":id,"name":"Shared","store_sync_id":null,"store_name":null,"updated_at":Utc::now().to_rfc3339(),"archived":false}).to_string();
    let version = record_signed_version(
        &scratch.conn,
        &public.network_public_key,
        db.identity.network_key.as_ref().unwrap(),
        "shopping_lists",
        id,
        false,
        &payload,
        &channel,
        "",
    )
    .unwrap();
    let report = apply_remote_versions(
        &db.conn,
        ApplyContext {
            network_pub: &network.public,
            peer_key: &public.network_public_key,
            recipient_b64: Some(&public.device_public_key),
            now_unix: Utc::now().timestamp(),
        },
        &[version],
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
    .unwrap();
    assert!(report.rejected.is_empty());
    let list = db.list_shopping_lists().unwrap().remove(0);
    let before = count(&db);
    assert!(
        db.add_planned_item(list.id, None, "Denied", 1.0, Unit::Kom, 100)
            .is_err()
    );
    assert_eq!(count(&db), before);
    assert!(db.list_shopping_lists().unwrap()[0].items.is_empty());
    grant(ListPermission::Write, "write-grant");
    db.add_planned_item(list.id, None, "Allowed", 1.0, Unit::Kom, 100)
        .unwrap();
    let local: Vec<_> = versions(&db)
        .into_iter()
        .filter(|v| v.channel == channel && v.author_pub_key == public.device_public_key)
        .collect();
    assert_eq!(local.len(), 2);
    assert!(local.iter().all(|v| v.grant_ref == "write-grant"));
}

#[test]
fn import_sync_package_json_routes_encrypted_v2_and_legacy_v1_cleanly() {
    let source = Database::open_in_memory().unwrap();
    let replica = join_for_test(&source);
    let sender = source.sync_identity().device_public_key;
    copy_membership_and_role(&source, &replica, &sender);
    let store = source.add_store("V2 Dućan", "Split", "#1768D1").unwrap();
    let list = source
        .create_shopping_list("V2 Tjedni popis", Some(store.id))
        .unwrap();
    source
        .add_planned_item(list.id, None, "Kava", 1.0, Unit::Kom, 450)
        .unwrap();

    let req = replica.sync_request(&sender).unwrap();
    let v2_page = source.sync_reply(&req, 256 * 1024).unwrap();
    let v2_json = serde_json::to_string(&v2_page).unwrap();

    // Stranger database cannot import a recipient-bound v2 package.
    let stranger = Database::open_in_memory().unwrap();
    assert!(stranger.import_sync_package_json(&v2_json).is_err());

    // Intended recipient imports the v2 package through import_sync_package_json.
    let (stores, _, lists) = replica.import_sync_package_json(&v2_json).unwrap();
    assert!(stores >= 1);
    assert!(lists >= 1);
    assert_eq!(replica.list_shopping_lists().unwrap().len(), 1);
    assert_eq!(replica.list_shopping_lists().unwrap()[0].items.len(), 1);
}

#[tokio::test]
async fn live_tls13_pairing_controller_admits_only_on_mutual_confirm_and_persists_across_reopen() {
    use crate::pairing_session::PairingController;
    use std::sync::Mutex;
    use syncnet::tls::{accept_exchange_on, dial_exchange_on};

    let dir = TestDir::new();
    let path_a = dir.0.join("admin.sqlite");
    let path_b = dir.0.join("joiner.sqlite");

    let db_a = Mutex::new(Database::open(&path_a).unwrap());
    let db_b = Mutex::new(Database::open(&path_b).unwrap());

    let ctrl_a = PairingController::new();
    let ctrl_b = PairingController::new();

    // 1. First attempt: A listens, B dials, both see the same SAS, but B refuses -> NO membership created!
    let (sk_a, id_a, can_admit_a) = {
        let a = db_a.lock().unwrap();
        let sync_id = a.sync_identity();
        (
            a.clone_device_signing_key(),
            PairingController::build_pair_identity(
                &sync_id.device_public_key,
                "Glavni mobitel",
                "list2cart",
            ),
            a.can_admit_members().unwrap(),
        )
    };
    let (sk_b, id_b, can_admit_b) = {
        let b = db_b.lock().unwrap();
        let sync_id = b.sync_identity();
        (
            b.clone_device_signing_key(),
            PairingController::build_pair_identity(
                &sync_id.device_public_key,
                "Laptop doma",
                "list2cart",
            ),
            b.can_admit_members().unwrap(),
        )
    };

    let st_listen = ctrl_a
        .start_listening(
            "127.0.0.1:0".parse().unwrap(),
            sk_a.clone(),
            id_a.clone(),
            can_admit_a,
        )
        .await
        .unwrap();
    assert!(st_listen.waiting);
    let port = st_listen.listen_port.unwrap();

    let st_b = ctrl_b
        .dial(
            format!("127.0.0.1:{port}").parse().unwrap(),
            sk_b.clone(),
            id_b.clone(),
            can_admit_b,
        )
        .await
        .unwrap();
    assert!(st_b.pending);

    // Wait briefly for listener task to record pending state
    for _ in 0..40 {
        if ctrl_a.status(can_admit_a).pending {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let st_a = ctrl_a.status(can_admit_a);
    assert!(st_a.pending);
    assert_eq!(st_a.sas, st_b.sas);
    assert_eq!(st_a.sas.as_ref().unwrap().len(), 6);

    // B rejects while A approves
    let confirm_a = ctrl_a.confirm(&db_a, true, PeerRole::OwnDevice, "*", true);
    let confirm_b = ctrl_b.confirm(&db_b, false, PeerRole::OwnDevice, "*", false);
    let (out_a, out_b) = tokio::join!(confirm_a, confirm_b);
    let out_a = out_a.unwrap();
    let out_b = out_b.unwrap();
    assert!(!out_a.paired && !out_b.paired);
    assert!(db_a.lock().unwrap().list_members().unwrap().is_empty());
    assert!(db_b.lock().unwrap().list_members().unwrap().is_empty());

    // 2. Second attempt: both sides approve -> B joins A's network, persists across disk reopen, and syncs!
    let st_listen2 = ctrl_a
        .start_listening(
            "127.0.0.1:0".parse().unwrap(),
            sk_a.clone(),
            id_a.clone(),
            can_admit_a,
        )
        .await
        .unwrap();
    let port2 = st_listen2.listen_port.unwrap();
    let st_b2 = ctrl_b
        .dial(
            format!("127.0.0.1:{port2}").parse().unwrap(),
            sk_b.clone(),
            id_b.clone(),
            can_admit_b,
        )
        .await
        .unwrap();
    for _ in 0..40 {
        if ctrl_a.status(can_admit_a).pending {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let st_a2 = ctrl_a.status(can_admit_a);
    assert_eq!(st_a2.sas, st_b2.sas);

    let (ok_a, ok_b) = tokio::join!(
        ctrl_a.confirm(&db_a, true, PeerRole::OwnDevice, "*", true),
        ctrl_b.confirm(&db_b, true, PeerRole::OwnDevice, "*", false),
    );
    let ok_a = ok_a.unwrap();
    let ok_b = ok_b.unwrap();
    assert!(ok_a.paired && ok_a.member && ok_a.admitted);
    assert!(ok_b.paired && ok_b.member);

    // Reopen both databases from disk and verify identity bindings and zero store conflicts
    drop(db_a);
    drop(db_b);
    let reopened_a = Database::open(&path_a).unwrap();
    let reopened_b = Database::open(&path_b).unwrap();
    assert_eq!(
        reopened_a.sync_identity().network_public_key,
        reopened_b.sync_identity().network_public_key
    );
    assert_eq!(
        reopened_a.sync_identity().principal_id,
        reopened_b.sync_identity().principal_id
    );

    // Authenticated TLS 1.3 exchange gate accepts the paired peer
    let (c_io, s_io) = tokio::io::duplex(32 * 1024);
    let a_pub_bytes = sk_a.verifying_key().to_bytes();
    let b_creds = reopened_b.tls_credentials().unwrap();
    let sk_a_srv = reopened_a.clone_device_signing_key();
    let reopened_a_mutex = std::sync::Arc::new(Mutex::new(reopened_a));
    let a_for_gate = reopened_a_mutex.clone();
    let srv = tokio::spawn(async move {
        accept_exchange_on(s_io, &sk_a_srv, "list2cart", None, move |pk, creds| {
            a_for_gate
                .lock()
                .unwrap()
                .verify_or_recover_peer_access(pk, creds)
                .unwrap_or(false)
        })
        .await
    });
    let cli = dial_exchange_on(
        c_io,
        &reopened_b.clone_device_signing_key(),
        "list2cart",
        Some(std::sync::Arc::new(move || b_creds.clone())),
        "127.0.0.1:4610",
        &a_pub_bytes,
    )
    .await
    .expect("paired exchange dial");
    let srv_stream = srv.await.unwrap().expect("paired exchange accept");
    assert_eq!(cli.peer_key, a_pub_bytes);
    assert_eq!(srv_stream.peer_key, sk_b.verifying_key().to_bytes());
}

#[test]
fn unilateral_finish_recovery_recovers_via_tls_credentials_and_idempotent_repairing() {
    use syncnet::pairing::PairHello;

    // Case 1: Joiner B finalized pairing (`WelcomePack`), but Admin A lost connection before `finalize_pairing`.
    let admin = Database::open_in_memory().unwrap();
    let mut joiner = Database::open_in_memory().unwrap();

    let admin_key_bytes = admin.clone_device_signing_key().verifying_key().to_bytes();
    let joiner_key_bytes = joiner.clone_device_signing_key().verifying_key().to_bytes();
    let joiner_b64 = joiner.sync_identity().device_public_key;

    let welcome_for_b = admin
        .prepare_welcome_pack(&joiner_key_bytes, PeerRole::OwnDevice, "*", true)
        .unwrap();

    // Admin has NOT written B's membership to SQLite yet!
    assert!(
        !admin
            .verify_or_recover_peer_access(&joiner_key_bytes, None)
            .unwrap()
    );

    // Joiner B receives `welcome_for_b` and commits locally
    let admin_hello = PairHello {
        protocol: "list2cart".into(),
        device_id: admin.sync_identity().device_public_key,
        name: "Admin".into(),
        version: "0.1.0".into(),
        meta: BTreeMap::new(),
        sas: 2,
    };
    let out_b = joiner
        .finalize_pairing(
            &admin_hello,
            &admin_key_bytes,
            "123456",
            PeerRole::OwnDevice,
            "*",
            None,
            Some(&welcome_for_b),
        )
        .unwrap();
    assert!(out_b.paired && out_b.member);

    // Now Joiner B connects to Admin A presenting its `tls_credentials()` in the TLS 1.3 cert SAN.
    // Admin A verifies and atomically recovers Joiner B's membership!
    let joiner_creds = joiner.tls_credentials().unwrap();
    assert!(joiner_creds.is_some());
    assert!(
        admin
            .verify_or_recover_peer_access(&joiner_key_bytes, joiner_creds.as_deref())
            .unwrap(),
        "Admin must recover unilaterally finalized membership from TLS credentials"
    );

    // Subsequent sync_request -> sync_reply -> import_sync_page succeeds with 0 store conflicts!
    admin
        .create_shopping_list("Oporavljeni popis", None)
        .unwrap();
    let req = joiner
        .sync_request(&admin.sync_identity().device_public_key)
        .unwrap();
    let reply = admin.sync_reply(&req, 256 * 1024).unwrap();
    joiner.import_sync_page(&reply).unwrap();
    assert_eq!(joiner.list_shopping_lists().unwrap().len(), 1);
    assert!(joiner.identity_conflicts().unwrap().is_empty());

    // Case 2: Admin A finalized pairing, Joiner C lost connection before receiving WelcomePack, then re-pairs.
    let mut joiner_c = Database::open_in_memory().unwrap();
    let c_key_bytes = joiner_c
        .clone_device_signing_key()
        .verifying_key()
        .to_bytes();
    let c_hello = PairHello {
        protocol: "list2cart".into(),
        device_id: joiner_c.sync_identity().device_public_key,
        name: "Joiner C".into(),
        version: "0.1.0".into(),
        meta: BTreeMap::new(),
        sas: 2,
    };
    let welcome_c1 = admin
        .prepare_welcome_pack(&c_key_bytes, PeerRole::FamilyMember, "*", true)
        .unwrap();
    let mut admin = admin;
    admin
        .finalize_pairing(
            &c_hello,
            &c_key_bytes,
            "654321",
            PeerRole::FamilyMember,
            "*",
            Some(&welcome_c1),
            None,
        )
        .unwrap();

    // Re-pairing between Admin A and Joiner C reuses C's membership cleanly without conflict
    let welcome_c2 = admin
        .prepare_welcome_pack(&c_key_bytes, PeerRole::FamilyMember, "*", true)
        .unwrap();
    assert_eq!(
        welcome_c1.role_for_you.as_ref().unwrap().principal_id,
        welcome_c2.role_for_you.as_ref().unwrap().principal_id
    );
    let out_c = joiner_c
        .finalize_pairing(
            &admin_hello,
            &admin_key_bytes,
            "654321",
            PeerRole::FamilyMember,
            "*",
            None,
            Some(&welcome_c2),
        )
        .unwrap();
    assert!(out_c.paired && out_c.member);
    assert_eq!(
        joiner_c.sync_identity().network_public_key,
        admin.sync_identity().network_public_key
    );
    let _ = joiner_b64;
}
