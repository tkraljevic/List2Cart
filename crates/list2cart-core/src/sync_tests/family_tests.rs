use super::acl_tests::{control_version, grant, member, revocation, with_grant};
use super::*;
use crate::apply::{
    ExportRequest, ProtectedSyncPayload, export_protected_bundle_for_recipient,
    import_protected_bundle,
};

fn export(
    f: &Fixture,
    conn: &Connection,
    recipient: &SigningKey,
    frontier: &BTreeMap<String, String>,
    budget: usize,
) -> Result<
    (
        list2cart_sync::wire::bundle_v2::ProtectedBundleV2,
        ProtectedSyncPayload,
        usize,
    ),
    String,
> {
    let bundle = export_protected_bundle_for_recipient(
        conn,
        ExportRequest {
            local_node_id: "isolated relay",
            sender_sk: &f.root,
            network_pub: &f.network.public,
            recipient_pub: &recipient.verifying_key(),
            peer_frontier: frontier,
            requested_holes: &[],
            max_bytes: budget,
            now_unix: f.now,
        },
    )?;
    let (plaintext, _) = bundle
        .open(crate::apply::PROTOCOL_NAME, recipient)
        .map_err(|e| e.to_string())?;
    let payload = serde_json::from_slice(&plaintext).map_err(|e| e.to_string())?;
    Ok((bundle, payload, plaintext.len()))
}

fn setup(f: &Fixture, conn: &Connection, reader: &SigningKey, writer: &SigningKey) {
    let stranger = SigningKey::from_bytes(&[103; 32]);
    let controls = [
        member(f, reader, f.now - 20),
        grant(f, reader, "reader-a", "list-a", ListPermission::Read),
        member(f, writer, f.now - 20),
        grant(f, writer, "grant-a", "list-a", ListPermission::Write),
        member(f, &stranger, f.now - 20),
        grant(
            f,
            &stranger,
            "unrelated-secret-grant",
            "secret-list",
            ListPermission::Write,
        ),
        ControlPayload::Ovlast(
            f.network
                .issue_ovlast(
                    &"unrelated-secret-admin".repeat(1500),
                    &stranger.verifying_key(),
                    "root",
                    f.now - 20,
                    3600,
                )
                .unwrap(),
        ),
        revocation(f, &stranger, "list_grant", "unrelated-secret-grant"),
        grant(
            f,
            writer,
            "writer-unrelated",
            "secret-list",
            ListPermission::Write,
        ),
    ];
    for (i, ctrl) in controls.iter().enumerate() {
        let report = f
            .apply(
                conn,
                &[control_version(
                    f,
                    i as u128 + 1,
                    &format!("ctrl-{i}"),
                    ctrl,
                    &f.root,
                )],
            )
            .unwrap();
        assert!(report.rejected.is_empty(), "{:?}", report.rejected);
    }
    let private = f.version(
        41,
        "shopping_lists",
        "private-list",
        "private:family-secret",
        f.list(40, "private-list").payload,
    );
    let mut domains = vec![f.list(40, "secret-list"), private];
    for seq in 50..70 {
        domains.push(with_grant(f.list(seq, "list-a"), writer, "grant-a"));
    }
    let mut covered = BTreeMap::<String, crate::ledger::VersionRange>::new();
    for v in &domains {
        covered
            .entry(crate::ledger::frontier_key(&v.node_id, &v.channel))
            .and_modify(|range| range.to_id = range.to_id.clone().max(v.version_id.clone()))
            .or_insert(crate::ledger::VersionRange {
                from_id: String::new(),
                to_id: v.version_id.clone(),
            });
    }
    let report = apply_remote_versions(
        conn,
        ApplyContext {
            network_pub: &f.network.public,
            peer_key: &f.network.public_b64(),
            recipient_b64: Some(&f.network.public_b64()),
            now_unix: f.now,
        },
        &domains,
        &BTreeMap::new(),
        &covered,
    )
    .unwrap();
    assert!(report.rejected.is_empty(), "{:?}", report.rejected);
}

fn recovery_ctx<'a>(
    f: &'a Fixture,
    conn: &'a Connection,
    key: &'a SigningKey,
) -> crate::recovery::RecoveryContext<'a> {
    crate::recovery::RecoveryContext {
        conn,
        key,
        network: &f.network.public,
        principal: "family-principal",
        now: f.now,
    }
}

fn bootstrap_reader(f: &Fixture, conn: &Connection, reader: &SigningKey) {
    for ctrl in [
        member(f, reader, f.now - 20),
        grant(f, reader, "reader-a", "list-a", ListPermission::Read),
    ] {
        store_control_record(conn, &f.network.public, &ctrl, f.now).unwrap();
    }
}

#[test]
fn automatic_family_grant_expiry_replaces_partial_data_with_control_only_snapshot() {
    let source = Database::open_in_memory().unwrap();
    let destination = Database::open_in_memory().unwrap();
    let f = Fixture::new();
    let reader = SigningKey::from_bytes(&[127; 32]);
    let writer = SigningKey::from_bytes(&[128; 32]);
    setup(&f, source.test_connection(), &reader, &writer);
    bootstrap_reader(&f, destination.test_connection(), &reader);
    let long_member = ControlPayload::Membership(
        f.network
            .admit_member(
                "family",
                &reader.verifying_key(),
                MemberRole::FamilyMember,
                "family-principal",
                "root",
                Validity {
                    issued_at: f.now - 5,
                    valid_for_secs: 10000,
                },
            )
            .unwrap(),
    );
    f.apply(
        source.test_connection(),
        &[control_version(
            &f,
            100,
            "long-member",
            &long_member,
            &f.root,
        )],
    )
    .unwrap();
    store_control_record(
        destination.test_connection(),
        &f.network.public,
        &long_member,
        f.now,
    )
    .unwrap();
    let peer = verifying_key_to_base64(&reader.verifying_key());
    let expiry = f.now + 3590;
    assert!(
        crate::apply::allowed_channels_for_recipient(
            source.test_connection(),
            &f.network.public,
            &peer,
            expiry
        )
        .unwrap()
        .contains("list:list-a")
    );
    let request = crate::exchange::request(
        recovery_ctx(&f, destination.test_connection(), &reader),
        &f.network.public_b64(),
    )
    .unwrap();
    let page = crate::exchange::reply(
        recovery_ctx(&f, source.test_connection(), &f.root),
        &request,
        5000,
    )
    .unwrap();
    let outcome = crate::exchange::receive(
        recovery_ctx(&f, destination.test_connection(), &reader),
        &page,
    )
    .unwrap();
    assert!(
        matches!(outcome,crate::exchange::SyncReceiveOutcome::Snapshot {progress} if !progress.complete)
    );
    let now = expiry + 1;
    let late = Fixture {
        root: f.root.clone(),
        network: NetworkKey::from_signing_key("isolated", f.root.clone()),
        now,
        created: chrono::DateTime::from_timestamp(now, 0)
            .unwrap()
            .to_rfc3339(),
        base: f.base,
    };
    assert!(
        !crate::apply::allowed_channels_for_recipient(
            source.test_connection(),
            &f.network.public,
            &peer,
            now
        )
        .unwrap()
        .contains("list:list-a")
    );
    finish_automatic_family(
        &late,
        source.test_connection(),
        destination.test_connection(),
        &reader,
    );
    let count: i64 = destination
        .test_connection()
        .query_row(
            "SELECT COUNT(*) FROM record_versions WHERE channel='list:list-a'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        count, 0,
        "expired grant must not commit staged list versions"
    );
    assert!(
        crate::apply::allowed_channels_for_recipient(
            destination.test_connection(),
            &f.network.public,
            &peer,
            now
        )
        .unwrap()
        .contains("control")
    );
    let ended = Fixture {
        now: f.now + 10001,
        ..late
    };
    assert!(
        crate::exchange::request(
            recovery_ctx(&ended, destination.test_connection(), &reader),
            &f.network.public_b64()
        )
        .is_err(),
        "expired membership must not create a request"
    );
}

fn finish_automatic_family(
    f: &Fixture,
    source: &Connection,
    destination: &Connection,
    reader: &SigningKey,
) -> String {
    for _ in 0..100 {
        let request = crate::exchange::request(
            recovery_ctx(f, destination, reader),
            &f.network.public_b64(),
        )
        .unwrap();
        assert_eq!(request.format, 3);
        assert!(request.frontier.keys().all(|k| k.ends_with("|list:list-a")));
        let page =
            crate::exchange::reply(recovery_ctx(f, source, &f.root), &request, 5000).unwrap();
        match crate::exchange::receive(recovery_ctx(f, destination, reader), &page).unwrap() {
            crate::exchange::SyncReceiveOutcome::Snapshot { progress } => {
                if progress.complete {
                    return progress.snapshot_id;
                }
            }
            _ => panic!("expected family snapshot"),
        }
    }
    panic!("family transfer failed to terminate")
}

struct ReopenedDatabase {
    conn: Connection,
    path: std::path::PathBuf,
}
impl ReopenedDatabase {
    fn from_isolated(db: Database) -> Self {
        let path =
            std::env::temp_dir().join(format!("list2cart-auto-family-{}.sqlite", Uuid::new_v4()));
        db.test_connection()
            .execute("VACUUM INTO ?1", [path.to_str().unwrap()])
            .unwrap();
        drop(db);
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
        Self { conn, path }
    }
    fn test_connection(&self) -> &Connection {
        &self.conn
    }
}
impl Drop for ReopenedDatabase {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[test]
fn automatic_family_resumes_lost_reply_confirms_cleanup_and_returns_idle() {
    let source = Database::open_in_memory().unwrap();
    let destination = Database::open_in_memory().unwrap();
    let f = Fixture::new();
    let reader = SigningKey::from_bytes(&[120; 32]);
    let writer = SigningKey::from_bytes(&[121; 32]);
    setup(&f, source.test_connection(), &reader, &writer);
    bootstrap_reader(&f, destination.test_connection(), &reader);
    for i in 0..15 {
        let ctrl = grant(
            &f,
            &reader,
            &format!("large-proof-{i}"),
            "list-a",
            ListPermission::Read,
        );
        f.apply(
            source.test_connection(),
            &[control_version(
                &f,
                100 + i,
                &format!("large-proof-{i}"),
                &ctrl,
                &f.root,
            )],
        )
        .unwrap();
    }
    assert!(
        export(
            &f,
            source.test_connection(),
            &reader,
            &BTreeMap::new(),
            5000
        )
        .is_err()
    );
    let first = crate::exchange::request(
        recovery_ctx(&f, destination.test_connection(), &reader),
        &f.network.public_b64(),
    )
    .unwrap();
    let lost = crate::exchange::reply(
        recovery_ctx(&f, source.test_connection(), &f.root),
        &first,
        5000,
    )
    .unwrap();
    let retry = crate::exchange::reply(
        recovery_ctx(&f, source.test_connection(), &f.root),
        &first,
        5000,
    )
    .unwrap();
    let id = |page: &list2cart_sync::wire::bundle_v2::ProtectedBundleV2| {
        let (plain, _) = page.open(crate::apply::PROTOCOL_NAME, &reader).unwrap();
        serde_json::from_slice::<serde_json::Value>(&plain).unwrap()["snapshot_page"]["manifest"]["snapshot_id"].as_str().unwrap().to_owned()
    };
    assert_eq!(id(&lost), id(&retry));
    crate::exchange::receive(
        recovery_ctx(&f, destination.test_connection(), &reader),
        &retry,
    )
    .unwrap();
    let source = ReopenedDatabase::from_isolated(source);
    let destination = ReopenedDatabase::from_isolated(destination);
    let completed = finish_automatic_family(
        &f,
        source.test_connection(),
        destination.test_connection(),
        &reader,
    );
    assert_eq!(completed, id(&lost));
    let count = || {
        source
            .test_connection()
            .query_row(
                "SELECT COUNT(*) FROM sync_snapshot_items WHERE direction='out'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .unwrap()
    };
    assert!(count() > 0, "last exported page is not a signed receipt");
    let request = crate::exchange::request(
        recovery_ctx(&f, destination.test_connection(), &reader),
        &f.network.public_b64(),
    )
    .unwrap();
    let mut forged = request.clone();
    forged.recovery.as_mut().unwrap().next_offset = 0;
    assert!(
        crate::exchange::reply(
            recovery_ctx(&f, source.test_connection(), &f.root),
            &forged,
            5000
        )
        .is_err()
    );
    assert!(count() > 0);
    source.test_connection().execute_batch("CREATE TRIGGER family_ack_fault BEFORE DELETE ON sync_snapshot_items BEGIN SELECT RAISE(ABORT,'ack fault'); END").unwrap();
    assert!(
        crate::exchange::reply(
            recovery_ctx(&f, source.test_connection(), &f.root),
            &request,
            5000
        )
        .is_err()
    );
    assert!(count() > 0);
    assert!(
        !crate::recovery::load_session(
            source.test_connection(),
            "out",
            &completed,
            &verifying_key_to_base64(&reader.verifying_key())
        )
        .unwrap()
        .unwrap()
        .2
    );
    source
        .test_connection()
        .execute_batch("DROP TRIGGER family_ack_fault")
        .unwrap();
    for _ in 0..2 {
        let page = crate::exchange::reply(
            recovery_ctx(&f, source.test_connection(), &f.root),
            &request,
            5000,
        )
        .unwrap();
        match crate::exchange::receive(
            recovery_ctx(&f, destination.test_connection(), &reader),
            &page,
        )
        .unwrap()
        {
            crate::exchange::SyncReceiveOutcome::Delta { report, has_more } => {
                assert_eq!(report.applied_versions, 0);
                assert!(!has_more)
            }
            _ => panic!("unchanged family must become idle"),
        }
    }
    assert_eq!(count(), 0);
    // A late independent item below the already acknowledged MAX must recover.
    let late = with_grant(f.item(45, false), &writer, "grant-a");
    let report = f.apply(source.test_connection(), &[late]).unwrap();
    assert!(report.rejected.is_empty());
    let newer = finish_automatic_family(
        &f,
        source.test_connection(),
        destination.test_connection(),
        &reader,
    );
    assert_ne!(newer, completed);
    let items: i64 = destination
        .test_connection()
        .query_row(
            "SELECT COUNT(*) FROM planned_items WHERE sync_id='item-1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(items, 1);
}

#[test]
fn automatic_family_replaces_obsolete_proofs_and_delivers_regrant_after_revoke() {
    let source = Database::open_in_memory().unwrap();
    let destination = Database::open_in_memory().unwrap();
    let f = Fixture::new();
    let reader = SigningKey::from_bytes(&[122; 32]);
    let writer = SigningKey::from_bytes(&[123; 32]);
    setup(&f, source.test_connection(), &reader, &writer);
    bootstrap_reader(&f, destination.test_connection(), &reader);
    let req = crate::exchange::request(
        recovery_ctx(&f, destination.test_connection(), &reader),
        &f.network.public_b64(),
    )
    .unwrap();
    let first = crate::exchange::reply(
        recovery_ctx(&f, source.test_connection(), &f.root),
        &req,
        5000,
    )
    .unwrap();
    crate::exchange::receive(
        recovery_ctx(&f, destination.test_connection(), &reader),
        &first,
    )
    .unwrap();
    let revoke = revocation(&f, &reader, "list_grant", "reader-a");
    f.apply(
        source.test_connection(),
        &[control_version(&f, 100, "revoke", &revoke, &f.root)],
    )
    .unwrap();
    finish_automatic_family(
        &f,
        source.test_connection(),
        destination.test_connection(),
        &reader,
    );
    let public = verifying_key_to_base64(&reader.verifying_key());
    assert!(
        !crate::apply::allowed_channels_for_recipient(
            destination.test_connection(),
            &f.network.public,
            &public,
            f.now
        )
        .unwrap()
        .contains("list:list-a")
    );
    let restored = grant(
        &f,
        &reader,
        "new-read-grant",
        "list-a",
        ListPermission::Read,
    );
    f.apply(
        source.test_connection(),
        &[control_version(&f, 101, "new-grant", &restored, &f.root)],
    )
    .unwrap();
    finish_automatic_family(
        &f,
        source.test_connection(),
        destination.test_connection(),
        &reader,
    );
    assert!(
        crate::apply::allowed_channels_for_recipient(
            destination.test_connection(),
            &f.network.public,
            &public,
            f.now
        )
        .unwrap()
        .contains("list:list-a")
    );
}

#[test]
fn family_snapshot_splits_large_proofs_reopens_both_databases_and_commits_atomically() {
    let source = Database::open_in_memory().unwrap();
    let destination = Database::open_in_memory().unwrap();
    let f = Fixture::new();
    let reader = SigningKey::from_bytes(&[114; 32]);
    let writer = SigningKey::from_bytes(&[115; 32]);
    setup(&f, source.test_connection(), &reader, &writer);
    bootstrap_reader(&f, destination.test_connection(), &reader);
    // Many individually small relevant proofs exceed one small transport page.
    for i in 0..15 {
        let ctrl = grant(
            &f,
            &reader,
            &format!("reader-history-{i}"),
            "list-a",
            ListPermission::Read,
        );
        f.apply(
            source.test_connection(),
            &[control_version(
                &f,
                100 + i,
                &format!("history-{i}"),
                &ctrl,
                &f.root,
            )],
        )
        .unwrap();
    }
    assert!(
        export(
            &f,
            source.test_connection(),
            &reader,
            &BTreeMap::new(),
            5000
        )
        .is_err()
    );
    let peer = verifying_key_to_base64(&reader.verifying_key());
    let manifest =
        crate::recovery::start_family(recovery_ctx(&f, source.test_connection(), &f.root), &peer)
            .unwrap();
    assert_eq!(manifest.format, 2);
    assert!(!manifest.frontier.keys().any(|k| k.ends_with("|control")));
    assert!(!manifest.backfill_revisions.contains_key("control"));
    let first = crate::recovery::export_page(
        recovery_ctx(&f, source.test_connection(), &f.root),
        &manifest.snapshot_id,
        &peer,
        0,
        5000,
    )
    .unwrap();
    let first_progress = crate::recovery::import_page(
        recovery_ctx(&f, destination.test_connection(), &reader),
        &first,
    )
    .unwrap();
    assert!(!first_progress.complete);
    assert_eq!(
        crate::recovery::import_page(
            recovery_ctx(&f, destination.test_connection(), &reader),
            &first
        )
        .unwrap(),
        first_progress
    );
    let domain_count = |conn: &Connection| {
        conn.query_row(
            "SELECT COUNT(*) FROM record_versions WHERE channel='list:list-a'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
    };
    assert_eq!(domain_count(destination.test_connection()), 0);
    let dir = std::env::temp_dir().join(format!("list2cart-family-{}", Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(dir.clone());
    let src_path = dir.join("source.sqlite");
    let dst_path = dir.join("recipient.sqlite");
    source
        .test_connection()
        .execute("VACUUM INTO ?1", [src_path.to_str().unwrap()])
        .unwrap();
    destination
        .test_connection()
        .execute("VACUUM INTO ?1", [dst_path.to_str().unwrap()])
        .unwrap();
    drop(source);
    drop(destination);
    let source = Connection::open(&src_path).unwrap();
    let destination = Connection::open(&dst_path).unwrap();
    let mut progress =
        crate::recovery::progress(&destination, &f.network.public_b64(), &manifest.snapshot_id)
            .unwrap()
            .unwrap();
    assert_eq!(progress, first_progress);
    let mut pages = 1;
    loop {
        let page = crate::recovery::export_page(
            recovery_ctx(&f, &source, &f.root),
            &manifest.snapshot_id,
            &peer,
            progress.next_offset,
            5000,
        )
        .unwrap();
        let (plain, _) = page.open(crate::apply::PROTOCOL_NAME, &reader).unwrap();
        assert!(plain.len() <= 5000);
        let text = String::from_utf8(plain.clone()).unwrap();
        assert!(
            !text.contains("unrelated-secret")
                && !text.contains("secret-list")
                && !text.contains("family-secret")
        );
        let json: serde_json::Value = serde_json::from_slice(&plain).unwrap();
        if json["snapshot_page"]["done"] == true {
            // A final projection failure must not admit even the staged proofs.
            destination.execute_batch("CREATE TRIGGER family_final_fault BEFORE INSERT ON record_versions WHEN NEW.channel='list:list-a' BEGIN SELECT RAISE(ABORT,'final fault'); END").unwrap();
            assert!(
                crate::recovery::import_page(recovery_ctx(&f, &destination, &reader), &page)
                    .is_err()
            );
            assert_eq!(domain_count(&destination), 0);
            assert_eq!(
                crate::recovery::progress(
                    &destination,
                    &f.network.public_b64(),
                    &manifest.snapshot_id
                )
                .unwrap()
                .unwrap(),
                progress
            );
            destination
                .execute_batch("DROP TRIGGER family_final_fault")
                .unwrap();
        }
        progress =
            crate::recovery::import_page(recovery_ctx(&f, &destination, &reader), &page).unwrap();
        pages += 1;
        assert!(pages < 100);
        if progress.complete {
            break;
        }
        assert_eq!(domain_count(&destination), 0);
    }
    assert!(pages > 3);
    assert_eq!(domain_count(&destination), 20);
    let confirmed =
        crate::ledger::get_confirmed_frontier(&destination, &f.network.public_b64()).unwrap();
    assert!(
        confirmed
            .get(&crate::ledger::frontier_key(
                &f.network.public_b64(),
                "control"
            ))
            .is_none_or(String::is_empty)
    );
    let revisions = crate::backfill::acknowledged(&destination, &f.network.public_b64()).unwrap();
    assert_eq!(
        revisions.get("list:list-a"),
        manifest.backfill_revisions.get("list:list-a")
    );
}

#[test]
fn family_snapshot_revocation_blocks_frozen_output_and_staged_input() {
    let source = Database::open_in_memory().unwrap();
    let destination = Database::open_in_memory().unwrap();
    let f = Fixture::new();
    let reader = SigningKey::from_bytes(&[116; 32]);
    let writer = SigningKey::from_bytes(&[117; 32]);
    setup(&f, source.test_connection(), &reader, &writer);
    bootstrap_reader(&f, destination.test_connection(), &reader);
    let peer = verifying_key_to_base64(&reader.verifying_key());
    let manifest =
        crate::recovery::start_family(recovery_ctx(&f, source.test_connection(), &f.root), &peer)
            .unwrap();
    let page = crate::recovery::export_page(
        recovery_ctx(&f, source.test_connection(), &f.root),
        &manifest.snapshot_id,
        &peer,
        0,
        5000,
    )
    .unwrap();
    crate::recovery::import_page(
        recovery_ctx(&f, destination.test_connection(), &reader),
        &page,
    )
    .unwrap();
    let revoke = revocation(&f, &reader, "list_grant", "reader-a");
    store_control_record(
        destination.test_connection(),
        &f.network.public,
        &revoke,
        f.now,
    )
    .unwrap();
    assert!(
        crate::recovery::import_page(
            recovery_ctx(&f, destination.test_connection(), &reader),
            &page
        )
        .is_err()
    );
    f.apply(
        source.test_connection(),
        &[control_version(&f, 100, "revoked", &revoke, &f.root)],
    )
    .unwrap();
    assert!(
        crate::recovery::export_page(
            recovery_ctx(&f, source.test_connection(), &f.root),
            &manifest.snapshot_id,
            &peer,
            0,
            5000
        )
        .is_err()
    );
    // Regranting the same channel still cannot reuse its obsolete proof set.
    let restored = grant(
        &f,
        &reader,
        "reader-restored",
        "list-a",
        ListPermission::Read,
    );
    f.apply(
        source.test_connection(),
        &[control_version(&f, 101, "restored", &restored, &f.root)],
    )
    .unwrap();
    assert!(
        crate::recovery::export_page(
            recovery_ctx(&f, source.test_connection(), &f.root),
            &manifest.snapshot_id,
            &peer,
            0,
            5000
        )
        .unwrap_err()
        .contains("promijenjene")
    );
}

#[test]
fn family_snapshot_rejects_private_scope_global_ack_skip_and_changed_manifest() {
    let source = Database::open_in_memory().unwrap();
    let destination = Database::open_in_memory().unwrap();
    let f = Fixture::new();
    let reader = SigningKey::from_bytes(&[118; 32]);
    let writer = SigningKey::from_bytes(&[119; 32]);
    setup(&f, source.test_connection(), &reader, &writer);
    bootstrap_reader(&f, destination.test_connection(), &reader);
    let peer = verifying_key_to_base64(&reader.verifying_key());
    let manifest =
        crate::recovery::start_family(recovery_ctx(&f, source.test_connection(), &f.root), &peer)
            .unwrap();
    let bundle = crate::recovery::export_page(
        recovery_ctx(&f, source.test_connection(), &f.root),
        &manifest.snapshot_id,
        &peer,
        0,
        5000,
    )
    .unwrap();
    let (plain, _) = bundle.open(crate::apply::PROTOCOL_NAME, &reader).unwrap();
    let original: serde_json::Value = serde_json::from_slice(&plain).unwrap();
    let seal = |value: &serde_json::Value| {
        list2cart_sync::wire::bundle_v2::ProtectedBundleV2::seal(
            crate::apply::PROTOCOL_NAME,
            &f.root,
            reader.verifying_key().as_bytes(),
            &serde_json::to_vec(value).unwrap(),
        )
        .unwrap()
    };
    for kind in 0..4 {
        let mut value = original.clone();
        match kind {
            0 => value["snapshot_page"]["manifest"]["channels"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!("private:secret")),
            1 => {
                value["snapshot_page"]["manifest"]["frontier"]
                    [crate::ledger::frontier_key(&f.network.public_b64(), "control")] =
                    serde_json::json!(manifest.snapshot_id);
            }
            2 => value["snapshot_page"]["offset"] = serde_json::json!(1),
            _ => value["snapshot_page"]["manifest"]["format"] = serde_json::json!(1),
        }
        assert!(
            crate::recovery::import_page(
                recovery_ctx(&f, destination.test_connection(), &reader),
                &seal(&value)
            )
            .is_err()
        );
    }
    assert!(
        crate::recovery::progress(
            destination.test_connection(),
            &f.network.public_b64(),
            &manifest.snapshot_id
        )
        .unwrap()
        .is_none()
    );
    let progress = crate::recovery::import_page(
        recovery_ctx(&f, destination.test_connection(), &reader),
        &bundle,
    )
    .unwrap();
    let mut changed = original;
    changed["snapshot_page"]["manifest"]["proof_sha256"] = serde_json::json!("0".repeat(64));
    assert!(
        crate::recovery::import_page(
            recovery_ctx(&f, destination.test_connection(), &reader),
            &seal(&changed)
        )
        .is_err()
    );
    assert_eq!(
        crate::recovery::progress(
            destination.test_connection(),
            &f.network.public_b64(),
            &manifest.snapshot_id
        )
        .unwrap()
        .unwrap(),
        progress
    );
}

#[test]
fn family_pages_replay_author_proofs_without_leaking_or_acknowledging_other_controls() {
    let source = Database::open_in_memory().unwrap();
    let destination = Database::open_in_memory().unwrap();
    let f = Fixture::new();
    let reader = SigningKey::from_bytes(&[101; 32]);
    let writer = SigningKey::from_bytes(&[102; 32]);
    setup(&f, source.test_connection(), &reader, &writer);
    let (_, full, full_len) = export(
        &f,
        source.test_connection(),
        &reader,
        &BTreeMap::new(),
        1024 * 1024,
    )
    .unwrap();
    let controls: Vec<_> = full
        .batch
        .versions
        .iter()
        .filter(|v| v.channel == "control")
        .cloned()
        .collect();
    assert_eq!(controls.len(), 4);
    // Leave room for a small number of domain versions, forcing continuation.
    let proof_bytes = crate::ledger::serialized_json_len(&controls).unwrap();
    let budget = proof_bytes + 3500;
    assert!(budget < full_len);
    let mut frontier = BTreeMap::new();
    let mut pages = 0;
    loop {
        let (bundle, payload, length) =
            export(&f, source.test_connection(), &reader, &frontier, budget).unwrap();
        assert!(length <= budget);
        let text = serde_json::to_string(&payload).unwrap();
        for secret in [
            "unrelated-secret",
            "secret-list",
            "writer-unrelated",
            "family-secret",
        ] {
            assert!(!text.contains(secret), "{secret}");
        }
        assert_eq!(
            payload
                .batch
                .versions
                .iter()
                .filter(|v| v.channel == "control")
                .count(),
            4
        );
        assert!(
            payload
                .batch
                .covered_ranges
                .keys()
                .chain(payload.batch.sender_holes.keys())
                .all(|key| !key.ends_with("|control"))
        );
        assert!(
            payload
                .batch
                .versions
                .iter()
                .any(|v| v.channel == "list:list-a")
        );
        let report = import_protected_bundle(
            destination.test_connection(),
            &reader,
            &f.network.public,
            "family-principal",
            &bundle,
            f.now,
        )
        .unwrap();
        assert!(report.rejected.is_empty(), "{:?}", report.rejected);
        let replay = import_protected_bundle(
            destination.test_connection(),
            &reader,
            &f.network.public,
            "family-principal",
            &bundle,
            f.now,
        )
        .unwrap();
        assert!(replay.rejected.is_empty());
        assert_eq!(replay.applied_versions, 0);
        let next = crate::ledger::get_confirmed_frontier(
            destination.test_connection(),
            &f.network.public_b64(),
        )
        .unwrap();
        assert_ne!(next, frontier, "a page must make domain progress");
        frontier = next;
        pages += 1;
        assert!(pages < 30);
        if !payload.batch.has_more {
            break;
        }
    }
    assert!(pages > 1);
    let count: i64 = destination
        .test_connection()
        .query_row(
            "SELECT COUNT(*) FROM record_versions WHERE channel='list:list-a'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 20);
    let (_, empty, _) = export(&f, source.test_connection(), &reader, &frontier, budget).unwrap();
    assert!(!empty.batch.has_more);
    assert!(empty.batch.versions.iter().all(|v| v.channel == "control"));
    assert!(empty.batch.covered_ranges.is_empty());
    assert!(
        export(
            &f,
            source.test_connection(),
            &reader,
            &BTreeMap::new(),
            proof_bytes
        )
        .is_err()
    );
}

#[test]
fn family_export_refuses_cache_only_authority_instead_of_sending_unverifiable_data() {
    let source = Database::open_in_memory().unwrap();
    let f = Fixture::new();
    let reader = SigningKey::from_bytes(&[104; 32]);
    for ctrl in [
        member(&f, &reader, f.now - 20),
        grant(&f, &reader, "read-a", "list-a", ListPermission::Read),
    ] {
        store_control_record(source.test_connection(), &f.network.public, &ctrl, f.now).unwrap();
    }
    f.apply(source.test_connection(), &[f.list(30, "list-a")])
        .unwrap();
    assert!(
        export(
            &f,
            source.test_connection(),
            &reader,
            &BTreeMap::new(),
            1024 * 1024
        )
        .unwrap_err()
        .contains("dokaz")
    );
}

#[test]
fn family_revocation_and_new_grant_are_replayed_even_above_old_control_frontier() {
    let source = Database::open_in_memory().unwrap();
    let destination = Database::open_in_memory().unwrap();
    let f = Fixture::new();
    let reader = SigningKey::from_bytes(&[105; 32]);
    let writer = SigningKey::from_bytes(&[106; 32]);
    let public = verifying_key_to_base64(&reader.verifying_key());
    setup(&f, source.test_connection(), &reader, &writer);
    let (initial, _, _) = export(
        &f,
        source.test_connection(),
        &reader,
        &BTreeMap::new(),
        1024 * 1024,
    )
    .unwrap();
    assert!(
        import_protected_bundle(
            destination.test_connection(),
            &reader,
            &f.network.public,
            "family-principal",
            &initial,
            f.now
        )
        .unwrap()
        .rejected
        .is_empty()
    );
    let mut frontier = crate::ledger::get_confirmed_frontier(
        destination.test_connection(),
        &f.network.public_b64(),
    )
    .unwrap();
    frontier.insert(
        crate::ledger::frontier_key(&f.network.public_b64(), "control"),
        "ffffffff-ffff-7fff-bfff-ffffffffffff".into(),
    );
    let revoked = revocation(&f, &reader, "list_grant", "reader-a");
    assert!(
        f.apply(
            source.test_connection(),
            &[control_version(&f, 100, "revoke-reader", &revoked, &f.root)]
        )
        .unwrap()
        .rejected
        .is_empty()
    );
    let (revoked_page, payload, _) = export(
        &f,
        source.test_connection(),
        &reader,
        &frontier,
        1024 * 1024,
    )
    .unwrap();
    assert!(
        payload
            .batch
            .versions
            .iter()
            .all(|v| v.channel == "control")
    );
    assert!(
        payload
            .batch
            .versions
            .iter()
            .any(|v| v.payload.contains("revocation"))
    );
    assert!(
        import_protected_bundle(
            destination.test_connection(),
            &reader,
            &f.network.public,
            "family-principal",
            &revoked_page,
            f.now
        )
        .unwrap()
        .rejected
        .is_empty()
    );
    assert!(
        !crate::apply::allowed_channels_for_recipient(
            destination.test_connection(),
            &f.network.public,
            &public,
            f.now
        )
        .unwrap()
        .contains("list:list-a")
    );
    let restored = grant(
        &f,
        &reader,
        "reader-restored",
        "list-a",
        ListPermission::Read,
    );
    f.apply(
        source.test_connection(),
        &[control_version(
            &f,
            101,
            "reader-restored",
            &restored,
            &f.root,
        )],
    )
    .unwrap();
    let (restored_page, payload, _) = export(
        &f,
        source.test_connection(),
        &reader,
        &frontier,
        1024 * 1024,
    )
    .unwrap();
    assert!(
        payload
            .batch
            .versions
            .iter()
            .any(|v| v.payload.contains("reader-restored"))
    );
    assert!(
        import_protected_bundle(
            destination.test_connection(),
            &reader,
            &f.network.public,
            "family-principal",
            &restored_page,
            f.now
        )
        .unwrap()
        .rejected
        .is_empty()
    );
    assert!(
        crate::apply::allowed_channels_for_recipient(
            destination.test_connection(),
            &f.network.public,
            &public,
            f.now
        )
        .unwrap()
        .contains("list:list-a")
    );
    let bad_write = with_grant(f.list(110, "list-a"), &reader, "reader-restored");
    assert!(
        crate::apply::verify_domain_version_auth(
            destination.test_connection(),
            &f.network.public,
            &bad_write,
            f.now
        )
        .is_err()
    );
}

#[test]
fn family_delegation_proof_precedes_envelopes_even_with_later_wrapper_uuid() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use ed25519_dalek::Signer;
    use list2cart_sync::trust::{ListGrant, Membership};
    let source = Database::open_in_memory().unwrap();
    let destination = Database::open_in_memory().unwrap();
    let f = Fixture::new();
    let reader = SigningKey::from_bytes(&[107; 32]);
    let writer = SigningKey::from_bytes(&[108; 32]);
    let admin = SigningKey::from_bytes(&[109; 32]);
    let authority = f
        .network
        .issue_ovlast(
            "relevant-admin",
            &admin.verifying_key(),
            "root",
            f.now - 30,
            3600,
        )
        .unwrap();
    let member = ControlPayload::Membership(
        Membership::admit_as(
            &admin,
            authority.clone(),
            "writer",
            &writer.verifying_key(),
            MemberRole::FamilyMember,
            "writer-principal",
            Validity {
                issued_at: f.now - 20,
                valid_for_secs: 1000,
            },
        )
        .unwrap(),
    );
    let mut permission = ListGrant {
        network: f.network.public_b64(),
        grant_id: "grant-a".into(),
        list_sync_id: "list-a".into(),
        recipient_pub_key: verifying_key_to_base64(&writer.verifying_key()),
        permission: ListPermission::Write,
        issued_by: "relevant-admin".into(),
        issued_at: f.now - 20,
        expires_at: f.now + 900,
        signature: String::new(),
        admitter: Some(authority.clone()),
    };
    permission.signature = STANDARD.encode(admin.sign(&permission.signed_bytes()).to_bytes());
    let report = f
        .apply(
            source.test_connection(),
            &[
                control_version(&f, 30, "admin", &ControlPayload::Ovlast(authority), &f.root),
                control_version(
                    &f,
                    1,
                    "reader",
                    &super::acl_tests::member(&f, &reader, f.now - 20),
                    &f.root,
                ),
                control_version(
                    &f,
                    2,
                    "reader-grant",
                    &grant(&f, &reader, "reader-a", "list-a", ListPermission::Read),
                    &f.root,
                ),
                control_version(&f, 3, "writer", &member, &admin),
                control_version(
                    &f,
                    4,
                    "writer-grant",
                    &ControlPayload::ListGrant(permission),
                    &admin,
                ),
                with_grant(f.list(40, "list-a"), &writer, "grant-a"),
            ],
        )
        .unwrap();
    assert!(report.rejected.is_empty(), "{:?}", report.rejected);
    let (bundle, payload, _) = export(
        &f,
        source.test_connection(),
        &reader,
        &BTreeMap::new(),
        1024 * 1024,
    )
    .unwrap();
    assert_eq!(
        payload
            .batch
            .versions
            .iter()
            .filter(|v| v.channel == "control")
            .count(),
        5
    );
    let report = import_protected_bundle(
        destination.test_connection(),
        &reader,
        &f.network.public,
        "family-principal",
        &bundle,
        f.now,
    )
    .unwrap();
    assert!(report.rejected.is_empty(), "{:?}", report.rejected);
    assert_eq!(report.projected_records, 1);
}

#[test]
fn family_foreign_own_device_does_not_receive_private_data_or_global_control_ack() {
    let source = Database::open_in_memory().unwrap();
    let destination = Database::open_in_memory().unwrap();
    let f = Fixture::new();
    let reader = SigningKey::from_bytes(&[110; 32]);
    let writer = SigningKey::from_bytes(&[111; 32]);
    setup(&f, source.test_connection(), &reader, &writer);
    let membership = ControlPayload::Membership(
        f.network
            .admit_member(
                "reader",
                &reader.verifying_key(),
                MemberRole::OwnDevice,
                "family-principal",
                "root",
                Validity {
                    issued_at: f.now - 5,
                    valid_for_secs: 3600,
                },
            )
            .unwrap(),
    );
    f.apply(
        source.test_connection(),
        &[
            control_version(&f, 100, "reader-renewal", &membership, &f.root),
            f.version(
                101,
                "shopping_lists",
                "foreign-private",
                "private:family-principal",
                f.list(101, "foreign-private").payload,
            ),
        ],
    )
    .unwrap();
    let (_, mut payload, _) = export(
        &f,
        source.test_connection(),
        &reader,
        &BTreeMap::new(),
        1024 * 1024,
    )
    .unwrap();
    assert!(
        payload
            .batch
            .versions
            .iter()
            .all(|v| !v.channel.starts_with("private:"))
    );
    assert!(
        !serde_json::to_string(&payload)
            .unwrap()
            .contains("unrelated-secret")
    );
    let endpoint = payload
        .batch
        .versions
        .iter()
        .filter(|v| v.channel == "control" && v.node_id == f.network.public_b64())
        .map(|v| v.version_id.clone())
        .max()
        .unwrap();
    let control_key = crate::ledger::frontier_key(&f.network.public_b64(), "control");
    payload.batch.covered_ranges.insert(
        control_key.clone(),
        crate::ledger::VersionRange {
            from_id: String::new(),
            to_id: endpoint,
        },
    );
    let bundle = list2cart_sync::wire::bundle_v2::ProtectedBundleV2::seal(
        crate::apply::PROTOCOL_NAME,
        &f.root,
        reader.verifying_key().as_bytes(),
        &serde_json::to_vec(&payload).unwrap(),
    )
    .unwrap();
    let report = import_protected_bundle(
        destination.test_connection(),
        &reader,
        &f.network.public,
        "family-principal",
        &bundle,
        f.now,
    )
    .unwrap();
    assert!(report.rejected.is_empty(), "{:?}", report.rejected);
    let frontier = crate::ledger::get_confirmed_frontier(
        destination.test_connection(),
        &f.network.public_b64(),
    )
    .unwrap();
    assert!(
        frontier.get(&control_key).is_none_or(String::is_empty),
        "a signed family packet cannot acknowledge the global control stream"
    );
}

#[test]
fn family_packet_failure_rolls_back_proofs_data_and_progress_then_retries() {
    let source = Database::open_in_memory().unwrap();
    let destination = Database::open_in_memory().unwrap();
    let f = Fixture::new();
    let reader = SigningKey::from_bytes(&[112; 32]);
    let writer = SigningKey::from_bytes(&[113; 32]);
    setup(&f, source.test_connection(), &reader, &writer);
    let (bundle, _, _) = export(
        &f,
        source.test_connection(),
        &reader,
        &BTreeMap::new(),
        1024 * 1024,
    )
    .unwrap();
    let conn = destination.test_connection();
    let count = || {
        conn.query_row("SELECT COUNT(*) FROM record_versions", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap()
    };
    let before = count();
    conn.execute_batch("CREATE TRIGGER family_fault BEFORE INSERT ON record_versions WHEN NEW.channel='list:list-a' BEGIN SELECT RAISE(ABORT,'family fault'); END;").unwrap();
    assert!(
        import_protected_bundle(
            conn,
            &reader,
            &f.network.public,
            "family-principal",
            &bundle,
            f.now
        )
        .is_err()
    );
    assert_eq!(count(), before);
    assert!(
        !crate::apply::allowed_channels_for_recipient(
            conn,
            &f.network.public,
            &verifying_key_to_base64(&reader.verifying_key()),
            f.now
        )
        .unwrap()
        .contains("list:list-a")
    );
    conn.execute_batch("DROP TRIGGER family_fault").unwrap();
    let report = import_protected_bundle(
        conn,
        &reader,
        &f.network.public,
        "family-principal",
        &bundle,
        f.now,
    )
    .unwrap();
    assert!(report.rejected.is_empty(), "{:?}", report.rejected);
    assert_eq!(report.projected_records, 20);
}
