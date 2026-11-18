use crate::acl_types::{ListPermission, MemberRole};
use crate::calc::{MAX_SAFE_CENTS, checked_line_total_millis};
use crate::ledger::{
    DeltaBatch, HoleRecord, Version, compute_delta_paged_filtered, get_confirmed_frontier,
    narrow_holes, record_hole, record_rejected_range, update_peer_ack,
};
use crate::models::{
    ControlPayload, PlannedItemSyncPayload, PriceHistorySyncPayload, ProductSyncPayload,
    ReceiptItemSyncPayload, ReceiptSyncPayload, ShoppingListSyncPayload, StoreSyncPayload,
    millis_to_qty,
};
use ed25519_dalek::{SigningKey, VerifyingKey};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use syncnet::identity::{parse_public_key, public_key_to_base64};
use syncnet::trust::{MAX_CLOCK_SKEW_SECS, SignedVersionEnvelope};
pub use syncnet::wire::bundle_v2::ProtectedBundleV2;
use uuid::Uuid;

pub const PROTOCOL_NAME: &str = "list2cart";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RejectedVersion {
    pub version_id: String,
    pub author_node_id: String,
    pub channel: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ApplyReport {
    pub applied_versions: usize,
    pub projected_records: usize,
    pub deferred_children: usize,
    pub rejected: Vec<RejectedVersion>,
    pub applied_ids: Vec<String>,
    #[serde(default)]
    pub blocked_projections: Vec<RejectedVersion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtectedSyncPayload {
    pub sender_node_id: String,
    pub sender_pub_key: String,
    pub batch: DeltaBatch,
}

pub struct ApplyContext<'a> {
    pub network_pub: &'a VerifyingKey,
    pub peer_key: &'a str,
    pub recipient_b64: Option<&'a str>,
    pub now_unix: i64,
}

pub struct ExportRequest<'a> {
    pub local_node_id: &'a str,
    pub sender_sk: &'a SigningKey,
    pub network_pub: &'a VerifyingKey,
    pub recipient_pub: &'a VerifyingKey,
    pub peer_frontier: &'a BTreeMap<String, String>,
    pub requested_holes: &'a [HoleRecord],
    pub max_bytes: usize,
    pub now_unix: i64,
}

/// Validates and atomically updates the trust cache. Publishing callers must also
/// record the signed control version in their enclosing transaction.
pub fn store_control_record(
    conn: &Connection,
    network_pub: &VerifyingKey,
    ctrl: &ControlPayload,
    now_unix: i64,
) -> Result<(), String> {
    crate::acl::atomic(conn, || {
        store_control_record_inner(conn, network_pub, ctrl, now_unix)
    })
}

fn store_control_record_inner(
    conn: &Connection,
    network_pub: &VerifyingKey,
    ctrl: &ControlPayload,
    now_unix: i64,
) -> Result<(), String> {
    match ctrl {
        ControlPayload::Ovlast(o) => {
            if o.issued_at > now_unix {
                return Err("Ovlast još nije važeća".into());
            }
            o.verify(network_pub, o.issued_at)
                .map_err(|e| format!("Neispravna ovlast: {e}"))?;
            let revoked = crate::acl::revoked(conn, "ovlast", &o.device_key, &o.device_id)?;
            let raw = serde_json::to_string(ctrl).map_err(|e| format!("JSON greska: {e}"))?;
            if !crate::acl::may_store(
                conn,
                crate::acl::ControlIdentity {
                    id: &format!("ovlast:{}", o.device_key),
                    kind: "ovlast",
                    subject: &o.device_key,
                    target: &o.device_id,
                    issued: o.issued_at,
                    raw: &raw,
                },
            )? {
                return Ok(());
            }
            conn.execute(
                "INSERT OR REPLACE INTO sync_trust_records
                 (record_id, kind, subject_key, target_id, permission, issued_at, expires_at, revoked, raw_json)
                 VALUES (?1, 'ovlast', ?2, ?3, 'admit', ?4, ?5, ?6, ?7)",
                params![
                    format!("ovlast:{}", o.device_key),
                    o.device_key,
                    o.device_id,
                    o.issued_at,
                    o.expires_at,
                    i32::from(revoked),
                    raw
                ],
            )
            .map_err(|e| format!("Greska upisa ovlasti: {e}"))?;
        }
        ControlPayload::Membership(m) => {
            if m.issued_at > now_unix {
                return Err("Članstvo još nije važeće".into());
            }
            let member_vk = parse_public_key(&m.device_key)
                .map_err(|e| format!("Neispravan kljuc clanstva: {e}"))?;
            m.verify(network_pub, &member_vk, m.issued_at)
                .map_err(|e| format!("Neispravno clanstvo: {e}"))?;
            crate::acl::check_delegation_at(conn, network_pub, m.admitter.as_ref(), m.issued_at)?;
            let revoked = crate::acl::revoked(conn, "membership", &m.device_key, &m.device_id)?;
            let raw = serde_json::to_string(ctrl).map_err(|e| format!("JSON greska: {e}"))?;
            if !crate::acl::may_store(
                conn,
                crate::acl::ControlIdentity {
                    id: &format!("membership:{}", m.device_key),
                    kind: "membership",
                    subject: &m.device_key,
                    target: &m.device_key,
                    issued: m.issued_at,
                    raw: &raw,
                },
            )? {
                return Ok(());
            }
            conn.execute(
                "INSERT OR REPLACE INTO sync_trust_records
                 (record_id, kind, subject_key, target_id, permission, issued_at, expires_at, revoked, raw_json)
                 VALUES (?1, 'membership', ?2, ?3, '', ?4, ?5, ?6, ?7)",
                params![
                    format!("membership:{}", m.device_key),
                    m.device_key,
                    m.device_key,
                    m.issued_at,
                    m.expires_at,
                    i32::from(revoked),
                    raw
                ],
            )
            .map_err(|e| format!("Greska upisa clanstva: {e}"))?;
        }
        ControlPayload::MemberRole(r) => {
            if r.issued_at > now_unix {
                return Err("Uloga članstva još nije važeća".into());
            }
            let member_vk = parse_public_key(&r.device_key)
                .map_err(|e| format!("Neispravan kljuc clanstva: {e}"))?;
            r.verify(network_pub, &member_vk, r.issued_at)
                .map_err(|e| format!("Neispravna uloga clanstva: {e}"))?;
            crate::acl::check_delegation_at(conn, network_pub, r.admitter.as_ref(), r.issued_at)?;
            let revoked = crate::acl::revoked(conn, "membership", &r.device_key, &r.device_id)?;
            let role_str = match r.role {
                MemberRole::OwnDevice => "own_device",
                MemberRole::FamilyMember => "family_member",
            };
            let raw = serde_json::to_string(ctrl).map_err(|e| format!("JSON greska: {e}"))?;
            if !crate::acl::may_store(
                conn,
                crate::acl::ControlIdentity {
                    id: &format!("member_role:{}", r.device_key),
                    kind: "member_role",
                    subject: &r.device_key,
                    target: &r.principal_id,
                    issued: r.issued_at,
                    raw: &raw,
                },
            )? {
                return Ok(());
            }
            conn.execute(
                "INSERT OR REPLACE INTO sync_trust_records
                 (record_id, kind, subject_key, target_id, permission, issued_at, expires_at, revoked, raw_json)
                 VALUES (?1, 'member_role', ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    format!("member_role:{}", r.device_key),
                    r.device_key,
                    r.principal_id,
                    role_str,
                    r.issued_at,
                    r.expires_at,
                    i32::from(revoked),
                    raw
                ],
            )
            .map_err(|e| format!("Greska upisa uloge clanstva: {e}"))?;
        }
        ControlPayload::ListGrant(g) => {
            if g.issued_at > now_unix {
                return Err("Grant još nije važeći".into());
            }
            let rec_vk = parse_public_key(&g.recipient_pub_key)
                .map_err(|e| format!("Neispravan kljuc granta: {e}"))?;
            g.verify(network_pub, &rec_vk, g.issued_at)
                .map_err(|e| format!("Neispravan grant popisa: {e}"))?;
            crate::acl::check_delegation_at(conn, network_pub, g.admitter.as_ref(), g.issued_at)?;
            let revoked =
                crate::acl::revoked(conn, "list_grant", &g.recipient_pub_key, &g.grant_id)?;
            let perm = match g.permission {
                ListPermission::Read => "read",
                ListPermission::Write => "write",
            };
            let raw = serde_json::to_string(ctrl).map_err(|e| format!("JSON greska: {e}"))?;
            if !crate::acl::may_store(
                conn,
                crate::acl::ControlIdentity {
                    id: &g.grant_id,
                    kind: "list_grant",
                    subject: &g.recipient_pub_key,
                    target: &g.list_sync_id,
                    issued: g.issued_at,
                    raw: &raw,
                },
            )? {
                return Ok(());
            }
            conn.execute(
                "INSERT OR REPLACE INTO sync_trust_records
                 (record_id, kind, subject_key, target_id, permission, issued_at, expires_at, revoked, raw_json)
                 VALUES (?1, 'list_grant', ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    g.grant_id,
                    g.recipient_pub_key,
                    g.list_sync_id,
                    perm,
                    g.issued_at,
                    g.expires_at,
                    i32::from(revoked),
                    raw
                ],
            )
            .map_err(|e| format!("Greska upisa granta popisa: {e}"))?;
        }
        ControlPayload::Revocation(r) => {
            let signer = r
                .verify_self()
                .map_err(|e| format!("Neispravan potpis opoziva: {e}"))?;
            let signer_b64 = public_key_to_base64(signer.as_bytes());
            let network_b64 = public_key_to_base64(network_pub.as_bytes());
            if signer_b64 != network_b64
                && !crate::acl::historical_admin(conn, network_pub, &signer_b64, r.issued_at)?
            {
                return Err("Potpisnik opoziva nema ovlast na mrezi".to_owned());
            }
            let scope = r.scope().map_err(|e| e.to_string())?;
            if !matches!(scope, "membership" | "ovlast" | "list_grant") {
                return Err("Nepoznata vrsta opoziva".into());
            }
            if r.public_key == network_b64 && scope != "list_grant" {
                return Err(
                    "Korijenski ključ mreže nije članstvo ni delegirana ovlast uređaja".into(),
                );
            }
            if r.issued_at > now_unix || r.revoked_at > now_unix {
                return Err("Opoziv još nije važeći".into());
            }
            if scope == "list_grant" {
                let existing: Option<(String, String)> = conn
                    .query_row(
                        "SELECT kind,subject_key FROM sync_trust_records WHERE record_id=?1",
                        [&r.target_id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()
                    .map_err(|e| e.to_string())?;
                if existing.is_some_and(|(kind, key)| kind != "list_grant" || key != r.public_key) {
                    return Err("Opoziv granta ne odgovara ključu i cilju ovlasti".into());
                }
            }
            let raw = serde_json::to_string(ctrl).map_err(|e| format!("JSON greska: {e}"))?;
            if !crate::acl::may_store(
                conn,
                crate::acl::ControlIdentity {
                    id: &r.id,
                    kind: "revocation",
                    subject: &r.public_key,
                    target: &r.target_id,
                    issued: r.issued_at,
                    raw: &raw,
                },
            )? {
                return Ok(());
            }
            conn.execute(
                "INSERT OR REPLACE INTO sync_trust_records
                 (record_id, kind, subject_key, target_id, permission, issued_at, expires_at, revoked, raw_json)
                   VALUES (?1, 'revocation', ?2, ?3, ?4, ?5, 0, 1, ?6)",
                params![r.id, r.public_key, r.target_id, scope,r.issued_at, raw],
            )
            .map_err(|e| format!("Greska upisa opoziva: {e}"))?;

            crate::acl::apply_revocation_flags(conn, false)?;
        }
    }
    Ok(())
}

pub(crate) fn control_payload_is_relayable(
    conn: &Connection,
    network_pub: &VerifyingKey,
    ctrl: &ControlPayload,
    now_unix: i64,
) -> Result<(), String> {
    let raw = serde_json::to_string(ctrl).map_err(|e| format!("JSON greska: {e}"))?;
    match ctrl {
        ControlPayload::Ovlast(o) => {
            if o.issued_at > now_unix {
                return Err("Ovlast još nije važeća".into());
            }
            o.verify(network_pub, o.issued_at)
                .map_err(|e| format!("Neispravna ovlast: {e}"))?;
            crate::acl::may_store(
                conn,
                crate::acl::ControlIdentity {
                    id: &format!("ovlast:{}", o.device_key),
                    kind: "ovlast",
                    subject: &o.device_key,
                    target: &o.device_id,
                    issued: o.issued_at,
                    raw: &raw,
                },
            )?;
        }
        ControlPayload::Membership(m) => {
            if m.issued_at > now_unix {
                return Err("Članstvo još nije važeće".into());
            }
            let member_vk = parse_public_key(&m.device_key)
                .map_err(|e| format!("Neispravan kljuc clanstva: {e}"))?;
            m.verify(network_pub, &member_vk, m.issued_at)
                .map_err(|e| format!("Neispravno clanstvo: {e}"))?;
            crate::acl::check_delegation_at(conn, network_pub, m.admitter.as_ref(), m.issued_at)?;
            crate::acl::may_store(
                conn,
                crate::acl::ControlIdentity {
                    id: &format!("membership:{}", m.device_key),
                    kind: "membership",
                    subject: &m.device_key,
                    target: &m.device_key,
                    issued: m.issued_at,
                    raw: &raw,
                },
            )?;
        }
        ControlPayload::MemberRole(r) => {
            if r.issued_at > now_unix {
                return Err("Uloga članstva još nije važeća".into());
            }
            let member_vk = parse_public_key(&r.device_key)
                .map_err(|e| format!("Neispravan kljuc clanstva: {e}"))?;
            r.verify(network_pub, &member_vk, r.issued_at)
                .map_err(|e| format!("Neispravna uloga clanstva: {e}"))?;
            crate::acl::check_delegation_at(conn, network_pub, r.admitter.as_ref(), r.issued_at)?;
            let raw = serde_json::to_string(ctrl).map_err(|e| format!("JSON greska: {e}"))?;
            crate::acl::may_store(
                conn,
                crate::acl::ControlIdentity {
                    id: &format!("member_role:{}", r.device_key),
                    kind: "member_role",
                    subject: &r.device_key,
                    target: &r.principal_id,
                    issued: r.issued_at,
                    raw: &raw,
                },
            )?;
        }
        ControlPayload::ListGrant(g) => {
            if g.issued_at > now_unix {
                return Err("Grant još nije važeći".into());
            }
            let rec_vk = parse_public_key(&g.recipient_pub_key)
                .map_err(|e| format!("Neispravan kljuc granta: {e}"))?;
            g.verify(network_pub, &rec_vk, g.issued_at)
                .map_err(|e| format!("Neispravan grant popisa: {e}"))?;
            crate::acl::check_delegation_at(conn, network_pub, g.admitter.as_ref(), g.issued_at)?;
            crate::acl::may_store(
                conn,
                crate::acl::ControlIdentity {
                    id: &g.grant_id,
                    kind: "list_grant",
                    subject: &g.recipient_pub_key,
                    target: &g.list_sync_id,
                    issued: g.issued_at,
                    raw: &raw,
                },
            )?;
        }
        ControlPayload::Revocation(r) => {
            let signer = r
                .verify_self()
                .map_err(|e| format!("Neispravan potpis opoziva: {e}"))?;
            let signer_b64 = public_key_to_base64(signer.as_bytes());
            let network_b64 = public_key_to_base64(network_pub.as_bytes());
            if signer_b64 != network_b64
                && !crate::acl::historical_admin(conn, network_pub, &signer_b64, r.issued_at)?
            {
                return Err("Potpisnik opoziva nema ovlast na mrezi".to_owned());
            }
            let scope = r.scope().map_err(|e| e.to_string())?;
            if !matches!(scope, "membership" | "ovlast" | "list_grant") {
                return Err("Nepoznata vrsta opoziva".into());
            }
            if r.public_key == network_b64 && scope != "list_grant" {
                return Err(
                    "Korijenski ključ mreže nije članstvo ni delegirana ovlast uređaja".into(),
                );
            }
            if r.issued_at > now_unix || r.revoked_at > now_unix {
                return Err("Opoziv još nije važeći".into());
            }
            if scope == "list_grant" {
                let existing: Option<(String, String)> = conn
                    .query_row(
                        "SELECT kind,subject_key FROM sync_trust_records WHERE record_id=?1",
                        [&r.target_id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()
                    .map_err(|e| e.to_string())?;
                if existing.is_some_and(|(kind, key)| kind != "list_grant" || key != r.public_key) {
                    return Err("Opoziv granta ne odgovara ključu i cilju ovlasti".into());
                }
            }
            crate::acl::may_store(
                conn,
                crate::acl::ControlIdentity {
                    id: &r.id,
                    kind: "revocation",
                    subject: &r.public_key,
                    target: &r.target_id,
                    issued: r.issued_at,
                    raw: &raw,
                },
            )?;
        }
    }
    Ok(())
}

pub(crate) fn control_version_is_relayable(
    conn: &Connection,
    network_pub: &VerifyingKey,
    v: &Version,
    now_unix: i64,
) -> Result<ControlPayload, String> {
    let ctrl = verify_control_version_envelope(v, network_pub, conn, now_unix)?;
    control_payload_is_relayable(conn, network_pub, &ctrl, now_unix)?;
    Ok(ctrl)
}

pub(crate) fn relayable_control_versions(
    conn: &Connection,
    network_pub: &VerifyingKey,
    now_unix: i64,
) -> Result<Vec<Version>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT version_id,entity,entity_id,node_id,supersedes,archived,payload,created_at,schema_version,channel,author_pub_key,grant_ref,signature,created_at_unix
             FROM record_versions WHERE channel='control' AND signature!='' AND legacy_imported=0 ORDER BY version_id ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], crate::ledger::read_version_row)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<Version>, _>>()
        .map_err(|e| e.to_string())?;
    let mut candidates = Vec::new();
    let mut immutable_ids = BTreeMap::<String, BTreeSet<String>>::new();
    for v in rows {
        if let Ok(ctrl) = control_version_is_relayable(conn, network_pub, &v, now_unix) {
            let raw = serde_json::to_string(&ctrl).map_err(|e| e.to_string())?;
            let immutable_id = match &ctrl {
                ControlPayload::ListGrant(g) => Some(g.grant_id.clone()),
                ControlPayload::Revocation(r) => Some(r.id.clone()),
                _ => None,
            };
            if let Some(id) = immutable_id.clone() {
                immutable_ids.entry(id).or_default().insert(raw.clone());
            }
            candidates.push((v, immutable_id));
        }
    }
    Ok(candidates
        .into_iter()
        .filter_map(|(v, immutable_id)| {
            if let Some(id) = immutable_id
                && immutable_ids.get(&id).is_some_and(|set| set.len() > 1)
            {
                return None;
            }
            Some(v)
        })
        .collect())
}

pub(crate) fn relayable_domain_versions(
    conn: &Connection,
    network_pub: &VerifyingKey,
    channels: &BTreeSet<String>,
    now_unix: i64,
) -> Result<Vec<Version>, String> {
    let net_b64 = public_key_to_base64(network_pub.as_bytes());
    let relayable_controls = relayable_control_versions(conn, network_pub, now_unix)?;
    let mut relayable_members = BTreeSet::new();
    let mut relayable_grants = BTreeSet::new();
    for cv in &relayable_controls {
        if let Ok(ctrl) = serde_json::from_str::<ControlPayload>(&cv.payload) {
            match ctrl {
                ControlPayload::Membership(m) => {
                    relayable_members.insert(m.device_key);
                }
                ControlPayload::ListGrant(g) => {
                    relayable_grants.insert(g.grant_id);
                }
                _ => {}
            }
        }
    }
    let has_any_control_versions: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM record_versions WHERE channel='control' AND signature!='' AND legacy_imported=0)",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT version_id,entity,entity_id,node_id,supersedes,archived,payload,created_at,schema_version,channel,author_pub_key,grant_ref,signature,created_at_unix
             FROM record_versions
             WHERE channel!='control' AND signature!='' AND legacy_imported=0
               AND version_id NOT IN (SELECT version_id FROM pending_projections)
             ORDER BY version_id ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], crate::ledger::read_version_row)
        .map_err(|e| e.to_string())?;
    let mut verified = Vec::new();
    for row in rows {
        let v = row.map_err(|e| e.to_string())?;
        if !channels.contains(&v.channel) {
            continue;
        }
        if has_any_control_versions && v.author_pub_key != net_b64 {
            let member_in_ledger: bool = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM record_versions WHERE channel='control' AND signature!='' AND legacy_imported=0 AND CASE WHEN json_valid(payload) THEN json_extract(payload,'$.control_kind')='membership' AND json_extract(payload,'$.data.deviceKey')=?1 ELSE 0 END)",
                    [&v.author_pub_key],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if member_in_ledger && !relayable_members.contains(&v.author_pub_key) {
                continue;
            }
            if v.channel.starts_with("list:") {
                let grant_in_ledger: bool = conn
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM record_versions WHERE channel='control' AND signature!='' AND legacy_imported=0 AND CASE WHEN json_valid(payload) THEN json_extract(payload,'$.control_kind')='list_grant' AND json_extract(payload,'$.data.grantId')=?1 ELSE 0 END)",
                        [&v.grant_ref],
                        |r| r.get(0),
                    )
                    .map_err(|e| e.to_string())?;
                if grant_in_ledger && !relayable_grants.contains(&v.grant_ref) {
                    continue;
                }
            }
        }
        let different_channel: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM record_versions WHERE entity=?1 AND entity_id=?2 AND channel!=?3 AND signature!='')",
                params![v.entity, v.entity_id, v.channel],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if different_channel
            || verify_domain_version_auth(conn, network_pub, &v, now_unix).is_err()
            || validate_payload_syntax_only(&v).is_err()
            || validate_identity_binding(conn, &v).is_err()
        {
            continue;
        }
        verified.push(v);
    }
    let mut valid_stores = BTreeSet::new();
    let mut valid_products = BTreeSet::new();
    let mut valid_lists = BTreeSet::new();
    for v in &verified {
        match v.entity.as_str() {
            "stores" => {
                valid_stores.insert(v.entity_id.clone());
            }
            "products" => {
                if let Ok(p) = serde_json::from_str::<ProductSyncPayload>(&v.payload) {
                    valid_products.insert((p.sync_id, p.barcode.trim().to_owned()));
                }
            }
            "shopping_lists" => {
                valid_lists.insert(v.entity_id.clone());
            }
            _ => {}
        }
    }
    let mut valid_receipts = BTreeSet::new();
    for v in &verified {
        if v.entity == "receipts"
            && let Ok(p) = serde_json::from_str::<ReceiptSyncPayload>(&v.payload)
            && valid_stores.contains(&p.store_sync_id)
        {
            valid_receipts.insert(v.entity_id.clone());
        }
    }
    Ok(verified
        .into_iter()
        .filter(|v| match v.entity.as_str() {
            "stores" | "products" | "shopping_lists" => true,
            "planned_items" => serde_json::from_str::<PlannedItemSyncPayload>(&v.payload)
                .is_ok_and(|p| valid_lists.contains(&p.list_sync_id)),
            "price_history" => serde_json::from_str::<PriceHistorySyncPayload>(&v.payload)
                .is_ok_and(|p| {
                    valid_products.contains(&(p.product_sync_id, p.barcode.trim().to_owned()))
                }),
            "receipts" => valid_receipts.contains(&v.entity_id),
            "receipt_items" => serde_json::from_str::<ReceiptItemSyncPayload>(&v.payload)
                .is_ok_and(|p| valid_receipts.contains(&p.receipt_sync_id)),
            _ => false,
        })
        .collect())
}

fn verify_control_version_envelope(
    v: &Version,
    network_pub: &VerifyingKey,
    conn: &Connection,
    now_unix: i64,
) -> Result<ControlPayload, String> {
    validate_version_metadata(v, now_unix)?;
    let env = SignedVersionEnvelope {
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
    let signer = env
        .verify_signature(PROTOCOL_NAME)
        .map_err(|e| format!("Neispravan potpis kontrolnog zapisa: {e}"))?;
    let signer_b64 = public_key_to_base64(signer.as_bytes());
    let net_b64 = public_key_to_base64(network_pub.as_bytes());
    if signer_b64 != net_b64
        && !crate::acl::historical_admin(conn, network_pub, &signer_b64, v.created_at_unix)?
    {
        return Err("Posiljatelj kontrolnog zapisa nema ovlast mreze".to_owned());
    }

    serde_json::from_str::<ControlPayload>(&v.payload)
        .map_err(|e| format!("Neispravan kontrolni payload: {e}"))
}

pub(crate) fn verify_domain_version_auth(
    conn: &Connection,
    network_pub: &VerifyingKey,
    v: &Version,
    now_unix: i64,
) -> Result<(), String> {
    validate_version_metadata(v, now_unix)?;
    if v.channel.starts_with("list:") && v.entity != "shopping_lists" && v.entity != "planned_items"
    {
        return Err("Kanal popisa ne smije nositi privatne entitete".into());
    }
    if v.schema_version != 1 {
        return Err(format!("Nepodrzana verzija sheme: {}", v.schema_version));
    }
    if v.version_id.trim().is_empty() || Uuid::parse_str(&v.version_id).is_err() {
        return Err("Neispravan version_id UUID".to_owned());
    }
    if v.node_id.contains('|') || v.channel.contains('|') {
        return Err("node_id ili channel sadrzi nedopusteni znak `|`".to_owned());
    }
    if v.created_at_unix > now_unix + MAX_CLOCK_SKEW_SECS {
        return Err("Vrijeme verzije je predaleko u buducnosti".to_owned());
    }

    let env = SignedVersionEnvelope {
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
    let author_vk = env
        .verify_signature(PROTOCOL_NAME)
        .map_err(|e| format!("Neispravan potpis autora verzije: {e}"))?;
    let author_b64 = public_key_to_base64(author_vk.as_bytes());
    let network_b64 = public_key_to_base64(network_pub.as_bytes());

    // Device membership revocations block the author key. List-grant revocations
    // are scoped to the referenced grant and must not disable unrelated lists.
    let author_revoked: bool = conn
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM sync_trust_records
                WHERE kind = 'revocation' AND subject_key = ?1 AND permission IN ('membership','legacy_key')
            )",
            params![author_b64],
            |r| r.get(0),
        )
        .map_err(|e| format!("Greska provjere opoziva autora: {e}"))?;
    if author_revoked {
        return Err("Kljuc autora je opozvan".to_owned());
    }

    if author_b64 == network_b64 {
        return Ok(());
    }

    let Some(_membership) =
        crate::acl::historical_membership(conn, network_pub, &author_b64, v.created_at_unix)?
    else {
        return Err("Autor nema valjano članstvo u trenutku nastanka verzije".into());
    };

    if let Some(target_principal) = v.channel.strip_prefix("private:") {
        let Some(role_rec) =
            crate::acl::historical_member_role(conn, network_pub, &author_b64, v.created_at_unix)?
        else {
            return Err("Autor nema valjanu ulogu članstva u trenutku nastanka verzije".into());
        };
        if role_rec.role != MemberRole::OwnDevice || role_rec.principal_id != target_principal {
            return Err(
                "Privatni kanal dopusten je iskljucivo provjerenom vlastitom uredjaju istog principala"
                    .to_owned(),
            );
        }
        return Ok(());
    }

    if let Some(list_sync_id) = v.channel.strip_prefix("list:") {
        let raw: Option<String> = conn
            .query_row(
                "SELECT raw_json FROM sync_trust_records WHERE kind='list_grant' AND subject_key=?1
             AND target_id=?2 AND permission='write' AND revoked=0 AND record_id=?3",
                params![author_b64, list_sync_id, v.grant_ref],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| format!("Greska provjere write ovlasti: {e}"))?;
        let Some(raw) = raw else {
            return Err("Autor nema write ovlast za ovaj popis".into());
        };
        let ControlPayload::ListGrant(grant) =
            serde_json::from_str(&raw).map_err(|e| format!("Neispravan dokaz granta: {e}"))?
        else {
            return Err("Neispravan tip write ovlasti".into());
        };
        if grant.permission != ListPermission::Write
            || grant.list_sync_id != list_sync_id
            || grant.grant_id != v.grant_ref
        {
            return Err("Potpisani grant ne daje write pravo na ovaj popis".into());
        }
        grant
            .verify(network_pub, &author_vk, v.created_at_unix)
            .map_err(|e| format!("Neispravan write grant: {e}"))?;
        crate::acl::check_delegation_at(
            conn,
            network_pub,
            grant.admitter.as_ref(),
            grant.issued_at,
        )?;
        if crate::acl::revoked(conn, "list_grant", &author_b64, &grant.grant_id)? {
            return Err("Write ovlast je opozvana".into());
        }
        return Ok(());
    }

    Err(format!("Nepoznat ili nedopusten kanal: {}", v.channel))
}

fn validate_version_metadata(v: &Version, now_unix: i64) -> Result<(), String> {
    let key = parse_public_key(&v.author_pub_key).map_err(|e| e.to_string())?;
    if v.node_id != public_key_to_base64(key.as_bytes()) || v.author_pub_key != v.node_id {
        return Err("Identitet autora i cvor verzije se ne podudaraju".into());
    }
    let id = Uuid::parse_str(&v.version_id).map_err(|_| "Neispravan UUID verzije")?;
    let (timestamp, _) = id
        .get_timestamp()
        .ok_or("UUID verzije nema vrijeme")?
        .to_unix();
    if id.get_version_num() != 7 || id.to_string() != v.version_id || v.schema_version != 1 {
        return Err("Nepodrzana shema ili nekanonski UUIDv7".into());
    }
    if timestamp > now_unix.saturating_add(MAX_CLOCK_SKEW_SECS).max(0) as u64
        || timestamp.abs_diff(v.created_at_unix.max(0) as u64) > 1
    {
        return Err("Vrijeme UUIDv7 ne odgovara vremenu verzije".into());
    }
    let created =
        chrono::DateTime::parse_from_rfc3339(&v.created_at).map_err(|_| "Neispravan created_at")?;
    if created.timestamp() != v.created_at_unix
        || v.node_id.is_empty()
        || v.node_id.contains('|')
        || v.channel.contains('|')
    {
        return Err("Neispravni metapodaci verzije".into());
    }
    Ok(())
}

fn validate_identity_binding(conn: &Connection, v: &Version) -> Result<(), String> {
    let payload: serde_json::Value = serde_json::from_str(&v.payload).map_err(|e| e.to_string())?;
    let immutable_fields: &[&str] = match v.entity.as_str() {
        "products" => &["barcode"],
        "price_history" => &["product_sync_id", "store_sync_id", "barcode"],
        "receipts" => &["store_sync_id"],
        "receipt_items" => &["receipt_sync_id"],
        "planned_items" => &["list_sync_id"],
        _ => &[],
    };
    if immutable_fields.is_empty() {
        return Ok(());
    }
    // Include admitted but not yet projected children, whose parent may arrive later.
    let mut stmt = conn.prepare("SELECT payload FROM record_versions WHERE entity=?1 AND entity_id=?2 AND signature != '' AND legacy_imported=0")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![v.entity, v.entity_id], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    for row in rows {
        let previous: serde_json::Value =
            serde_json::from_str(&row.map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        if immutable_fields
            .iter()
            .any(|field| previous[*field] != payload[*field])
        {
            return Err("Stabilni identitet ne smije promijeniti roditelja ili barkod".into());
        }
    }
    Ok(())
}

fn validate_and_project_entity(conn: &Connection, v: &Version) -> Result<bool, String> {
    match v.entity.as_str() {
        "stores" => {
            let p: StoreSyncPayload = serde_json::from_str(&v.payload)
                .map_err(|e| format!("Neispravan payload trgovine: {e}"))?;
            if p.sync_id != v.entity_id || p.name.trim().is_empty() {
                return Err("Neispravan sync_id ili prazan naziv trgovine".to_owned());
            }
            let existing: Option<i64> = conn
                .query_row(
                    "SELECT id FROM stores WHERE sync_id = ?1",
                    params![p.sync_id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| format!("Greska upita trgovine: {e}"))?;
            match existing {
                None => {
                    conn.execute(
                        "INSERT INTO stores (sync_id, name, location, color, updated_at, archived)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                        params![
                            p.sync_id,
                            p.name.trim(),
                            p.location.trim(),
                            p.color.trim(),
                            p.updated_at,
                            i32::from(v.archived || p.archived)
                        ],
                    )
                    .map_err(|e| format!("Greska projekcije trgovine: {e}"))?;
                }
                Some(id) => {
                    conn.execute(
                        "UPDATE stores SET sync_id = ?1, name = ?2, location = ?3, color = ?4, updated_at = ?5, archived = ?6 WHERE id = ?7",
                        params![
                            p.sync_id,
                            p.name.trim(),
                            p.location.trim(),
                            p.color.trim(),
                            p.updated_at,
                            i32::from(v.archived || p.archived),
                            id
                        ],
                    )
                    .map_err(|e| format!("Greska azuriranja trgovine: {e}"))?;
                }
            }
            Ok(true)
        }
        "products" => {
            let p: ProductSyncPayload = serde_json::from_str(&v.payload)
                .map_err(|e| format!("Neispravan payload artikla: {e}"))?;
            if p.sync_id != v.entity_id
                || p.barcode.trim().is_empty()
                || p.name.trim().is_empty()
                || !(0..=MAX_SAFE_CENTS).contains(&p.last_price_cents)
            {
                return Err("Neispravna polja ili cijena artikla".to_owned());
            }
            let mut resolved_hash = String::new();
            let mut resolved_inline = None;
            if let Some(hash) = p
                .image_hash
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                let mime = p.image_mime.as_deref().unwrap_or("");
                crate::content::record_content_ref_and_want(
                    conn,
                    hash,
                    mime,
                    p.image_bytes,
                    "products",
                    &p.sync_id,
                    &v.channel,
                )?;
                resolved_hash = hash.to_owned();
            } else if let Some(raw_url) = p
                .image_data_url
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                if let Ok((mime, bytes)) = crate::content::parse_image_data_url(raw_url) {
                    resolved_hash = crate::content::store_verified_blob(
                        conn,
                        None,
                        None,
                        &mime,
                        &bytes,
                        "peer",
                        Some(("products", &p.sync_id, &v.channel)),
                    )?;
                } else {
                    resolved_inline = Some(raw_url.to_owned());
                }
            }
            conn.execute(
                "INSERT INTO products (barcode, sync_id, name, category, default_unit, last_price_cents, last_store_name, image_data_url, image_hash, updated_at, archived)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                   ON CONFLICT(sync_id) DO UPDATE SET
                    name = excluded.name,
                    category = excluded.category,
                    default_unit = excluded.default_unit,
                    last_price_cents = excluded.last_price_cents,
                    last_store_name = excluded.last_store_name,
                    image_data_url = CASE WHEN excluded.image_hash != '' THEN NULL ELSE COALESCE(excluded.image_data_url, products.image_data_url) END,
                    image_hash = CASE WHEN excluded.image_hash != '' THEN excluded.image_hash ELSE products.image_hash END,
                    updated_at = excluded.updated_at,
                    archived = excluded.archived",
                params![
                    p.barcode.trim(),
                    p.sync_id,
                    p.name.trim(),
                    p.category.trim(),
                    p.default_unit.as_str(),
                    p.last_price_cents,
                    p.last_store_name.trim(),
                    resolved_inline,
                    resolved_hash,
                    p.updated_at,
                    i32::from(v.archived || p.archived)
                ],
            )
            .map_err(|e| format!("Greska projekcije artikla: {e}"))?;
            Ok(true)
        }
        "price_history" => {
            let p: PriceHistorySyncPayload = serde_json::from_str(&v.payload)
                .map_err(|e| format!("Neispravan payload cijene: {e}"))?;
            if p.sync_id != v.entity_id
                || !(0..=MAX_SAFE_CENTS).contains(&p.price_cents)
                || p.barcode.trim().is_empty()
            {
                return Err("Neispravan zapis povijesti cijene".to_owned());
            }
            let has_prod: bool = conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM products WHERE sync_id = ?1 AND barcode = ?2)",
                    params![p.product_sync_id, p.barcode.trim()],
                    |r| r.get(0),
                )
                .map_err(|e| format!("Greska provjere artikla cijene: {e}"))?;
            if !has_prod {
                enqueue_pending(conn, v, "products", &p.product_sync_id)?;
                return Ok(false);
            }
            conn.execute(
                "INSERT INTO price_history (sync_id, barcode, store_sync_id, product_sync_id, store_name, price_cents, recorded_at, updated_at, archived)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(sync_id) DO UPDATE SET
                    price_cents = excluded.price_cents,
                    updated_at = excluded.updated_at,
                    archived = excluded.archived",
                params![
                    p.sync_id,
                    p.barcode.trim(),
                    p.store_sync_id,
                    p.product_sync_id,
                    p.store_name.trim(),
                    p.price_cents,
                    p.recorded_at,
                    p.updated_at,
                    i32::from(v.archived || p.archived)
                ],
            )
            .map_err(|e| format!("Greska projekcije povijesti cijene: {e}"))?;
            Ok(true)
        }
        "receipts" => {
            let p: ReceiptSyncPayload = serde_json::from_str(&v.payload)
                .map_err(|e| format!("Neispravan payload racuna: {e}"))?;
            if p.sync_id != v.entity_id
                || !(0..=MAX_SAFE_CENTS).contains(&p.cart_total_cents)
                || !(0..=MAX_SAFE_CENTS).contains(&p.receipt_total_cents)
                || p.difference_cents != p.cart_total_cents - p.receipt_total_cents
            {
                return Err("Neusklađeni zbrojevi racuna".to_owned());
            }
            let store_id: Option<i64> = conn
                .query_row(
                    "SELECT id FROM stores WHERE sync_id = ?1",
                    params![p.store_sync_id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| format!("Greska trazenja trgovine racuna: {e}"))?;
            let Some(store_id) = store_id else {
                enqueue_pending(conn, v, "stores", &p.store_sync_id)?;
                return Ok(false);
            };

            conn.execute(
                "INSERT INTO receipts (sync_id, store_id, store_sync_id, store_name, created_at, cart_total_cents, receipt_total_cents, difference_cents, archived)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(sync_id) DO UPDATE SET
                    store_id = excluded.store_id,
                    store_sync_id = excluded.store_sync_id,
                    store_name = excluded.store_name,
                    cart_total_cents = excluded.cart_total_cents,
                    receipt_total_cents = excluded.receipt_total_cents,
                    difference_cents = excluded.difference_cents,
                    archived = excluded.archived",
                params![
                    p.sync_id,
                    store_id,
                    p.store_sync_id,
                    p.store_name.trim(),
                    p.created_at,
                    p.cart_total_cents,
                    p.receipt_total_cents,
                    p.difference_cents,
                    i32::from(v.archived || p.archived)
                ],
            )
            .map_err(|e| format!("Greska projekcije racuna: {e}"))?;
            Ok(true)
        }
        "receipt_items" => {
            let p: ReceiptItemSyncPayload = serde_json::from_str(&v.payload)
                .map_err(|e| format!("Neispravan payload stavke racuna: {e}"))?;
            let qty = millis_to_qty(p.qty_millis);
            let expected_total =
                checked_line_total_millis(p.qty_millis, p.unit, p.unit_price_cents)?;
            if p.sync_id != v.entity_id
                || p.qty_millis <= 0
                || p.unit_price_cents < 0
                || p.total_cents != expected_total
            {
                return Err("Neispravna kolicina ili izracun stavke racuna".to_owned());
            }
            let receipt_id: Option<i64> = conn
                .query_row(
                    "SELECT id FROM receipts WHERE sync_id = ?1",
                    params![p.receipt_sync_id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| format!("Greska provjere roditeljskog racuna: {e}"))?;
            let Some(rid) = receipt_id else {
                enqueue_pending(conn, v, "receipts", &p.receipt_sync_id)?;
                return Ok(false);
            };
            conn.execute(
                "INSERT INTO receipt_items (sync_id, receipt_id, receipt_sync_id, barcode, name, quantity, unit, unit_price_cents, total_cents, updated_at, archived)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(sync_id) DO UPDATE SET
                      barcode = excluded.barcode,
                      name = excluded.name,
                    quantity = excluded.quantity,
                    unit = excluded.unit,
                    unit_price_cents = excluded.unit_price_cents,
                    total_cents = excluded.total_cents,
                    updated_at = excluded.updated_at,
                    archived = excluded.archived",
                params![
                    p.sync_id,
                    rid,
                    p.receipt_sync_id,
                    p.barcode.trim(),
                    p.name.trim(),
                    qty,
                    p.unit.as_str(),
                    p.unit_price_cents,
                    p.total_cents,
                    p.updated_at,
                    i32::from(v.archived || p.archived)
                ],
            )
            .map_err(|e| format!("Greska projekcije stavke racuna: {e}"))?;
            Ok(true)
        }
        "shopping_lists" => {
            let p: ShoppingListSyncPayload = serde_json::from_str(&v.payload)
                .map_err(|e| format!("Neispravan payload popisa: {e}"))?;
            if p.sync_id != v.entity_id || p.name.trim().is_empty() {
                return Err("Neispravan sync_id ili prazan naziv popisa".to_owned());
            }
            let resolved_store_id: Option<i64> = if let Some(ref s_sync) = p.store_sync_id {
                conn.query_row(
                    "SELECT id FROM stores WHERE sync_id = ?1 LIMIT 1",
                    params![s_sync],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| format!("Greska trazenja trgovine popisa: {e}"))?
            } else {
                None
            };

            conn.execute(
                "INSERT INTO shopping_lists (sync_id, name, store_id, store_sync_id, updated_at, archived)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(sync_id) DO UPDATE SET
                    name = excluded.name,
                    store_id = COALESCE(excluded.store_id, shopping_lists.store_id),
                    store_sync_id = COALESCE(excluded.store_sync_id, shopping_lists.store_sync_id),
                    updated_at = excluded.updated_at,
                    archived = excluded.archived",
                params![
                    p.sync_id,
                    p.name.trim(),
                    resolved_store_id,
                    p.store_sync_id,
                    p.updated_at,
                    i32::from(v.archived || p.archived)
                ],
            )
            .map_err(|e| format!("Greska projekcije popisa: {e}"))?;
            Ok(true)
        }
        "planned_items" => {
            let p: PlannedItemSyncPayload = serde_json::from_str(&v.payload)
                .map_err(|e| format!("Neispravan payload stavke popisa: {e}"))?;
            if p.sync_id != v.entity_id
                || p.qty_millis <= 0
                || p.estimated_price_cents < 0
                || p.name.trim().is_empty()
            {
                return Err("Neispravna stavka popisa".to_owned());
            }
            let list_id: Option<i64> = conn
                .query_row(
                    "SELECT id FROM shopping_lists WHERE sync_id = ?1",
                    params![p.list_sync_id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| format!("Greska provjere roditeljskog popisa: {e}"))?;
            let Some(lid) = list_id else {
                enqueue_pending(conn, v, "shopping_lists", &p.list_sync_id)?;
                return Ok(false);
            };
            let qty = millis_to_qty(p.qty_millis);
            conn.execute(
                "INSERT INTO planned_items (sync_id, list_id, list_sync_id, barcode, name, quantity, unit, estimated_price_cents, checked, updated_at, archived)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(sync_id) DO UPDATE SET
                    list_id = excluded.list_id,
                    list_sync_id = excluded.list_sync_id,
                    barcode = excluded.barcode,
                    name = excluded.name,
                    quantity = excluded.quantity,
                    unit = excluded.unit,
                    estimated_price_cents = excluded.estimated_price_cents,
                    checked = excluded.checked,
                    updated_at = excluded.updated_at,
                    archived = excluded.archived",
                params![
                    p.sync_id,
                    lid,
                    p.list_sync_id,
                    p.barcode.as_deref(),
                    p.name.trim(),
                    qty,
                    p.unit.as_str(),
                    p.estimated_price_cents,
                    i32::from(p.checked),
                    p.updated_at,
                    i32::from(v.archived || p.archived)
                ],
            )
            .map_err(|e| format!("Greska projekcije stavke popisa: {e}"))?;
            Ok(true)
        }
        other => Err(format!("Nepoznata vrsta entiteta: {other}")),
    }
}

fn enqueue_pending(
    conn: &Connection,
    v: &Version,
    parent_entity: &str,
    parent_sync_id: &str,
) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO pending_projections (version_id, entity, entity_id, parent_entity, parent_sync_id, payload)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            v.version_id,
            v.entity,
            v.entity_id,
            parent_entity,
            parent_sync_id,
            v.payload
        ],
    )
    .map_err(|e| format!("Greska stavljanja zavisne projekcije na cekanje: {e}"))?;
    Ok(())
}

fn flush_pending_children(
    conn: &Connection,
    parent_entity: &str,
    parent_sync_id: &str,
    network_pub: &VerifyingKey,
    now_unix: i64,
    allowed: Option<&BTreeSet<String>>,
    blocked: &mut Vec<RejectedVersion>,
) -> Result<usize, String> {
    let rows: Vec<(String, String, String, String)> = {
        let mut stmt = conn
            .prepare(
                "SELECT version_id, entity, entity_id, payload
                 FROM pending_projections
                 WHERE parent_entity = ?1 AND parent_sync_id = ?2
                 ORDER BY version_id ASC",
            )
            .map_err(|e| format!("Greska upita cekajuce projekcije: {e}"))?;
        let mapped = stmt
            .query_map(params![parent_entity, parent_sync_id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(|e| format!("Greska citanja cekajuce projekcije: {e}"))?;
        mapped
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Greska retka cekajuce projekcije: {e}"))?
    };

    let mut projected = 0usize;
    for (vid, entity, entity_id, _payload) in rows {
        let max_vid: Option<String> = conn
            .query_row(
                "SELECT MAX(version_id) FROM record_versions WHERE entity = ?1 AND entity_id = ?2 AND legacy_imported = 0",
                params![entity, entity_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| format!("Greska provjere najnovije verzije: {e}"))?
            .flatten();

        let is_winner = max_vid.as_deref() == Some(vid.as_str());
        if is_winner {
            let v: Version = conn
                .query_row(
                    "SELECT version_id, entity, entity_id, node_id, supersedes, archived,
                            payload, created_at, schema_version, channel, author_pub_key,
                            grant_ref, signature, created_at_unix
                     FROM record_versions WHERE version_id = ?1",
                    params![vid],
                    |row| {
                        let arch: i32 = row.get(5)?;
                        Ok(Version {
                            version_id: row.get(0)?,
                            entity: row.get(1)?,
                            entity_id: row.get(2)?,
                            node_id: row.get(3)?,
                            supersedes: row.get(4)?,
                            archived: arch != 0,
                            payload: row.get(6)?,
                            created_at: row.get(7)?,
                            schema_version: row.get(8)?,
                            channel: row.get(9)?,
                            author_pub_key: row.get(10)?,
                            grant_ref: row.get(11)?,
                            signature: row.get(12)?,
                            created_at_unix: row.get(13)?,
                        })
                    },
                )
                .map_err(|e| format!("Greska ucitavanja cekajuce verzije: {e}"))?;

            let authority = if allowed.is_some_and(|channels| !channels.contains(&v.channel)) {
                Err("Primatelj više nema pravo na kanal čekajuće projekcije".into())
            } else {
                verify_domain_version_auth(conn, network_pub, &v, now_unix)
            };
            if let Err(reason) = authority {
                if !blocked.iter().any(|entry| entry.version_id == v.version_id) {
                    blocked.push(RejectedVersion {
                        version_id: v.version_id.clone(),
                        author_node_id: v.node_id.clone(),
                        channel: v.channel.clone(),
                        reason,
                    });
                }
                // Retain the signed evidence and queue entry, without blocking
                // independent valid parents or silently promoting old authority.
                continue;
            }
            if validate_and_project_entity(conn, &v)? {
                projected += 1;
            } else {
                // Still waiting for a dependency: retain the durable queue entry.
                continue;
            }
        }
        conn.execute(
            "DELETE FROM pending_projections WHERE version_id = ?1",
            params![vid],
        )
        .map_err(|e| format!("Greska ciscenja cekajuce projekcije: {e}"))?;
    }
    Ok(projected)
}

/// Unified transactional admission and deterministic projection for both network sync and v2 bundles.
pub fn apply_remote_versions(
    conn: &Connection,
    context: ApplyContext<'_>,
    versions: &[Version],
    sender_holes: &BTreeMap<String, crate::ledger::VersionRange>,
    covered_ranges: &BTreeMap<String, crate::ledger::VersionRange>,
) -> Result<ApplyReport, String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("Greska pokretanja transakcije primjene: {e}"))?;
    let report =
        apply_remote_versions_in_transaction(&tx, context, versions, sender_holes, covered_ranges)?;
    tx.commit()
        .map_err(|e| format!("Greska potvrde transakcije primjene: {e}"))?;
    Ok(report)
}

/// Caller owns the transaction, including snapshot progress and final admission.
pub(crate) fn apply_remote_versions_in_transaction(
    conn: &Connection,
    context: ApplyContext<'_>,
    versions: &[Version],
    sender_holes: &BTreeMap<String, crate::ledger::VersionRange>,
    covered_ranges: &BTreeMap<String, crate::ledger::VersionRange>,
) -> Result<ApplyReport, String> {
    if conn.is_autocommit() {
        return Err("Primjena zahtijeva transakciju".into());
    }
    let ApplyContext {
        network_pub,
        peer_key,
        recipient_b64,
        now_unix,
    } = context;
    let tx = conn;

    let mut report = ApplyReport::default();
    let initial_frontier = crate::ledger::get_frontier(tx)?;

    // 1. Always process control records BEFORE domain records in the same batch
    let mut control_versions: Vec<&Version> =
        versions.iter().filter(|v| v.channel == "control").collect();
    let mut domain_versions: Vec<&Version> =
        versions.iter().filter(|v| v.channel != "control").collect();

    // Authority proofs precede their delegated envelopes even when a relay
    // published the authority's wrapper later. Revocations of authority or
    // device keys precede delegated revocations and certificates, and all
    // control records precede domain data.
    let net_b64 = public_key_to_base64(network_pub.as_bytes());
    let control_rank = |v: &Version| match serde_json::from_str::<ControlPayload>(&v.payload) {
        Ok(ControlPayload::Ovlast(_)) => 0,
        Ok(ControlPayload::Revocation(r))
            if r.scope() == Ok("ovlast")
                || r.signer_key == net_b64
                || v.author_pub_key == net_b64 =>
        {
            1
        }
        Ok(ControlPayload::Revocation(_)) => 2,
        _ => 3,
    };
    control_versions.sort_by(|a, b| {
        control_rank(a)
            .cmp(&control_rank(b))
            .then_with(|| a.version_id.cmp(&b.version_id))
    });
    domain_versions.sort_by(|a, b| a.version_id.cmp(&b.version_id));

    for v in control_versions {
        if let Some(existing) = load_existing_version(tx, &v.version_id)? {
            if !versions_identical(&existing, v) {
                report.rejected.push(RejectedVersion {
                    version_id: v.version_id.clone(),
                    author_node_id: v.node_id.clone(),
                    channel: v.channel.clone(),
                    reason: "Sukob sadrzaja za isti version_id".to_owned(),
                });
            }
            continue;
        }
        match verify_control_version_envelope(v, network_pub, tx, now_unix) {
            Ok(ctrl) => {
                if let Err(err) = store_control_record(tx, network_pub, &ctrl, now_unix) {
                    if tx.is_autocommit() {
                        return Err(err);
                    }
                    report.rejected.push(RejectedVersion {
                        version_id: v.version_id.clone(),
                        author_node_id: v.node_id.clone(),
                        channel: v.channel.clone(),
                        reason: err,
                    });
                    continue;
                }
                insert_remote_version_row(tx, v)?;
                report.applied_versions += 1;
                report.applied_ids.push(v.version_id.clone());
            }
            Err(reason) => {
                report.rejected.push(RejectedVersion {
                    version_id: v.version_id.clone(),
                    author_node_id: v.node_id.clone(),
                    channel: v.channel.clone(),
                    reason,
                });
            }
        }
    }

    // 2. Record any sender-declared holes BEFORE applying domain versions
    for (fkey, range) in sender_holes {
        let (author, ch) = crate::ledger::split_frontier_key(fkey);
        record_hole(tx, author, ch, &range.from_id, &range.to_id)?;
    }

    let recipient_allowed = match recipient_b64 {
        Some(rb64) if rb64 == public_key_to_base64(network_pub.as_bytes()) => None,
        Some(rb64) => Some(allowed_channels_for_recipient(
            tx,
            network_pub,
            rb64,
            now_unix,
        )?),
        None => None,
    };

    // 3. Apply domain versions deterministically
    for v in domain_versions {
        if let Some(ref allowed) = recipient_allowed
            && !allowed.contains(&v.channel)
        {
            report.rejected.push(RejectedVersion {
                version_id: v.version_id.clone(),
                author_node_id: v.node_id.clone(),
                channel: v.channel.clone(),
                reason: "Primatelj nema pravo pristupa ovom kanalu".to_owned(),
            });
            continue;
        }
        if let Some(existing) = load_existing_version(tx, &v.version_id)? {
            if !versions_identical(&existing, v) {
                report.rejected.push(RejectedVersion {
                    version_id: v.version_id.clone(),
                    author_node_id: v.node_id.clone(),
                    channel: v.channel.clone(),
                    reason: "Sukob sadrzaja za isti version_id".to_owned(),
                });
            }
            continue;
        }

        if let Err(reason) = verify_domain_version_auth(tx, network_pub, v, now_unix) {
            record_rejected_range(tx, peer_key, &v.node_id, &v.channel, &v.version_id)?;
            report.rejected.push(RejectedVersion {
                version_id: v.version_id.clone(),
                author_node_id: v.node_id.clone(),
                channel: v.channel.clone(),
                reason,
            });
            continue;
        }

        // Validate payload syntax & numbers BEFORE inserting into record_versions
        if let Err(reason) =
            validate_payload_syntax_only(v).and_then(|()| validate_identity_binding(tx, v))
        {
            record_rejected_range(tx, peer_key, &v.node_id, &v.channel, &v.version_id)?;
            report.rejected.push(RejectedVersion {
                version_id: v.version_id.clone(),
                author_node_id: v.node_id.clone(),
                channel: v.channel.clone(),
                reason,
            });
            continue;
        }

        let different_channel: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM record_versions WHERE entity = ?1 AND entity_id = ?2
             AND channel != ?3 AND signature != '')",
                params![v.entity, v.entity_id, v.channel],
                |r| r.get(0),
            )
            .map_err(|e| format!("Greska provjere kanala identiteta: {e}"))?;
        if different_channel {
            report.rejected.push(RejectedVersion {
                version_id: v.version_id.clone(),
                author_node_id: v.node_id.clone(),
                channel: v.channel.clone(),
                reason: "Identitet zapisa vec pripada drugom kanalu".into(),
            });
            continue;
        }

        // Check for missing supersedes (hole detection when an indirect peer skipped an earlier version)
        if !v.supersedes.is_empty() {
            let has_prev: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM record_versions WHERE version_id = ?1)",
                    params![v.supersedes],
                    |r| r.get(0),
                )
                .map_err(|e| format!("Greska provjere prethodne verzije: {e}"))?;
            if !has_prev {
                let prev_local: String = tx
                    .query_row(
                        "SELECT COALESCE(MAX(version_id), '') FROM record_versions
                         WHERE node_id = ?1 AND channel = ?2 AND version_id < ?3",
                        params![v.node_id, v.channel, v.supersedes],
                        |r| r.get(0),
                    )
                    .unwrap_or_default();
                record_hole(tx, &v.node_id, &v.channel, &prev_local, &v.version_id)?;
            }
        }

        insert_remote_version_row(tx, v)?;
        report.applied_versions += 1;
        report.applied_ids.push(v.version_id.clone());

        let max_vid: Option<String> = tx
            .query_row(
                "SELECT MAX(version_id) FROM record_versions
                 WHERE entity = ?1 AND entity_id = ?2 AND legacy_imported = 0",
                params![v.entity, v.entity_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| format!("Greska provjere najnovijeg version_id: {e}"))?
            .flatten();

        if max_vid.as_deref() == Some(v.version_id.as_str()) {
            match validate_and_project_entity(tx, v)? {
                true => {
                    report.projected_records += 1;
                }
                false => {
                    report.deferred_children += 1;
                }
            }
        }
    }

    // A dependency chain can be store -> receipt -> receipt_item. Drain until no
    // further projection is possible; each successful pass removes a queued row.
    loop {
        let parents: Vec<(String, String)> = {
            let mut stmt = tx
                .prepare("SELECT DISTINCT parent_entity,parent_sync_id FROM pending_projections")
                .map_err(|e| e.to_string())?;
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?
        };
        let mut progress = 0;
        for (entity, id) in parents {
            progress += flush_pending_children(
                tx,
                &entity,
                &id,
                network_pub,
                now_unix,
                recipient_allowed.as_ref(),
                &mut report.blocked_projections,
            )?;
        }
        report.projected_records += progress;
        if progress == 0 {
            break;
        }
    }

    // An arrival can be below another peer's MAX even when above our own MAX.
    // Invalidate on every new admitted remote row; a duplicate changes nothing.
    // Local authoring keeps its ordinary delta path. Relays conservatively use
    // a snapshot rather than claiming knowledge they cannot prove for a peer.
    let admitted: std::collections::BTreeSet<_> = report.applied_ids.iter().collect();
    let backfilled: std::collections::BTreeSet<_> = versions
        .iter()
        .filter(|v| admitted.contains(&v.version_id))
        .map(|v| v.channel.clone())
        .collect();
    crate::backfill::mark_channels(tx, &backfilled)?;

    // Receipt of a high UUID is not proof of the preceding interval, including
    // independent records without a supersedes link. Keep it open until a valid
    // complete page covers it. All bookkeeping shares the projection transaction.
    for v in versions {
        if load_existing_version(tx, &v.version_id)?
            .is_some_and(|stored| versions_identical(&stored, v))
        {
            let key = crate::ledger::frontier_key(&v.node_id, &v.channel);
            let from = initial_frontier.get(&key).map(String::as_str).unwrap_or("");
            record_hole(tx, &v.node_id, &v.channel, from, &v.version_id)?;
        }
    }
    let rejected_streams: std::collections::BTreeSet<_> = report
        .rejected
        .iter()
        .map(|v| crate::ledger::frontier_key(&v.author_node_id, &v.channel))
        .collect();
    for rejected in &report.rejected {
        record_rejected_range(
            tx,
            peer_key,
            &rejected.author_node_id,
            &rejected.channel,
            &rejected.version_id,
        )?;
        // A late rejected version below an already observed maximum must also
        // reopen that stream; otherwise MAX would still hide it from another peer.
        if uuid::Uuid::parse_str(&rejected.version_id)
            .is_ok_and(|id| id.get_version_num() == 7 && id.to_string() == rejected.version_id)
        {
            let has_higher: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM record_versions WHERE node_id=?1 AND channel=?2 AND version_id>?3 AND signature!='' AND legacy_imported=0)",
                    params![rejected.author_node_id, rejected.channel, rejected.version_id],
                    |r| r.get(0),
                )
                .map_err(|e| format!("Greska granice odbijene verzije: {e}"))?;
            if has_higher {
                let previous: String = tx.query_row(
                    "SELECT COALESCE(MAX(version_id), '') FROM record_versions WHERE node_id=?1 AND channel=?2 AND version_id<?3",
                    params![rejected.author_node_id, rejected.channel, rejected.version_id], |r| r.get(0),
                ).map_err(|e| format!("Greska granice odbijene verzije: {e}"))?;
                record_hole(
                    tx,
                    &rejected.author_node_id,
                    &rejected.channel,
                    &previous,
                    &rejected.version_id,
                )?;
            }
        }
    }

    // A sender's range assertion cannot acknowledge rejected data, an absent
    // endpoint, a forbidden channel or an interval the sender itself lacks.
    for (fkey, range) in covered_ranges {
        let (author, ch) = crate::ledger::split_frontier_key(fkey);
        if range.to_id <= range.from_id
            || rejected_streams.contains(fkey)
            || recipient_allowed
                .as_ref()
                .is_some_and(|allowed| !allowed.contains(ch))
            || sender_holes
                .get(fkey)
                .is_some_and(|hole| hole.from_id < range.to_id && hole.to_id > range.from_id)
            || !versions
                .iter()
                .any(|v| v.node_id == author && v.channel == ch && v.version_id == range.to_id)
        {
            continue;
        }
        let endpoint = load_existing_version(tx, &range.to_id)?;
        if !endpoint
            .is_some_and(|v| v.node_id == author && v.channel == ch && !v.signature.is_empty())
        {
            continue;
        }
        narrow_holes(tx, author, ch, &range.from_id, &range.to_id)?;
        let hole_floor: Option<String> = tx
            .query_row(
                "SELECT MIN(from_version_id) FROM sync_holes WHERE author_node_id = ?1 AND channel = ?2",
                params![author, ch],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| format!("Greska provjere preostalih rupa: {e}"))?
            .flatten();
        let ack_target = match hole_floor {
            Some(hf) if hf < range.to_id => hf,
            _ => range.to_id.clone(),
        };
        update_peer_ack(tx, peer_key, author, ch, &ack_target, &range.to_id)?;
    }

    Ok(report)
}

pub(crate) fn validate_payload_syntax_only(v: &Version) -> Result<(), String> {
    if let Some(list_id) = v.channel.strip_prefix("list:") {
        match v.entity.as_str() {
            "shopping_lists" if v.entity_id == list_id => {}
            "planned_items" => {
                let p: PlannedItemSyncPayload = serde_json::from_str(&v.payload)
                    .map_err(|e| format!("Neispravna stavka: {e}"))?;
                if p.list_sync_id != list_id {
                    return Err("Stavka ne pripada dopustenom popisu".into());
                }
            }
            _ => return Err("Entitet ne pripada kanalu popisa".into()),
        }
    } else if !v.channel.starts_with("private:") {
        return Err("Nepoznat domenski kanal".into());
    }
    match v.entity.as_str() {
        "stores" => {
            let p: StoreSyncPayload = serde_json::from_str(&v.payload)
                .map_err(|e| format!("Neispravan payload trgovine: {e}"))?;
            if p.sync_id != v.entity_id || p.name.trim().is_empty() {
                return Err("Neispravna polja trgovine".to_owned());
            }
        }
        "products" => {
            let p: ProductSyncPayload = serde_json::from_str(&v.payload)
                .map_err(|e| format!("Neispravan payload artikla: {e}"))?;
            if p.sync_id != v.entity_id
                || p.barcode.trim().is_empty()
                || p.name.trim().is_empty()
                || !(0..=MAX_SAFE_CENTS).contains(&p.last_price_cents)
            {
                return Err("Neispravna polja ili cijena artikla".to_owned());
            }
        }
        "price_history" => {
            let p: PriceHistorySyncPayload = serde_json::from_str(&v.payload)
                .map_err(|e| format!("Neispravan payload cijene: {e}"))?;
            if p.sync_id != v.entity_id
                || !(0..=MAX_SAFE_CENTS).contains(&p.price_cents)
                || p.barcode.trim().is_empty()
            {
                return Err("Neispravan zapis povijesti cijene".to_owned());
            }
        }
        "receipts" => {
            let p: ReceiptSyncPayload = serde_json::from_str(&v.payload)
                .map_err(|e| format!("Neispravan payload racuna: {e}"))?;
            if let Some(ids) = &p.item_ids {
                let unique: BTreeSet<_> = ids.iter().collect();
                if unique.len() != ids.len()
                    || ids.iter().any(|id| id.trim().is_empty())
                    || ids.len() > 10000
                {
                    return Err("Neispravan popis stavki racuna".into());
                }
            }
            if p.sync_id != v.entity_id
                || !(0..=MAX_SAFE_CENTS).contains(&p.cart_total_cents)
                || !(0..=MAX_SAFE_CENTS).contains(&p.receipt_total_cents)
                || p.difference_cents != p.cart_total_cents - p.receipt_total_cents
            {
                return Err("Neusklađeni zbrojevi racuna".to_owned());
            }
        }
        "receipt_items" => {
            let p: ReceiptItemSyncPayload = serde_json::from_str(&v.payload)
                .map_err(|e| format!("Neispravan payload stavke racuna: {e}"))?;
            let expected = checked_line_total_millis(p.qty_millis, p.unit, p.unit_price_cents)?;
            if p.sync_id != v.entity_id
                || p.qty_millis <= 0
                || p.unit_price_cents < 0
                || p.total_cents != expected
            {
                return Err("Neispravna kolicina ili total stavke racuna".to_owned());
            }
        }
        "shopping_lists" => {
            let p: ShoppingListSyncPayload = serde_json::from_str(&v.payload)
                .map_err(|e| format!("Neispravan payload popisa: {e}"))?;
            if p.sync_id != v.entity_id || p.name.trim().is_empty() {
                return Err("Neispravan naziv ili ID popisa".to_owned());
            }
        }
        "planned_items" => {
            let p: PlannedItemSyncPayload = serde_json::from_str(&v.payload)
                .map_err(|e| format!("Neispravan payload stavke popisa: {e}"))?;
            checked_line_total_millis(p.qty_millis, p.unit, p.estimated_price_cents)?;
            if p.sync_id != v.entity_id
                || p.qty_millis <= 0
                || p.estimated_price_cents < 0
                || p.name.trim().is_empty()
            {
                return Err("Neispravna stavka popisa".to_owned());
            }
        }
        other => return Err(format!("Nepoznat entitet: {other}")),
    }
    Ok(())
}

/// A signed header is complete only when its exact active item set and sum agree.
pub(crate) fn receipt_is_complete(conn: &Connection, id: &str) -> Result<bool, String> {
    let header: Option<String> = conn.query_row(
        "SELECT payload FROM record_versions WHERE entity='receipts' AND entity_id=?1 AND signature!='' AND legacy_imported=0 ORDER BY version_id DESC LIMIT 1",[id],|r|r.get(0))
        .optional().map_err(|e|e.to_string())?;
    let Some(header) = header else {
        return Ok(true);
    };
    let header: ReceiptSyncPayload = serde_json::from_str(&header).map_err(|e| e.to_string())?;
    let Some(expected) = header.item_ids else {
        return Ok(false);
    };
    let expected: BTreeSet<_> = expected.into_iter().collect();
    let mut stmt = conn.prepare("SELECT sync_id,total_cents FROM receipt_items WHERE receipt_id=(SELECT id FROM receipts WHERE sync_id=?1) AND archived=0 ORDER BY sync_id").map_err(|e|e.to_string())?;
    let rows = stmt
        .query_map([id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
        .map_err(|e| e.to_string())?;
    let mut actual = BTreeSet::new();
    let mut total = 0i64;
    for row in rows {
        let (id, cents) = row.map_err(|e| e.to_string())?;
        actual.insert(id);
        let Some(sum) = total.checked_add(cents) else {
            return Ok(false);
        };
        total = sum;
        if !(0..=MAX_SAFE_CENTS).contains(&total) {
            return Ok(false);
        }
    }
    Ok(actual == expected && total == header.cart_total_cents)
}

fn load_existing_version(conn: &Connection, vid: &str) -> Result<Option<Version>, String> {
    conn.query_row(
        "SELECT version_id, entity, entity_id, node_id, supersedes, archived,
                payload, created_at, schema_version, channel, author_pub_key,
                grant_ref, signature, created_at_unix
         FROM record_versions WHERE version_id = ?1",
        params![vid],
        |row| {
            let arch: i32 = row.get(5)?;
            Ok(Version {
                version_id: row.get(0)?,
                entity: row.get(1)?,
                entity_id: row.get(2)?,
                node_id: row.get(3)?,
                supersedes: row.get(4)?,
                archived: arch != 0,
                payload: row.get(6)?,
                created_at: row.get(7)?,
                schema_version: row.get(8)?,
                channel: row.get(9)?,
                author_pub_key: row.get(10)?,
                grant_ref: row.get(11)?,
                signature: row.get(12)?,
                created_at_unix: row.get(13)?,
            })
        },
    )
    .optional()
    .map_err(|e| format!("Greska provjere postojece verzije: {e}"))
}

fn versions_identical(a: &Version, b: &Version) -> bool {
    a.version_id == b.version_id
        && a.entity == b.entity
        && a.entity_id == b.entity_id
        && a.node_id == b.node_id
        && a.channel == b.channel
        && a.archived == b.archived
        && a.payload == b.payload
        && a.signature == b.signature
}

fn insert_remote_version_row(conn: &Connection, v: &Version) -> Result<(), String> {
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO record_versions
         (version_id, entity, entity_id, node_id, supersedes, archived, payload, created_at, schema_version, channel, author_pub_key, grant_ref, signature, created_at_unix, applied_at, legacy_imported)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, 0)",
        params![
            v.version_id,
            v.entity,
            v.entity_id,
            v.node_id,
            v.supersedes,
            i32::from(v.archived),
            v.payload,
            v.created_at,
            v.schema_version,
            v.channel,
            v.author_pub_key,
            v.grant_ref,
            v.signature,
            v.created_at_unix,
            now
        ],
    )
    .map_err(|e| format!("Greska pohrane primljene verzije: {e}"))?;
    Ok(())
}

/// Computes the exact set of channels permitted for a specific recipient public key.
/// Never computes a union of all family members' grants.
pub fn allowed_channels_for_recipient(
    conn: &Connection,
    network_pub: &VerifyingKey,
    recipient_pub_b64: &str,
    now_unix: i64,
) -> Result<BTreeSet<String>, String> {
    let mut channels = BTreeSet::new();
    let net_b64 = public_key_to_base64(network_pub.as_bytes());

    let revoked: bool = conn
        .query_row(
            "SELECT EXISTS(
                SELECT 1 FROM sync_trust_records
                WHERE kind = 'revocation' AND subject_key = ?1 AND permission IN ('membership','legacy_key')
            )",
            params![recipient_pub_b64],
            |r| r.get(0),
        )
        .map_err(|e| format!("Greska provjere opoziva primatelja: {e}"))?;
    if revoked {
        return Ok(channels);
    }

    let is_owner_root = recipient_pub_b64 == net_b64;
    let cached_membership_blocked:bool=conn.query_row("SELECT EXISTS(SELECT 1 FROM sync_trust_records WHERE kind='membership' AND subject_key=?1 AND revoked!=0)",[recipient_pub_b64],|r|r.get(0)).map_err(|e|e.to_string())?;
    let membership: Option<Option<(String, String)>> = if is_owner_root {
        Some(Some((
            "own_device".to_owned(),
            syncnet::identity::principal_id_from_key(network_pub.as_bytes()),
        )))
    } else if cached_membership_blocked {
        None
    } else if crate::acl::historical_membership(conn, network_pub, recipient_pub_b64, now_unix)?
        .is_some()
    {
        let role_info =
            crate::acl::historical_member_role(conn, network_pub, recipient_pub_b64, now_unix)?
                .map(|r| {
                    (
                        match r.role {
                            MemberRole::OwnDevice => "own_device",
                            MemberRole::FamilyMember => "family_member",
                        }
                        .to_owned(),
                        r.principal_id,
                    )
                });
        Some(role_info)
    } else {
        None
    };

    let Some(role_info) = membership else {
        return Ok(channels);
    };

    channels.insert("control".to_owned());

    if let Some((role, principal_id)) = role_info
        && role == "own_device"
    {
        channels.insert(format!("private:{principal_id}"));
    }
    if is_owner_root {
        let mut stmt = conn
            .prepare("SELECT DISTINCT channel FROM record_versions WHERE channel LIKE 'list:%'")
            .map_err(|e| format!("Greska citanja lista za own_device: {e}"))?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| format!("Greska mapiranja kanala: {e}"))?;
        for ch in rows {
            channels.insert(ch.map_err(|e| format!("Greska retka kanala: {e}"))?);
        }
    }

    // Add every explicit, active, non-revoked ListGrant targeting THIS specific recipient key
    let mut g_stmt = conn
        .prepare(
            "SELECT target_id,raw_json FROM sync_trust_records
             WHERE kind = 'list_grant' AND subject_key = ?1 AND revoked = 0 AND issued_at <= ?2
               AND (expires_at = 0 OR expires_at >= ?2)",
        )
        .map_err(|e| format!("Greska citanja grantova primatelja: {e}"))?;
    let g_rows = g_stmt
        .query_map(params![recipient_pub_b64, now_unix], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })
        .map_err(|e| format!("Greska mapiranja grantova primatelja: {e}"))?;
    for list_id in g_rows {
        let (lid, raw) = list_id.map_err(|e| format!("Greska retka granta: {e}"))?;
        let ControlPayload::ListGrant(grant) =
            serde_json::from_str(&raw).map_err(|e| format!("Neispravan dokaz granta: {e}"))?
        else {
            return Err("Neispravan tip dokaza granta".into());
        };
        let recipient = parse_public_key(recipient_pub_b64).map_err(|e| e.to_string())?;
        if grant.list_sync_id == lid
            && grant.verify(network_pub, &recipient, now_unix).is_ok()
            && crate::acl::check_delegation_at(
                conn,
                network_pub,
                grant.admitter.as_ref(),
                grant.issued_at,
            )
            .is_ok()
            && !crate::acl::revoked(conn, "list_grant", recipient_pub_b64, &grant.grant_id)?
        {
            channels.insert(format!("list:{lid}"));
        }
    }

    Ok(channels)
}

pub fn export_protected_bundle_for_recipient(
    conn: &Connection,
    request: ExportRequest<'_>,
) -> Result<ProtectedBundleV2, String> {
    if !conn.is_autocommit() {
        return export_protected_bundle_in_snapshot(conn, request);
    }
    // Rights, proof selection and page data must describe one database snapshot.
    let transaction = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let bundle = export_protected_bundle_in_snapshot(&transaction, request)?;
    transaction.commit().map_err(|e| e.to_string())?;
    Ok(bundle)
}

fn export_protected_bundle_in_snapshot(
    conn: &Connection,
    request: ExportRequest<'_>,
) -> Result<ProtectedBundleV2, String> {
    let ExportRequest {
        local_node_id,
        sender_sk,
        network_pub,
        recipient_pub,
        peer_frontier,
        requested_holes,
        max_bytes,
        now_unix,
    } = request;
    let recipient_b64 = public_key_to_base64(recipient_pub.as_bytes());
    let mut allowed = allowed_channels_for_recipient(conn, network_pub, &recipient_b64, now_unix)?;

    let sender_public = public_key_to_base64(sender_sk.verifying_key().as_bytes());
    let budget = crate::ledger::delta_byte_budget(max_bytes);
    let wrapper_bytes = crate::ledger::serialized_json_len(&serde_json::json!({
        "sender_node_id": local_node_id, "sender_pub_key": sender_public, "batch": null
    }))? - 4; // `null` is replaced by the serialized batch.
    let batch_budget = budget
        .checked_sub(wrapper_bytes)
        .filter(|n| *n > 0)
        .ok_or("Granica stranice je premala za zaglavlje paketa")?;
    let sender_allowed =
        allowed_channels_for_recipient(conn, network_pub, &sender_public, now_unix)?;
    let same_principal = allowed
        .iter()
        .any(|channel| channel.starts_with("private:") && sender_allowed.contains(channel));
    let batch = if recipient_b64 == public_key_to_base64(network_pub.as_bytes()) || same_principal {
        let mut relayable_ids: BTreeSet<String> = if allowed.contains("control") {
            relayable_control_versions(conn, network_pub, now_unix)?
                .into_iter()
                .map(|v| v.version_id)
                .collect()
        } else {
            BTreeSet::new()
        };
        relayable_ids.extend(
            relayable_domain_versions(conn, network_pub, &allowed, now_unix)?
                .into_iter()
                .map(|v| v.version_id),
        );
        compute_delta_paged_filtered(
            conn,
            peer_frontier,
            requested_holes,
            &allowed,
            batch_budget,
            Some(&relayable_ids),
        )?
    } else {
        allowed.retain(|channel| !channel.starts_with("private:"));
        crate::family::page(
            conn,
            network_pub,
            &recipient_b64,
            &public_key_to_base64(network_pub.as_bytes()),
            peer_frontier,
            requested_holes,
            &allowed,
            batch_budget,
            now_unix,
        )?
    };

    let inner = ProtectedSyncPayload {
        sender_node_id: local_node_id.to_owned(),
        sender_pub_key: sender_public,
        batch,
    };
    let plaintext =
        serde_json::to_vec(&inner).map_err(|e| format!("Greska serijalizacije v2 paketa: {e}"))?;
    if plaintext.len() > budget {
        return Err("Zaštićeni sadržaj prelazi granicu stranice".into());
    }

    ProtectedBundleV2::seal(
        PROTOCOL_NAME,
        sender_sk,
        recipient_pub.as_bytes(),
        &plaintext,
    )
    .map_err(|e| format!("Greska sifririranja v2 paketa: {e}"))
}

pub fn import_protected_bundle(
    conn: &Connection,
    recipient_sk: &SigningKey,
    network_pub: &VerifyingKey,
    local_principal_id: &str,
    bundle: &ProtectedBundleV2,
    now_unix: i64,
) -> Result<ApplyReport, String> {
    let (plaintext, sender_pub_bytes) = bundle
        .open(PROTOCOL_NAME, recipient_sk)
        .map_err(|e| format!("Odbijen v2 paket: {e}"))?;
    let payload: ProtectedSyncPayload = serde_json::from_slice(&plaintext)
        .map_err(|e| format!("Neispravan sadrzaj v2 paketa: {e}"))?;

    let sender_b64 = public_key_to_base64(&sender_pub_bytes);
    if payload.sender_pub_key != sender_b64 {
        return Err("Neusklađen kljuc posiljatelja u v2 paketu".to_owned());
    }

    let mut all_versions = Vec::new();
    if !payload.batch.snapshot_versions.is_empty() {
        all_versions.extend(payload.batch.snapshot_versions);
    }
    all_versions.extend(payload.batch.versions);

    let sender_is_own_device =
        sender_is_own_principal(conn, network_pub, &sender_b64, local_principal_id, now_unix)?;

    let mut pre_rejected = Vec::new();
    let mut filtered_versions = Vec::with_capacity(all_versions.len());
    for v in all_versions {
        if let Some(target_principal) = v.channel.strip_prefix("private:")
            && (!sender_is_own_device || target_principal != local_principal_id)
        {
            pre_rejected.push(RejectedVersion {
                version_id: v.version_id,
                author_node_id: v.node_id,
                channel: v.channel,
                reason:
                    "Nedopusten prijenos privatnog kanala preko tudjeg ili neovlastenog uredjaja"
                        .to_owned(),
            });
            continue;
        }
        filtered_versions.push(v);
    }

    let recipient_b64 = public_key_to_base64(recipient_sk.verifying_key().as_bytes());
    // A version filtered before admission must not still acknowledge its stream.
    let mut covered_ranges = payload.batch.covered_ranges;
    let mut sender_holes = payload.batch.sender_holes;
    if !sender_is_own_device {
        // Family proofs are a replayed subset, never a complete control stream.
        covered_ranges.retain(|key, _| crate::ledger::split_frontier_key(key).1 != "control");
        sender_holes.retain(|key, _| crate::ledger::split_frontier_key(key).1 != "control");
    }
    for rejected in &pre_rejected {
        covered_ranges.remove(&crate::ledger::frontier_key(
            &rejected.author_node_id,
            &rejected.channel,
        ));
    }
    let mut report = apply_remote_versions(
        conn,
        ApplyContext {
            network_pub,
            peer_key: &sender_b64,
            recipient_b64: Some(&recipient_b64),
            now_unix,
        },
        &filtered_versions,
        &sender_holes,
        &covered_ranges,
    )?;
    report.rejected.extend(pre_rejected);
    Ok(report)
}

pub(crate) fn sender_is_own_principal(
    conn: &Connection,
    network_pub: &VerifyingKey,
    sender_b64: &str,
    local_principal_id: &str,
    now_unix: i64,
) -> Result<bool, String> {
    Ok(
        allowed_channels_for_recipient(conn, network_pub, sender_b64, now_unix)?
            .contains(&format!("private:{local_principal_id}")),
    )
}

pub fn current_confirmed_frontier(
    conn: &Connection,
    peer_pub_b64: &str,
) -> Result<BTreeMap<String, String>, String> {
    get_confirmed_frontier(conn, peer_pub_b64)
}
