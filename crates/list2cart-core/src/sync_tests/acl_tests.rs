use super::*;
use crate::apply::allowed_channels_for_recipient;
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::Signer;
use syncnet::trust::{ListGrant, Membership, Revocation};

pub(super) fn member(f: &Fixture, key: &SigningKey, issued: i64) -> ControlPayload {
    ControlPayload::Membership(
        f.network
            .admit_member(
                "family",
                &key.verifying_key(),
                MemberRole::FamilyMember,
                "family-principal",
                "root",
                Validity {
                    issued_at: issued,
                    valid_for_secs: 3600,
                },
            )
            .unwrap(),
    )
}
pub(super) fn grant(
    f: &Fixture,
    key: &SigningKey,
    id: &str,
    list: &str,
    permission: ListPermission,
) -> ControlPayload {
    ControlPayload::ListGrant(
        f.network
            .issue_list_grant(
                id,
                list,
                &key.verifying_key(),
                permission,
                "root",
                Validity {
                    issued_at: f.now - 10,
                    valid_for_secs: 3600,
                },
            )
            .unwrap(),
    )
}
pub(super) fn revocation(
    f: &Fixture,
    key: &SigningKey,
    kind: &str,
    target: &str,
) -> ControlPayload {
    ControlPayload::Revocation(
        Revocation {
            id: String::new(),
            kind: kind.into(),
            target_id: target.into(),
            public_key: verifying_key_to_base64(&key.verifying_key()),
            issued_at: f.now,
            revoked_at: f.now,
            revoked_by: "root".into(),
            signer_key: String::new(),
            signature: String::new(),
        }
        .sign(&f.root),
    )
}
pub(super) fn with_grant(v: Version, key: &SigningKey, grant: &str) -> Version {
    let mut env: SignedVersionEnvelope =
        serde_json::from_value(serde_json::to_value(v).unwrap()).unwrap();
    env.grant_ref = grant.into();
    env.node_id = verifying_key_to_base64(&key.verifying_key());
    serde_json::from_value(serde_json::to_value(env.sign(key)).unwrap()).unwrap()
}

fn earlier(f: &Fixture, seconds: i64) -> Fixture {
    Fixture {
        root: f.root.clone(),
        network: NetworkKey::from_signing_key("isolated test", f.root.clone()),
        now: f.now - seconds,
        created: chrono::DateTime::from_timestamp(f.now - seconds, 0)
            .unwrap()
            .to_rfc3339(),
        base: f.base - ((seconds as u128 * 1000) << 80),
    }
}

pub(super) fn control_version(
    f: &Fixture,
    seq: u128,
    id: &str,
    ctrl: &ControlPayload,
    signer: &SigningKey,
) -> Version {
    f.signed(
        seq,
        "control",
        id,
        "control",
        serde_json::to_string(ctrl).unwrap(),
        signer,
    )
}

#[test]
fn historical_membership_uses_latest_issuance_without_privilege_fallback() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.test_connection();
    let f = Fixture::new();
    let old = earlier(&f, 100);
    let key = SigningKey::from_bytes(&[94; 32]);
    let public = verifying_key_to_base64(&key.verifying_key());
    let make_member = |role, issued, lifetime| {
        ControlPayload::Membership(
            f.network
                .admit_member(
                    "family",
                    &key.verifying_key(),
                    role,
                    "family-principal",
                    "root",
                    Validity {
                        issued_at: issued,
                        valid_for_secs: lifetime,
                    },
                )
                .unwrap(),
        )
    };
    let owner = make_member(MemberRole::OwnDevice, old.now - 10, 1000);
    let downgrade = make_member(MemberRole::FamilyMember, old.now + 10, 20);
    let renewal = make_member(MemberRole::FamilyMember, f.now - 10, 1000);
    for (clock, seq, payload) in [(&old, 1, &owner), (&old, 2, &downgrade), (&f, 1, &renewal)] {
        assert!(
            f.apply(
                conn,
                &[control_version(clock, seq, "member", payload, &f.root)]
            )
            .unwrap()
            .rejected
            .is_empty()
        );
    }
    let historic = crate::acl::historical_membership(conn, &f.network.public, &public, old.now)
        .unwrap()
        .unwrap();
    assert_eq!(historic.role, MemberRole::OwnDevice);
    assert_eq!(
        crate::acl::historical_membership(conn, &f.network.public, &public, old.now + 15)
            .unwrap()
            .unwrap()
            .role,
        MemberRole::FamilyMember
    );
    assert!(
        crate::acl::historical_membership(conn, &f.network.public, &public, old.now + 40)
            .unwrap()
            .is_none(),
        "expired newer certificate must not revive old owner rights"
    );
    let ambiguous = make_member(MemberRole::FamilyMember, old.now - 10, 1000);
    assert!(
        store_control_record(conn, &f.network.public, &ambiguous, f.now).is_err(),
        "a newer cache entry must not hide conflicting historical issuance"
    );
    // Every control proof survives normal compaction.
    crate::ledger::auto_compact(
        conn,
        crate::ledger::CompactionPolicy {
            retention_days: 1,
            max_versions_per_record: 1,
        },
    )
    .unwrap();
    assert_eq!(
        crate::acl::historical_membership(conn, &f.network.public, &public, old.now)
            .unwrap()
            .unwrap()
            .role,
        MemberRole::OwnDevice
    );
}

#[test]
fn expired_history_validates_old_writes_without_restoring_current_access() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.test_connection();
    let f = Fixture::new();
    let old = earlier(&f, 100);
    let key = SigningKey::from_bytes(&[91; 32]);
    let membership = ControlPayload::Membership(
        old.network
            .admit_member(
                "family",
                &key.verifying_key(),
                MemberRole::FamilyMember,
                "family-principal",
                "root",
                Validity {
                    issued_at: old.now - 10,
                    valid_for_secs: 30,
                },
            )
            .unwrap(),
    );
    let permission = ControlPayload::ListGrant(
        old.network
            .issue_list_grant(
                "grant-a",
                "list-a",
                &key.verifying_key(),
                ListPermission::Write,
                "root",
                Validity {
                    issued_at: old.now - 10,
                    valid_for_secs: 30,
                },
            )
            .unwrap(),
    );
    let renewal = member(&f, &key, f.now - 10);
    // Newer effective membership is deliberately admitted before the old proof.
    f.apply(conn, &[control_version(&f, 1, "member", &renewal, &f.root)])
        .unwrap();
    let mut list = old.list(4, "list-a");
    list = with_grant(list, &key, "grant-a");
    let report = f
        .apply(
            conn,
            &[
                list.clone(),
                control_version(&old, 1, "member", &membership, &f.root),
                control_version(&old, 2, "grant-a", &permission, &f.root),
            ],
        )
        .unwrap();
    assert!(report.rejected.is_empty(), "{:?}", report.rejected);
    assert_eq!(report.applied_versions, 3);
    let allowed = allowed_channels_for_recipient(
        conn,
        &f.network.public,
        &verifying_key_to_base64(&key.verifying_key()),
        f.now,
    )
    .unwrap();
    assert!(
        !allowed.contains("list:list-a"),
        "historical grant must not grant current access"
    );
    assert_eq!(
        f.apply(conn, &[with_grant(f.list(10, "list-a"), &key, "grant-a")])
            .unwrap()
            .rejected
            .len(),
        1
    );
    // Revocation remains conservative even for versions signed before it.
    store_control_record(
        conn,
        &f.network.public,
        &revocation(&f, &key, "list_grant", "grant-a"),
        f.now,
    )
    .unwrap();
    assert!(
        crate::apply::verify_domain_version_auth(conn, &f.network.public, &list, f.now).is_err()
    );
}

#[test]
fn expired_delegation_proves_historical_control_but_not_new_issuance() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.test_connection();
    let f = Fixture::new();
    let old = earlier(&f, 100);
    let admin = SigningKey::from_bytes(&[92; 32]);
    let key = SigningKey::from_bytes(&[93; 32]);
    let authority = f
        .network
        .issue_ovlast("admin", &admin.verifying_key(), "root", old.now - 10, 40)
        .unwrap();
    let membership = ControlPayload::Membership(
        Membership::admit_as(
            &admin,
            authority.clone(),
            "family",
            &key.verifying_key(),
            MemberRole::FamilyMember,
            "family-principal",
            Validity {
                issued_at: old.now,
                valid_for_secs: 20,
            },
        )
        .unwrap(),
    );
    let mut permission = ListGrant {
        network: f.network.public_b64(),
        grant_id: "grant-a".into(),
        list_sync_id: "list-a".into(),
        recipient_pub_key: verifying_key_to_base64(&key.verifying_key()),
        permission: ListPermission::Write,
        issued_by: "admin".into(),
        issued_at: old.now,
        expires_at: old.now + 20,
        signature: String::new(),
        admitter: Some(authority.clone()),
    };
    permission.signature = STANDARD.encode(admin.sign(&permission.signed_bytes()).to_bytes());
    let report = f
        .apply(
            conn,
            &[
                control_version(
                    &old,
                    1,
                    "admin",
                    &ControlPayload::Ovlast(authority),
                    &f.root,
                ),
                control_version(&old, 2, "member", &membership, &admin),
                control_version(
                    &old,
                    3,
                    "grant-a",
                    &ControlPayload::ListGrant(permission.clone()),
                    &admin,
                ),
                with_grant(old.list(4, "list-a"), &key, "grant-a"),
            ],
        )
        .unwrap();
    assert!(report.rejected.is_empty(), "{:?}", report.rejected);
    assert_eq!(report.applied_versions, 4);
    permission.grant_id = "too-late".into();
    permission.issued_at = f.now;
    permission.expires_at = f.now + 20;
    permission.signature = STANDARD.encode(admin.sign(&permission.signed_bytes()).to_bytes());
    assert!(
        store_control_record(
            conn,
            &f.network.public,
            &ControlPayload::ListGrant(permission),
            f.now
        )
        .is_err()
    );
    store_control_record(
        conn,
        &f.network.public,
        &revocation(&f, &admin, "ovlast", "admin"),
        f.now,
    )
    .unwrap();
    assert!(
        crate::apply::verify_domain_version_auth(
            conn,
            &f.network.public,
            &with_grant(old.list(5, "list-a"), &key, "grant-a"),
            f.now
        )
        .is_err()
    );
}

#[test]
fn stable_control_ids_cannot_rebind_or_replay_old_memberships() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.test_connection();
    let f = Fixture::new();
    let key = SigningKey::from_bytes(&[82; 32]);
    let old = member(&f, &key, f.now - 20);
    let new = member(&f, &key, f.now - 10);
    for ctrl in [&old, &new, &old] {
        store_control_record(conn, &f.network.public, ctrl, f.now).unwrap();
    }
    let issued: i64 = conn
        .query_row(
            "SELECT issued_at FROM sync_trust_records WHERE kind='membership' AND subject_key=?1",
            [verifying_key_to_base64(&key.verifying_key())],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(issued, f.now - 10);
    let rebound = ControlPayload::Membership(
        f.network
            .admit_member(
                "family",
                &key.verifying_key(),
                MemberRole::OwnDevice,
                "other-principal",
                "root",
                Validity {
                    issued_at: f.now,
                    valid_for_secs: 3600,
                },
            )
            .unwrap(),
    );
    assert!(store_control_record(conn, &f.network.public, &rebound, f.now).is_err());
    let first = grant(&f, &key, "stable-grant", "list-a", ListPermission::Read);
    store_control_record(conn, &f.network.public, &first, f.now).unwrap();
    for bad in [
        grant(&f, &key, "stable-grant", "list-b", ListPermission::Read),
        grant(&f, &key, "stable-grant", "list-a", ListPermission::Write),
        grant(
            &f,
            &key,
            &format!(
                "membership:{}",
                verifying_key_to_base64(&key.verifying_key())
            ),
            "list-a",
            ListPermission::Write,
        ),
    ] {
        assert!(store_control_record(conn, &f.network.public, &bad, f.now).is_err());
    }
    store_control_record(conn, &f.network.public, &first, f.now).unwrap();
    let allowed = allowed_channels_for_recipient(
        conn,
        &f.network.public,
        &verifying_key_to_base64(&key.verifying_key()),
        f.now - 30,
    )
    .unwrap();
    assert!(
        allowed.is_empty(),
        "a rolled-back clock must not activate not-yet-issued rights"
    );
}

#[test]
fn read_only_and_key_revocations_are_enforced_in_either_arrival_order() {
    for revoke_first in [false, true] {
        let db = Database::open_in_memory().unwrap();
        let conn = db.test_connection();
        let f = Fixture::new();
        let key = SigningKey::from_bytes(&[83; 32]);
        let public = verifying_key_to_base64(&key.verifying_key());
        store_control_record(
            conn,
            &f.network.public,
            &member(&f, &key, f.now - 20),
            f.now,
        )
        .unwrap();
        let read = grant(&f, &key, "grant-a", "list-a", ListPermission::Read);
        store_control_record(conn, &f.network.public, &read, f.now).unwrap();
        let attempt = f.signed(
            1,
            "shopping_lists",
            "list-a",
            "list:list-a",
            f.list(1, "list-a").payload,
            &key,
        );
        assert_eq!(f.apply(conn, &[attempt]).unwrap().rejected.len(), 1);
        let write = grant(&f, &key, "write-a", "list-a", ListPermission::Write);
        let revoke = revocation(&f, &key, "list_grant", "write-a");
        for ctrl in if revoke_first {
            [&revoke, &write]
        } else {
            [&write, &revoke]
        } {
            store_control_record(conn, &f.network.public, ctrl, f.now).unwrap();
        }
        let attempt = with_grant(
            f.signed(
                2,
                "shopping_lists",
                "list-a",
                "list:list-a",
                f.list(2, "list-a").payload,
                &key,
            ),
            &key,
            "write-a",
        );
        assert_eq!(f.apply(conn, &[attempt]).unwrap().rejected.len(), 1);
        assert!(
            allowed_channels_for_recipient(conn, &f.network.public, &public, f.now)
                .unwrap()
                .contains("list:list-a"),
            "the independent read grant remains active"
        );
        let wrong_target = revocation(
            &f,
            &SigningKey::from_bytes(&[84; 32]),
            "list_grant",
            "grant-a",
        );
        assert!(store_control_record(conn, &f.network.public, &wrong_target, f.now).is_err());
        let full = revocation(&f, &key, "clanstvo", "family");
        store_control_record(conn, &f.network.public, &full, f.now).unwrap();
        assert!(
            allowed_channels_for_recipient(conn, &f.network.public, &public, f.now)
                .unwrap()
                .is_empty()
        );
        store_control_record(
            conn,
            &f.network.public,
            &member(&f, &key, f.now + 1),
            f.now + 2,
        )
        .unwrap();
        assert!(
            allowed_channels_for_recipient(conn, &f.network.public, &public, f.now + 2)
                .unwrap()
                .is_empty(),
            "a revoked device key cannot re-admit itself through membership replay or renewal"
        );
    }
}

#[test]
fn revoked_pending_author_cannot_project_when_parent_arrives_later() {
    for kind in ["list_grant", "clanstvo"] {
        let db = Database::open_in_memory().unwrap();
        let conn = db.test_connection();
        let f = Fixture::new();
        let writer = SigningKey::from_bytes(&[131; 32]);
        for proof in [
            member(&f, &writer, f.now - 20),
            grant(&f, &writer, "grant-a", "list-a", ListPermission::Write),
        ] {
            store_control_record(conn, &f.network.public, &proof, f.now).unwrap();
        }
        let child = with_grant(f.item(2, false), &writer, "grant-a");
        let report = f.apply(conn, std::slice::from_ref(&child)).unwrap();
        assert_eq!(report.deferred_children, 1);
        store_control_record(
            conn,
            &f.network.public,
            &revocation(&f, &writer, kind, "grant-a"),
            f.now,
        )
        .unwrap();
        let report = f.apply(conn, &[f.list(1, "list-a")]).unwrap();
        assert!(
            report.rejected.is_empty(),
            "the valid parent must still be admitted"
        );
        assert_eq!(report.blocked_projections.len(), 1);
        assert_eq!(report.blocked_projections[0].version_id, child.version_id);
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM planned_items WHERE sync_id='item-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            count, 0,
            "{kind}: a delayed parent must not project a revoked author's queued child"
        );
        let kept: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM record_versions WHERE version_id=?1",
                [&child.version_id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(kept, 1, "signed evidence must be preserved");
    }
}

#[test]
fn superseded_admin_authority_cannot_issue_new_certificates_via_old_embedded_proof() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.test_connection();
    let f = Fixture::new();
    let admin = SigningKey::from_bytes(&[129; 32]);
    let reader = SigningKey::from_bytes(&[130; 32]);
    let old = f
        .network
        .issue_ovlast("admin", &admin.verifying_key(), "root", f.now - 100, 2000)
        .unwrap();
    let short = f
        .network
        .issue_ovlast("admin", &admin.verifying_key(), "root", f.now - 30, 20)
        .unwrap();
    store_control_record(
        conn,
        &f.network.public,
        &ControlPayload::Ovlast(old.clone()),
        f.now,
    )
    .unwrap();
    let mut permission = ListGrant {
        network: f.network.public_b64(),
        grant_id: "stale-admin-grant".into(),
        list_sync_id: "list-a".into(),
        recipient_pub_key: verifying_key_to_base64(&reader.verifying_key()),
        permission: ListPermission::Write,
        issued_by: "admin".into(),
        issued_at: f.now - 5,
        expires_at: f.now + 100,
        signature: String::new(),
        admitter: Some(old.clone()),
    };
    permission.signature = STANDARD.encode(admin.sign(&permission.signed_bytes()).to_bytes());
    store_control_record(
        conn,
        &f.network.public,
        &member(&f, &reader, f.now - 60),
        f.now,
    )
    .unwrap();
    store_control_record(
        conn,
        &f.network.public,
        &ControlPayload::ListGrant(permission.clone()),
        f.now,
    )
    .unwrap();
    store_control_record(
        conn,
        &f.network.public,
        &ControlPayload::Ovlast(short),
        f.now,
    )
    .unwrap();
    let public = verifying_key_to_base64(&reader.verifying_key());
    assert!(
        !allowed_channels_for_recipient(conn, &f.network.public, &public, f.now)
            .unwrap()
            .contains("list:list-a")
    );
    assert!(
        crate::apply::verify_domain_version_auth(
            conn,
            &f.network.public,
            &with_grant(f.list(200, "list-a"), &reader, "stale-admin-grant"),
            f.now
        )
        .is_err()
    );
    assert!(
        store_control_record(
            conn,
            &f.network.public,
            &ControlPayload::ListGrant(permission),
            f.now
        )
        .is_err(),
        "old long authority cannot bypass newer shortened authority"
    );
    let bad_member = Membership::admit_as(
        &admin,
        old.clone(),
        "reader",
        &reader.verifying_key(),
        MemberRole::FamilyMember,
        "family-principal",
        Validity {
            issued_at: f.now - 5,
            valid_for_secs: 100,
        },
    )
    .unwrap();
    assert!(
        store_control_record(
            conn,
            &f.network.public,
            &ControlPayload::Membership(bad_member),
            f.now
        )
        .is_err()
    );
    let historical = Membership::admit_as(
        &admin,
        old,
        "reader",
        &reader.verifying_key(),
        MemberRole::FamilyMember,
        "family-principal",
        Validity {
            issued_at: f.now - 50,
            valid_for_secs: 100,
        },
    )
    .unwrap();
    store_control_record(
        conn,
        &f.network.public,
        &ControlPayload::Membership(historical),
        f.now,
    )
    .unwrap();
}

#[test]
fn revoking_admin_device_key_closes_delegated_read_access_even_with_stale_flags() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.test_connection();
    let f = Fixture::new();
    let admin = SigningKey::from_bytes(&[124; 32]);
    let reader = SigningKey::from_bytes(&[125; 32]);
    let authority = f
        .network
        .issue_ovlast("admin", &admin.verifying_key(), "root", f.now - 100, 3600)
        .unwrap();
    store_control_record(
        conn,
        &f.network.public,
        &ControlPayload::Ovlast(authority.clone()),
        f.now,
    )
    .unwrap();
    store_control_record(
        conn,
        &f.network.public,
        &member(&f, &reader, f.now - 50),
        f.now,
    )
    .unwrap();
    let mut delegated = ListGrant {
        network: f.network.public_b64(),
        grant_id: "delegated-read".into(),
        list_sync_id: "list-a".into(),
        recipient_pub_key: verifying_key_to_base64(&reader.verifying_key()),
        permission: ListPermission::Read,
        issued_by: "admin".into(),
        issued_at: f.now - 40,
        expires_at: f.now + 200,
        signature: String::new(),
        admitter: Some(authority.clone()),
    };
    delegated.signature = STANDARD.encode(admin.sign(&delegated.signed_bytes()).to_bytes());
    store_control_record(
        conn,
        &f.network.public,
        &ControlPayload::ListGrant(delegated),
        f.now,
    )
    .unwrap();
    let public = verifying_key_to_base64(&reader.verifying_key());
    assert!(
        allowed_channels_for_recipient(conn, &f.network.public, &public, f.now)
            .unwrap()
            .contains("list:list-a")
    );
    store_control_record(
        conn,
        &f.network.public,
        &revocation(&f, &admin, "clanstvo", "admin"),
        f.now,
    )
    .unwrap();
    assert!(
        !allowed_channels_for_recipient(conn, &f.network.public, &public, f.now)
            .unwrap()
            .contains("list:list-a"),
        "revoking administrator device must revoke its issued read grant"
    );
    conn.execute(
        "UPDATE sync_trust_records SET revoked=0 WHERE kind!='revocation'",
        [],
    )
    .unwrap();
    assert!(
        !allowed_channels_for_recipient(conn, &f.network.public, &public, f.now)
            .unwrap()
            .contains("list:list-a"),
        "old persisted flags must not bypass the issuer revocation"
    );
    let delegated_member = Membership::admit_as(
        &admin,
        authority,
        "own",
        &SigningKey::from_bytes(&[126; 32]).verifying_key(),
        MemberRole::OwnDevice,
        "family-principal",
        Validity {
            issued_at: f.now - 30,
            valid_for_secs: 100,
        },
    )
    .unwrap();
    // Simulate a cache written before that administrator was revoked.
    conn.execute("INSERT INTO sync_trust_records(record_id,kind,subject_key,target_id,permission,issued_at,expires_at,revoked,raw_json) VALUES(?1,'membership',?2,'family-principal','own_device',?3,?4,0,?5)",params![format!("membership:{}",delegated_member.device_key),delegated_member.device_key,delegated_member.issued_at,delegated_member.expires_at,serde_json::to_string(&ControlPayload::Membership(delegated_member.clone())).unwrap()]).unwrap();
    assert!(
        allowed_channels_for_recipient(
            conn,
            &f.network.public,
            &delegated_member.device_key,
            f.now
        )
        .unwrap()
        .is_empty()
    );
    assert!(
        !crate::apply::sender_is_own_principal(
            conn,
            &f.network.public,
            &delegated_member.device_key,
            "family-principal",
            f.now
        )
        .unwrap()
    );
}

#[test]
fn revoking_delegation_blocks_embedded_old_authority_and_its_issued_grants() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.test_connection();
    let f = Fixture::new();
    let admin = SigningKey::from_bytes(&[85; 32]);
    let family = SigningKey::from_bytes(&[86; 32]);
    let authority = f
        .network
        .issue_ovlast("admin", &admin.verifying_key(), "root", f.now - 100, 3600)
        .unwrap();
    store_control_record(
        conn,
        &f.network.public,
        &ControlPayload::Ovlast(authority.clone()),
        f.now,
    )
    .unwrap();
    let delegated = Membership::admit_as(
        &admin,
        authority.clone(),
        "family",
        &family.verifying_key(),
        MemberRole::FamilyMember,
        "family-principal",
        Validity {
            issued_at: f.now - 50,
            valid_for_secs: 300,
        },
    )
    .unwrap();
    store_control_record(
        conn,
        &f.network.public,
        &ControlPayload::Membership(delegated),
        f.now,
    )
    .unwrap();
    let make_grant = |id: &str| {
        let mut g = ListGrant {
            network: f.network.public_b64(),
            grant_id: id.into(),
            list_sync_id: "list-a".into(),
            recipient_pub_key: verifying_key_to_base64(&family.verifying_key()),
            permission: ListPermission::Write,
            issued_by: "admin".into(),
            issued_at: f.now - 40,
            expires_at: f.now + 200,
            signature: String::new(),
            admitter: Some(authority.clone()),
        };
        g.signature = STANDARD.encode(admin.sign(&g.signed_bytes()).to_bytes());
        ControlPayload::ListGrant(g)
    };
    let issued = make_grant("delegated-grant");
    store_control_record(conn, &f.network.public, &issued, f.now).unwrap();
    let ControlPayload::Revocation(anchor) = revocation(&f, &f.root, "clanstvo", "root") else {
        unreachable!()
    };
    assert!(
        store_control_record(
            conn,
            &f.network.public,
            &ControlPayload::Revocation(anchor.sign(&admin)),
            f.now
        )
        .is_err(),
        "delegation cannot revoke the network trust anchor as a device"
    );
    store_control_record(
        conn,
        &f.network.public,
        &revocation(&f, &admin, "ovlast", "admin"),
        f.now,
    )
    .unwrap();
    assert!(
        allowed_channels_for_recipient(
            conn,
            &f.network.public,
            &verifying_key_to_base64(&family.verifying_key()),
            f.now
        )
        .unwrap()
        .is_empty()
    );
    assert!(
        store_control_record(
            conn,
            &f.network.public,
            &make_grant("new-after-revocation"),
            f.now
        )
        .is_err()
    );
    let newer_member = Membership::admit_as(
        &admin,
        authority.clone(),
        "new-family",
        &SigningKey::from_bytes(&[87; 32]).verifying_key(),
        MemberRole::FamilyMember,
        "new-principal",
        Validity {
            issued_at: f.now - 1,
            valid_for_secs: 100,
        },
    )
    .unwrap();
    assert!(
        store_control_record(
            conn,
            &f.network.public,
            &ControlPayload::Membership(newer_member),
            f.now
        )
        .is_err()
    );
    store_control_record(
        conn,
        &f.network.public,
        &ControlPayload::Ovlast(authority),
        f.now,
    )
    .unwrap();
    assert!(
        store_control_record(conn, &f.network.public, &issued, f.now).is_err(),
        "replaying the old authority cannot restore it"
    );
}

#[test]
fn control_write_failure_rolls_back_the_revocation_proof_and_flags() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.test_connection();
    let f = Fixture::new();
    let key = SigningKey::from_bytes(&[88; 32]);
    let public = verifying_key_to_base64(&key.verifying_key());
    store_control_record(
        conn,
        &f.network.public,
        &member(&f, &key, f.now - 20),
        f.now,
    )
    .unwrap();
    store_control_record(
        conn,
        &f.network.public,
        &grant(&f, &key, "grant-a", "list-a", ListPermission::Write),
        f.now,
    )
    .unwrap();
    let revoke = revocation(&f, &key, "list_grant", "grant-a");
    conn.execute_batch("CREATE TEMP TRIGGER fail_acl BEFORE UPDATE ON sync_trust_records WHEN NEW.revoked=1 BEGIN SELECT RAISE(ABORT,'injected ACL failure'); END;").unwrap();
    assert!(store_control_record(conn, &f.network.public, &revoke, f.now).is_err());
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sync_trust_records WHERE kind='revocation'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 0);
    assert!(
        allowed_channels_for_recipient(conn, &f.network.public, &public, f.now)
            .unwrap()
            .contains("list:list-a")
    );
    let ControlPayload::Revocation(r) = &revoke else {
        unreachable!()
    };
    let control = f.version(
        1,
        "revocation",
        &r.id,
        "control",
        serde_json::to_string(&revoke).unwrap(),
    );
    let report = f
        .apply(conn, &[control, f.list(2, "valid-despite-failed-control")])
        .unwrap();
    assert_eq!(report.rejected.len(), 1);
    assert_eq!(report.applied_versions, 1);
    let proofs: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sync_trust_records WHERE kind='revocation'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        proofs, 0,
        "catching a control failure inside a batch must not leave its partial proof"
    );
    conn.execute_batch("DROP TRIGGER fail_acl").unwrap();
    store_control_record(conn, &f.network.public, &revoke, f.now).unwrap();
    assert!(
        !allowed_channels_for_recipient(conn, &f.network.public, &public, f.now)
            .unwrap()
            .contains("list:list-a")
    );
}

#[test]
fn unknown_legacy_revocation_stays_fail_closed_during_scope_migration() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.test_connection();
    let f = Fixture::new();
    let key = SigningKey::from_bytes(&[90; 32]);
    let public = verifying_key_to_base64(&key.verifying_key());
    store_control_record(
        conn,
        &f.network.public,
        &member(&f, &key, f.now - 20),
        f.now,
    )
    .unwrap();
    store_control_record(
        conn,
        &f.network.public,
        &grant(&f, &key, "grant-a", "list-a", ListPermission::Write),
        f.now,
    )
    .unwrap();
    let unknown = revocation(&f, &key, "future-unknown-scope", "unknown");
    assert!(store_control_record(conn, &f.network.public, &unknown, f.now).is_err());
    let ControlPayload::Revocation(rev) = &unknown else {
        unreachable!()
    };
    conn.execute(
        "INSERT INTO sync_trust_records VALUES(?1,'revocation',?2,?3,'',?4,0,1,?5)",
        params![
            rev.id,
            rev.public_key,
            rev.target_id,
            rev.issued_at,
            serde_json::to_string(&unknown).unwrap()
        ],
    )
    .unwrap();
    conn.execute_batch("DELETE FROM schema_migrations WHERE version>=6")
        .unwrap();
    crate::migrations::run_migrations(conn).unwrap();
    let scope: String = conn
        .query_row(
            "SELECT permission FROM sync_trust_records WHERE record_id=?1",
            [&rev.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(scope, "legacy_key");
    assert!(
        allowed_channels_for_recipient(conn, &f.network.public, &public, f.now)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn scope_migration_repairs_only_the_grant_scope_and_rolls_back_on_failure() {
    let db = Database::open_in_memory().unwrap();
    let conn = db.test_connection();
    let f = Fixture::new();
    let key = SigningKey::from_bytes(&[89; 32]);
    let public = verifying_key_to_base64(&key.verifying_key());
    for ctrl in [
        member(&f, &key, f.now - 20),
        grant(&f, &key, "grant-a", "list-a", ListPermission::Write),
        grant(&f, &key, "grant-b", "list-b", ListPermission::Write),
        revocation(&f, &key, "list_grant", "grant-a"),
    ] {
        store_control_record(conn, &f.network.public, &ctrl, f.now).unwrap();
    }
    conn.execute(
        "UPDATE sync_trust_records SET revoked=1 WHERE kind!='revocation' AND subject_key=?1",
        [&public],
    )
    .unwrap();
    conn.execute_batch("UPDATE sync_trust_records SET permission='' WHERE kind='revocation'; DELETE FROM schema_migrations WHERE version>=6; CREATE TEMP TRIGGER fail_acl_migration BEFORE UPDATE ON sync_trust_records WHEN NEW.kind='membership' BEGIN SELECT RAISE(ABORT,'injected migration failure'); END;").unwrap();
    assert!(crate::migrations::run_migrations(conn).is_err());
    let scope: String = conn
        .query_row(
            "SELECT permission FROM sync_trust_records WHERE kind='revocation'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(scope.is_empty());
    let version: i64 = conn
        .query_row("SELECT MAX(version) FROM schema_migrations", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(version, 5);
    conn.execute_batch("DROP TRIGGER fail_acl_migration")
        .unwrap();
    crate::migrations::run_migrations(conn).unwrap();
    crate::migrations::run_migrations(conn).unwrap();
    let allowed = allowed_channels_for_recipient(conn, &f.network.public, &public, f.now).unwrap();
    assert!(allowed.contains("list:list-b"));
    assert!(!allowed.contains("list:list-a"));
}
