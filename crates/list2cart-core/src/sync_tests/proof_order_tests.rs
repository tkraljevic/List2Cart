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
