use super::acl_tests::{control_version, grant, member, revocation, with_grant};
use super::*;

#[test]
fn missing_author_proofs_can_arrive_in_either_order_and_replay_completes() {
    for member_first in [true, false] {
        let db = Database::open_in_memory().unwrap();
        let conn = db.test_connection();
        let f = Fixture::new();
        let writer = SigningKey::from_bytes(&[132; 32]);
        let membership = member(&f, &writer, f.now - 20);
        let permission = grant(&f, &writer, "grant-a", "list-a", ListPermission::Write);
        let child = with_grant(f.item(2, false), &writer, "grant-a");
        assert_eq!(
            f.apply(conn, std::slice::from_ref(&child))
                .unwrap()
                .rejected
                .len(),
            1
        );
        let controls = if member_first {
            [membership, permission]
        } else {
            [permission, membership]
        };
        let first = control_version(&f, 10, "proof-one", &controls[0], &f.root);
        assert_eq!(
            f.apply(conn, &[child.clone(), first])
                .unwrap()
                .rejected
                .len(),
            1
        );
        let second = control_version(&f, 11, "proof-two", &controls[1], &f.root);
        let accepted = f.apply(conn, &[child.clone(), second]).unwrap();
        assert!(accepted.rejected.is_empty(), "{:?}", accepted.rejected);
        assert_eq!(accepted.deferred_children, 1);
        let parent = f.apply(conn, &[f.list(1, "list-a")]).unwrap();
        assert!(parent.blocked_projections.is_empty());
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM planned_items WHERE sync_id='item-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
        let covered = BTreeMap::from([(
            crate::ledger::frontier_key(&child.node_id, &child.channel),
            crate::ledger::VersionRange {
                from_id: String::new(),
                to_id: child.version_id.clone(),
            },
        )]);
        let report = apply_remote_versions(
            conn,
            ApplyContext {
                network_pub: &f.network.public,
                peer_key: &f.network.public_b64(),
                recipient_b64: Some(&f.network.public_b64()),
                now_unix: f.now,
            },
            std::slice::from_ref(&child),
            &BTreeMap::new(),
            &covered,
        )
        .unwrap();
        assert!(report.rejected.is_empty());
        assert_eq!(report.applied_versions, 0);
        let frontier =
            crate::ledger::get_confirmed_frontier(conn, &f.network.public_b64()).unwrap();
        assert_eq!(
            frontier.get(&crate::ledger::frontier_key(&child.node_id, &child.channel)),
            Some(&child.version_id)
        );
    }
}

#[test]
fn revocation_in_same_packet_precedes_historical_domain_in_any_input_order() {
    for kind in ["list_grant", "clanstvo"] {
        for control_first in [true, false] {
            let db = Database::open_in_memory().unwrap();
            let conn = db.test_connection();
            let f = Fixture::new();
            let writer = SigningKey::from_bytes(&[133; 32]);
            for proof in [
                member(&f, &writer, f.now - 20),
                grant(&f, &writer, "grant-a", "list-a", ListPermission::Write),
            ] {
                store_control_record(conn, &f.network.public, &proof, f.now).unwrap();
            }
            f.apply(conn, &[f.list(1, "list-a")]).unwrap();
            let child = with_grant(f.item(2, false), &writer, "grant-a");
            let revoke = control_version(
                &f,
                20,
                "revoked",
                &revocation(&f, &writer, kind, "grant-a"),
                &f.root,
            );
            let batch = if control_first {
                vec![revoke, child.clone()]
            } else {
                vec![child.clone(), revoke]
            };
            let report = f.apply(conn, &batch).unwrap();
            assert_eq!(report.applied_versions, 1);
            assert_eq!(report.rejected.len(), 1);
            assert_eq!(report.rejected[0].version_id, child.version_id);
            assert_eq!(
                f.apply(conn, &[child]).unwrap().rejected.len(),
                1,
                "replay cannot restore revoked author rights"
            );
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM planned_items WHERE sync_id='item-1'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(count, 0);
        }
    }
}

#[test]
fn snapshot_and_family_recovery_succeed_after_author_and_admin_revocations_and_settle_idle() {
    use crate::acl_types::ListGrant;
    use base64::{Engine, engine::general_purpose::STANDARD};
    use ed25519_dalek::Signer;
    use syncnet::trust::Membership;

    let source = Database::open_in_memory().unwrap();
    let own_dest = Database::open_in_memory().unwrap();
    let fam_dest = Database::open_in_memory().unwrap();
    let f = Fixture::new();

    let own_peer_key = SigningKey::from_bytes(&[140; 32]);
    let reader = SigningKey::from_bytes(&[141; 32]);
    let revoked_writer = SigningKey::from_bytes(&[142; 32]);
    let revoked_admin = SigningKey::from_bytes(&[143; 32]);
    let delegated_writer = SigningKey::from_bytes(&[144; 32]);
    let shortened_admin = SigningKey::from_bytes(&[145; 32]);
    let valid_writer = SigningKey::from_bytes(&[146; 32]);

    let root_principal = syncnet::identity::principal_id_from_key(f.network.public.as_bytes());
    let own_membership = ControlPayload::Membership(
        f.network
            .admit_member(
                "own-peer",
                &own_peer_key.verifying_key(),
                "root",
                Validity {
                    issued_at: f.now - 100,
                    valid_for_secs: 3600,
                },
            )
            .unwrap(),
    );
    let own_role = ControlPayload::MemberRole(
        issue_member_role(
            &f.network,
            "own-peer",
            &own_peer_key.verifying_key(),
            MemberRole::OwnDevice,
            &root_principal,
            "root",
            Validity {
                issued_at: f.now - 100,
                valid_for_secs: 3600,
            },
        )
        .unwrap(),
    );
    let own_list_grant = grant(&f, &own_peer_key, "own-a", "list-a", ListPermission::Read);
    let root_own_membership = ControlPayload::Membership(
        f.network
            .admit_member(
                "root-own",
                &f.root.verifying_key(),
                "root",
                Validity {
                    issued_at: f.now - 100,
                    valid_for_secs: 3600,
                },
            )
            .unwrap(),
    );
    let root_own_role = ControlPayload::MemberRole(
        issue_member_role(
            &f.network,
            "root-own",
            &f.root.verifying_key(),
            MemberRole::OwnDevice,
            &root_principal,
            "root",
            Validity {
                issued_at: f.now - 100,
                valid_for_secs: 3600,
            },
        )
        .unwrap(),
    );
    let reader_member = member(&f, &reader, f.now - 100);
    let reader_grant = grant(&f, &reader, "reader-a", "list-a", ListPermission::Read);

    let w1_member = member(&f, &revoked_writer, f.now - 100);
    let w1_grant = grant(
        &f,
        &revoked_writer,
        "grant-w1",
        "list-a",
        ListPermission::Write,
    );

    let admin1_ovlast = f
        .network
        .issue_ovlast(
            "admin1",
            &revoked_admin.verifying_key(),
            "root",
            f.now - 100,
            3600,
        )
        .unwrap();
    let w2_member = ControlPayload::Membership(
        Membership::admit_as(
            &revoked_admin,
            admin1_ovlast.clone(),
            "w2",
            &delegated_writer.verifying_key(),
            Validity {
                issued_at: f.now - 90,
                valid_for_secs: 1000,
            },
        )
        .unwrap(),
    );
    let mut w2_grant_inner = ListGrant {
        network: f.network.public_b64(),
        grant_id: "grant-w2".into(),
        list_sync_id: "list-a".into(),
        recipient_pub_key: verifying_key_to_base64(&delegated_writer.verifying_key()),
        permission: ListPermission::Write,
        issued_by: "admin1".into(),
        issued_at: f.now - 90,
        expires_at: f.now + 900,
        signature: String::new(),
        admitter: Some(admin1_ovlast.clone()),
    };
    w2_grant_inner.signature = STANDARD.encode(
        revoked_admin
            .sign(&w2_grant_inner.signed_bytes())
            .to_bytes(),
    );
    let w2_grant = ControlPayload::ListGrant(w2_grant_inner);

    let admin2_old = f
        .network
        .issue_ovlast(
            "admin2",
            &shortened_admin.verifying_key(),
            "root",
            f.now - 100,
            2000,
        )
        .unwrap();
    let admin2_short = f
        .network
        .issue_ovlast(
            "admin2",
            &shortened_admin.verifying_key(),
            "root",
            f.now - 40,
            20,
        )
        .unwrap();
    let w3_member = ControlPayload::Membership(
        Membership::admit_as(
            &shortened_admin,
            admin2_old.clone(),
            "w3",
            &valid_writer.verifying_key(),
            Validity {
                issued_at: f.now - 80,
                valid_for_secs: 1000,
            },
        )
        .unwrap(),
    );
    let mut w3_valid_grant_inner = ListGrant {
        network: f.network.public_b64(),
        grant_id: "grant-w3-valid".into(),
        list_sync_id: "list-a".into(),
        recipient_pub_key: verifying_key_to_base64(&valid_writer.verifying_key()),
        permission: ListPermission::Write,
        issued_by: "admin2".into(),
        issued_at: f.now - 80,
        expires_at: f.now + 900,
        signature: String::new(),
        admitter: Some(admin2_old.clone()),
    };
    w3_valid_grant_inner.signature = STANDARD.encode(
        shortened_admin
            .sign(&w3_valid_grant_inner.signed_bytes())
            .to_bytes(),
    );
    let w3_valid_grant = ControlPayload::ListGrant(w3_valid_grant_inner);

    // Stale grant issued at f.now - 30 (after admin2_short at f.now - 40), embedding admin2_old.
    let mut w3_stale_grant_inner = ListGrant {
        network: f.network.public_b64(),
        grant_id: "grant-w3-stale".into(),
        list_sync_id: "list-a".into(),
        recipient_pub_key: verifying_key_to_base64(&valid_writer.verifying_key()),
        permission: ListPermission::Write,
        issued_by: "admin2".into(),
        issued_at: f.now - 30,
        expires_at: f.now + 900,
        signature: String::new(),
        admitter: Some(admin2_old.clone()),
    };
    w3_stale_grant_inner.signature = STANDARD.encode(
        shortened_admin
            .sign(&w3_stale_grant_inner.signed_bytes())
            .to_bytes(),
    );
    let w3_stale_grant = ControlPayload::ListGrant(w3_stale_grant_inner);

    let f_at_80 = Fixture {
        root: f.root.clone(),
        network: NetworkKey::from_signing_key("isolated test", f.root.clone()),
        now: f.now - 80,
        created: chrono::DateTime::from_timestamp(f.now - 80, 0)
            .unwrap()
            .to_rfc3339(),
        base: f.base - ((80u128 * 1000) << 80),
    };
    let f_at_30 = Fixture {
        root: f.root.clone(),
        network: NetworkKey::from_signing_key("isolated test", f.root.clone()),
        now: f.now - 30,
        created: chrono::DateTime::from_timestamp(f.now - 30, 0)
            .unwrap()
            .to_rfc3339(),
        base: f.base - ((30u128 * 1000) << 80),
    };

    // Admit initial controls and writes on source BEFORE revocations and shortened ovlast arrive.
    let initial_controls = [
        control_version(&f, 1, "own-m", &own_membership, &f.root),
        control_version(&f, 18, "own-r", &own_role, &f.root),
        control_version(&f, 2, "own-g", &own_list_grant, &f.root),
        control_version(&f, 3, "root-m", &root_own_membership, &f.root),
        control_version(&f, 19, "root-r", &root_own_role, &f.root),
        control_version(&f, 4, "reader-m", &reader_member, &f.root),
        control_version(&f, 5, "reader-g", &reader_grant, &f.root),
        control_version(&f, 6, "w1-m", &w1_member, &f.root),
        control_version(&f, 7, "w1-g", &w1_grant, &f.root),
        control_version(
            &f,
            8,
            "admin1-o",
            &ControlPayload::Ovlast(admin1_ovlast),
            &f.root,
        ),
        control_version(&f, 9, "w2-m", &w2_member, &revoked_admin),
        control_version(&f, 10, "w2-g", &w2_grant, &revoked_admin),
        control_version(
            &f_at_80,
            11,
            "admin2-old",
            &ControlPayload::Ovlast(admin2_old),
            &f.root,
        ),
        control_version(&f_at_80, 12, "w3-m", &w3_member, &shortened_admin),
        control_version(
            &f_at_80,
            13,
            "w3-g-valid",
            &w3_valid_grant,
            &shortened_admin,
        ),
        control_version(
            &f_at_30,
            14,
            "w3-g-stale",
            &w3_stale_grant,
            &shortened_admin,
        ),
    ];
    let admit_complete = |conn: &Connection, rows: &[Version]| {
        let mut covered = BTreeMap::<String, crate::ledger::VersionRange>::new();
        for v in rows {
            covered
                .entry(crate::ledger::frontier_key(&v.node_id, &v.channel))
                .and_modify(|range| range.to_id = range.to_id.clone().max(v.version_id.clone()))
                .or_insert(crate::ledger::VersionRange {
                    from_id: String::new(),
                    to_id: v.version_id.clone(),
                });
        }
        apply_remote_versions(
            conn,
            ApplyContext {
                network_pub: &f.network.public,
                peer_key: &f.network.public_b64(),
                recipient_b64: Some(&f.network.public_b64()),
                now_unix: f.now,
            },
            rows,
            &BTreeMap::new(),
            &covered,
        )
        .unwrap()
    };

    assert!(
        admit_complete(source.test_connection(), &initial_controls)
            .rejected
            .is_empty()
    );

    let make_item = |seq: u128, id: &str, list_id: &str, name: &str, supersedes: &str| {
        let mut v = f.version(
            seq,
            "planned_items",
            id,
            &format!("list:{list_id}"),
            serde_json::to_string(&PlannedItemSyncPayload {
                sync_id: id.into(),
                list_sync_id: list_id.into(),
                barcode: None,
                name: name.into(),
                qty_millis: 1000,
                unit: Unit::Kom,
                estimated_price_cents: 100,
                checked: false,
                updated_at: f.created.clone(),
                archived: false,
            })
            .unwrap(),
        );
        v.supersedes = supersedes.into();
        let mut env: SignedVersionEnvelope =
            serde_json::from_value(serde_json::to_value(v).unwrap()).unwrap();
        env = env.sign(PROTOCOL_NAME, &f.root);
        serde_json::from_value(serde_json::to_value(env).unwrap()).unwrap()
    };

    let list_a = f.list(20, "list-a");
    let item_w1_only = with_grant(
        make_item(21, "item-w1", "list-a", "Od opozvanog pisca", ""),
        &revoked_writer,
        "grant-w1",
    );
    let item_shared_v1 = with_grant(
        make_item(22, "item-shared", "list-a", "Prva verzija opozvanog", ""),
        &revoked_writer,
        "grant-w1",
    );
    let item_shared_v2 = make_item(
        23,
        "item-shared",
        "list-a",
        "Valjana nadogradnja",
        &item_shared_v1.version_id,
    );
    let item_w2 = with_grant(
        make_item(24, "item-w2", "list-a", "Od opozvanog admina", ""),
        &delegated_writer,
        "grant-w2",
    );
    let item_w3_valid = with_grant(
        make_item(
            25,
            "item-w3-valid",
            "list-a",
            "Valjana stara delegacija",
            "",
        ),
        &valid_writer,
        "grant-w3-valid",
    );
    let item_w3_stale = with_grant(
        make_item(26, "item-w3-stale", "list-a", "Zastarjela delegacija", ""),
        &valid_writer,
        "grant-w3-stale",
    );
    // Unprojected orphan child whose parent list never arrived.
    let item_orphan = make_item(27, "item-orphan", "missing-list", "Siroče", "");

    let domains = [
        list_a,
        item_w1_only,
        item_shared_v1,
        item_shared_v2,
        item_w2,
        item_w3_valid,
        item_w3_stale,
        item_orphan,
    ];
    assert!(
        admit_complete(source.test_connection(), &domains)
            .rejected
            .is_empty()
    );

    // Record a compaction floor for the later-revoked writer to ensure it never causes an endless recovery loop.
    let w1_stream = crate::ledger::frontier_key(
        &verifying_key_to_base64(&revoked_writer.verifying_key()),
        "list:list-a",
    );
    source
        .test_connection()
        .execute(
            "INSERT INTO settings(key,value) VALUES(?1,?2)",
            params![
                format!("compaction_floor:{w1_stream}"),
                Uuid::from_u128(f.base + 22).to_string()
            ],
        )
        .unwrap();

    // Now apply revocations and the shortened Ovlast on source.
    let rev_w1 = revocation(&f, &revoked_writer, "list_grant", "grant-w1");
    let rev_admin1 = revocation(&f, &revoked_admin, "ovlast", "admin1");
    assert!(
        admit_complete(
            source.test_connection(),
            &[
                control_version(&f, 30, "rev-w1", &rev_w1, &f.root),
                control_version(&f, 31, "rev-admin1", &rev_admin1, &f.root),
                control_version(
                    &f,
                    32,
                    "admin2-short",
                    &ControlPayload::Ovlast(admin2_short),
                    &f.root
                ),
            ]
        )
        .rejected
        .is_empty()
    );

    // Also verify that a rejected version from a revoked author does not block snapshot creation with a phantom hole.
    let post_revoke_attempt = with_grant(
        make_item(40, "item-post-revoke", "list-a", "Odbijeno", ""),
        &revoked_writer,
        "grant-w1",
    );
    assert_eq!(
        f.apply(source.test_connection(), &[post_revoke_attempt])
            .unwrap()
            .rejected
            .len(),
        1
    );

    // Bootstrap destination nodes with their own membership and reader grant.
    for ctrl in [
        &own_membership,
        &own_role,
        &own_list_grant,
        &root_own_membership,
        &root_own_role,
    ] {
        store_control_record(own_dest.test_connection(), &f.network.public, ctrl, f.now).unwrap();
    }
    for ctrl in [&reader_member, &reader_grant] {
        store_control_record(fam_dest.test_connection(), &f.network.public, ctrl, f.now).unwrap();
    }

    fn ctx<'a>(
        f: &'a Fixture,
        conn: &'a Connection,
        key: &'a SigningKey,
        principal: &'a str,
    ) -> crate::recovery::RecoveryContext<'a> {
        crate::recovery::RecoveryContext {
            conn,
            key,
            network: &f.network.public,
            principal,
            now: f.now,
        }
    }

    // 1. Automatic family snapshot exchange (format 3 request -> format 2 snapshot -> completion ACK -> idle delta).
    for _ in 0..20 {
        let req = crate::exchange::request(
            ctx(&f, fam_dest.test_connection(), &reader, "family-principal"),
            &f.network.public_b64(),
        )
        .unwrap();
        let reply = crate::exchange::reply(
            ctx(&f, source.test_connection(), &f.root, &root_principal),
            &req,
            4096,
        )
        .unwrap();
        match crate::exchange::receive(
            ctx(&f, fam_dest.test_connection(), &reader, "family-principal"),
            &reply,
        )
        .unwrap()
        {
            crate::exchange::SyncReceiveOutcome::Snapshot { progress } => {
                if progress.complete {
                    break;
                }
            }
            other => panic!("expected family snapshot, got {other:?}"),
        }
    }
    // Next exchange confirms completion and settles to an idle Delta with 0 versions.
    let ack_req = crate::exchange::request(
        ctx(&f, fam_dest.test_connection(), &reader, "family-principal"),
        &f.network.public_b64(),
    )
    .unwrap();
    let idle_reply = crate::exchange::reply(
        ctx(&f, source.test_connection(), &f.root, &root_principal),
        &ack_req,
        4096,
    )
    .unwrap();
    match crate::exchange::receive(
        ctx(&f, fam_dest.test_connection(), &reader, "family-principal"),
        &idle_reply,
    )
    .unwrap()
    {
        crate::exchange::SyncReceiveOutcome::Delta { report, has_more } => {
            assert!(report.rejected.is_empty());
            assert_eq!(report.applied_versions, 0);
            assert!(!has_more);
        }
        other => panic!("expected idle delta after family snapshot ACK, got {other:?}"),
    }

    // 2. Own-device snapshot recovery (format 1) + completion ACK -> idle delta.
    let own_peer_b64 = verifying_key_to_base64(&own_peer_key.verifying_key());
    let manifest = crate::recovery::start(
        ctx(&f, source.test_connection(), &f.root, &root_principal),
        &own_peer_b64,
    )
    .unwrap();
    let page = crate::recovery::export_page(
        ctx(&f, source.test_connection(), &f.root, &root_principal),
        &manifest.snapshot_id,
        &own_peer_b64,
        0,
        1024 * 1024,
    )
    .unwrap();
    let progress = crate::recovery::import_page(
        ctx(
            &f,
            own_dest.test_connection(),
            &own_peer_key,
            &root_principal,
        ),
        &page,
    )
    .unwrap();
    assert!(progress.complete);

    for dest in [&fam_dest, &own_dest] {
        let lists = dest.list_shopping_lists().unwrap();
        assert_eq!(lists.len(), 1);
        let names: Vec<_> = lists[0].items.iter().map(|i| i.name.as_str()).collect();
        assert!(names.contains(&"Valjana nadogradnja"), "{names:?}");
        assert!(names.contains(&"Valjana stara delegacija"), "{names:?}");
        assert!(!names.contains(&"Od opozvanog pisca"), "{names:?}");
        assert!(!names.contains(&"Od opozvanog admina"), "{names:?}");
        assert!(!names.contains(&"Zastarjela delegacija"), "{names:?}");
        assert!(!names.contains(&"Siroče"), "{names:?}");
    }
}

#[test]
fn package_json_import_accepts_v2_and_v1_and_rejects_unknown_formats_atomically() {
    let source = Database::open_in_memory().unwrap();
    let store = source
        .add_store("V2 Paket Dućan", "Zagreb", "#1768D1")
        .unwrap();
    let list = source
        .create_shopping_list("V2 Popis", Some(store.id))
        .unwrap();
    source
        .add_planned_item(list.id, None, "Mlijeko", 2.0, Unit::L, 150)
        .unwrap();

    // Test unsupported format versions are rejected without changing database state.
    let dest = Database::open_in_memory().unwrap();
    let initial_stores = dest.list_stores().unwrap().len();
    assert!(
        dest.import_sync_package_json(r#"{"format_version":99}"#)
            .is_err()
    );
    assert!(dest.import_sync_package_json(r#"{"version":99}"#).is_err());
    assert_eq!(dest.list_stores().unwrap().len(), initial_stores);

    // Test legacy v1 JSON package import via import_sync_package_json.
    let legacy = source
        .export_sync_bundle_filtered(crate::ledger::PeerRole::FamilyMember, Some(&list.sync_id))
        .unwrap();
    let legacy_json = serde_json::to_string(&legacy).unwrap();
    let (_, _, imported_lists) = dest.import_sync_package_json(&legacy_json).unwrap();
    assert_eq!(imported_lists, 1);
    assert_eq!(dest.list_shopping_lists().unwrap().len(), 1);
}
