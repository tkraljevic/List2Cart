use crate::{
    acl_types::{
        ListGrant, ListPermission, MemberRole, MemberRoleRecord, issue_list_grant,
        issue_member_role,
    },
    apply::{store_control_record, verify_domain_version_auth},
    ledger::{PeerRole, Version, record_signed_version},
    models::ControlPayload,
};
use chrono::Utc;
use ed25519_dalek::{SigningKey, VerifyingKey};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
};
use syncnet::{
    identity::{parse_public_key, principal_id_from_key, public_key_to_base64},
    pairing::{PairHello, PairIdentity, PairingSessionSlot, reject_pending_in_background},
    trust::{Membership, NetworkKey, Ovlast, Validity},
};

use tokio_util::sync::CancellationToken;

pub const DEFAULT_MEMBERSHIP_VALIDITY_SECS: i64 = 365 * 24 * 3600;

/// Credentials embedded in a device's TLS 1.3 X.509 certificate SAN (`urn:list2cart:vjerodajnice:...`).
/// Allows an admitter or peer to verify and recover a unilaterally completed pairing on the next connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeCredentials {
    pub membership: Membership,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member_role: Option<MemberRoleRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ovlast: Option<Ovlast>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grants: Vec<ListGrant>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub control_versions: Vec<Version>,
}

/// Welcome package exchanged inside `PairConfirm` only when both peers approve the SAS code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WelcomePack {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub network_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub network_key: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub principal_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mine: Option<Membership>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub my_role: Option<MemberRoleRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub for_you: Option<Membership>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role_for_you: Option<MemberRoleRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ovlast: Option<Ovlast>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grants: Vec<ListGrant>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub control_versions: Vec<Version>,
}

/// Public, secret-free pairing status returned to Tauri / UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PairStatusView {
    pub waiting: bool,
    pub pending: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub listen_port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sas: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_device_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_public_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub can_admit: bool,
}

/// Public, secret-free outcome of a pairing decision returned to Tauri / UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairOutcomeView {
    pub paired: bool,
    pub member: bool,
    pub admitted: bool,
    pub peer_public_key: String,
    pub message: String,
}

pub struct PairingContext<'a> {
    pub conn: &'a Connection,
    pub device_key: &'a SigningKey,
    pub network_pub: &'a VerifyingKey,
    pub network_key: Option<&'a SigningKey>,
    pub principal: &'a str,
    pub network_name: &'a str,
}

fn load_active_membership(
    conn: &Connection,
    network_pub: &VerifyingKey,
    subject_b64: &str,
    now_unix: i64,
) -> Result<Option<Membership>, String> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT raw_json FROM sync_trust_records
             WHERE kind = 'membership' AND subject_key = ?1 AND revoked = 0
               AND issued_at <= ?2 AND (expires_at = 0 OR expires_at >= ?2)
             ORDER BY issued_at DESC LIMIT 1",
            params![subject_b64, now_unix],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| format!("Greska citanja clanstva: {e}"))?;
    let Some(raw) = raw else {
        return Ok(None);
    };
    let ControlPayload::Membership(m) =
        serde_json::from_str(&raw).map_err(|e| format!("Neispravno spremljeno clanstvo: {e}"))?
    else {
        return Ok(None);
    };
    let vk = parse_public_key(subject_b64).map_err(|e| e.to_string())?;
    if m.verify(network_pub, &vk, now_unix).is_err()
        || crate::acl::check_delegation_at(conn, network_pub, m.admitter.as_ref(), m.issued_at)
            .is_err()
        || crate::acl::revoked(conn, "membership", subject_b64, &m.device_id)?
    {
        return Ok(None);
    }
    Ok(Some(m))
}

fn load_active_member_role(
    conn: &Connection,
    network_pub: &VerifyingKey,
    subject_b64: &str,
    now_unix: i64,
) -> Result<Option<MemberRoleRecord>, String> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT raw_json FROM sync_trust_records
             WHERE kind = 'member_role' AND subject_key = ?1 AND revoked = 0
               AND issued_at <= ?2 AND (expires_at = 0 OR expires_at >= ?2)
             ORDER BY issued_at DESC LIMIT 1",
            params![subject_b64, now_unix],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| format!("Greska citanja uloge clanstva: {e}"))?;
    let Some(raw) = raw else {
        return Ok(None);
    };
    let ControlPayload::MemberRole(r) = serde_json::from_str(&raw)
        .map_err(|e| format!("Neispravna spremljena uloga clanstva: {e}"))?
    else {
        return Ok(None);
    };
    let vk = parse_public_key(subject_b64).map_err(|e| e.to_string())?;
    if r.verify(network_pub, &vk, now_unix).is_err()
        || crate::acl::check_delegation_at(conn, network_pub, r.admitter.as_ref(), r.issued_at)
            .is_err()
        || crate::acl::revoked(conn, "membership", subject_b64, &r.device_id)?
    {
        return Ok(None);
    }
    Ok(Some(r))
}

fn load_active_ovlast(
    conn: &Connection,
    network_pub: &VerifyingKey,
    subject_b64: &str,
    now_unix: i64,
) -> Result<Option<Ovlast>, String> {
    let raw: Option<String> = conn
        .query_row(
            "SELECT raw_json FROM sync_trust_records
             WHERE kind = 'ovlast' AND subject_key = ?1 AND revoked = 0
               AND issued_at <= ?2 AND (expires_at = 0 OR expires_at >= ?2)
             ORDER BY issued_at DESC LIMIT 1",
            params![subject_b64, now_unix],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| format!("Greska citanja ovlasti: {e}"))?;
    let Some(raw) = raw else {
        return Ok(None);
    };
    let ControlPayload::Ovlast(o) =
        serde_json::from_str(&raw).map_err(|e| format!("Neispravna spremljena ovlast: {e}"))?
    else {
        return Ok(None);
    };
    if o.verify(network_pub, now_unix).is_err()
        || crate::acl::revoked(conn, "ovlast", subject_b64, &o.device_id)?
    {
        return Ok(None);
    }
    Ok(Some(o))
}

fn load_control_version_for_entity(
    conn: &Connection,
    entity: &str,
    entity_id: &str,
) -> Result<Option<Version>, String> {
    conn.query_row(
        "SELECT version_id, entity, entity_id, node_id, supersedes, archived, payload, created_at,
                schema_version, channel, author_pub_key, grant_ref, signature, created_at_unix
         FROM record_versions
         WHERE channel = 'control' AND entity = ?1 AND entity_id = ?2 AND signature != ''
         ORDER BY version_id DESC LIMIT 1",
        params![entity, entity_id],
        |row| {
            Ok(Version {
                version_id: row.get(0)?,
                entity: row.get(1)?,
                entity_id: row.get(2)?,
                node_id: row.get(3)?,
                supersedes: row.get(4)?,
                archived: row.get::<_, i32>(5)? != 0,
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
    .map_err(|e| format!("Greska citanja kontrolne verzije: {e}"))
}

pub fn build_tls_credentials(ctx: &PairingContext<'_>) -> Result<Option<Vec<u8>>, String> {
    let now = Utc::now().timestamp();
    let device_b64 = public_key_to_base64(ctx.device_key.verifying_key().as_bytes());
    let Some(membership) = load_active_membership(ctx.conn, ctx.network_pub, &device_b64, now)?
    else {
        return Ok(None);
    };
    let ovlast = load_active_ovlast(ctx.conn, ctx.network_pub, &device_b64, now)?;

    let mut grants = Vec::new();
    let mut stmt = ctx
        .conn
        .prepare(
            "SELECT raw_json FROM sync_trust_records
             WHERE kind = 'list_grant' AND subject_key = ?1 AND revoked = 0
               AND issued_at <= ?2 AND (expires_at = 0 OR expires_at >= ?2)",
        )
        .map_err(|e| format!("Greska citanja grantova za certifikat: {e}"))?;
    let rows = stmt
        .query_map(params![device_b64, now], |r| r.get::<_, String>(0))
        .map_err(|e| format!("Greska mapiranja grantova: {e}"))?;
    for row in rows {
        let raw = row.map_err(|e| e.to_string())?;
        if let Ok(ControlPayload::ListGrant(g)) = serde_json::from_str(&raw) {
            grants.push(g);
        }
    }

    let mut control_versions = Vec::new();
    if let Some(ref o) = ovlast
        && let Some(v) = load_control_version_for_entity(
            ctx.conn,
            "ovlast",
            &format!("ovlast:{}", o.device_key),
        )?
    {
        control_versions.push(v);
    }
    if let Some(ref admitter) = membership.admitter
        && let Some(v) = load_control_version_for_entity(
            ctx.conn,
            "ovlast",
            &format!("ovlast:{}", admitter.device_key),
        )?
        && !control_versions
            .iter()
            .any(|x| x.version_id == v.version_id)
    {
        control_versions.push(v);
    }
    if let Some(v) = load_control_version_for_entity(
        ctx.conn,
        "membership",
        &format!("membership:{device_b64}"),
    )? {
        control_versions.push(v);
    }

    let member_role = load_active_member_role(ctx.conn, ctx.network_pub, &device_b64, now)?;
    if let Some(v) = load_control_version_for_entity(
        ctx.conn,
        "member_role",
        &format!("member_role:{device_b64}"),
    )? {
        control_versions.push(v);
    }

    let creds = NodeCredentials {
        membership,
        member_role,
        ovlast,
        grants,
        control_versions,
    };
    serde_json::to_vec(&creds)
        .map(Some)
        .map_err(|e| format!("Greska serijalizacije vjerodajnica: {e}"))
}

pub fn can_admit(ctx: &PairingContext<'_>) -> Result<bool, String> {
    if ctx.network_key.is_some() {
        return Ok(true);
    }
    let now = Utc::now().timestamp();
    let device_b64 = public_key_to_base64(ctx.device_key.verifying_key().as_bytes());
    Ok(load_active_ovlast(ctx.conn, ctx.network_pub, &device_b64, now)?.is_some())
}

/// Prepares and signs a `WelcomePack` in memory without modifying the database.
/// Nothing is persisted until both sides confirm the SAS code.
pub fn prepare_welcome_pack(
    ctx: &PairingContext<'_>,
    peer_key: &[u8; 32],
    role: PeerRole,
    shared_lists: &str,
    admit: bool,
) -> Result<WelcomePack, String> {
    let now = Utc::now().timestamp();
    let our_b64 = public_key_to_base64(ctx.device_key.verifying_key().as_bytes());
    let peer_b64 = public_key_to_base64(peer_key);
    if peer_b64 == our_b64 {
        return Err("Uredjaj se ne moze upariti sam sa sobom".into());
    }
    let peer_vk = parse_public_key(&peer_b64).map_err(|e| e.to_string())?;
    if crate::acl::revoked(ctx.conn, "membership", &peer_b64, "")? {
        return Err("Kljuc drugog uredjaja je opozvan; uparivanje zahtijeva novi kljuc".into());
    }

    let network_b64 = public_key_to_base64(ctx.network_pub.as_bytes());
    let mine = load_active_membership(ctx.conn, ctx.network_pub, &our_b64, now)?;
    let my_role = load_active_member_role(ctx.conn, ctx.network_pub, &our_b64, now)?;
    let our_ovlast = load_active_ovlast(ctx.conn, ctx.network_pub, &our_b64, now)?;

    let mut pack = WelcomePack {
        network_name: ctx.network_name.to_owned(),
        network_key: network_b64,
        principal_id: ctx.principal.to_owned(),
        mine,
        my_role,
        for_you: None,
        role_for_you: None,
        ovlast: our_ovlast.clone(),
        grants: Vec::new(),
        control_versions: Vec::new(),
    };

    if let Some(v) =
        load_control_version_for_entity(ctx.conn, "membership", &format!("membership:{our_b64}"))?
    {
        pack.control_versions.push(v);
    }
    if let Some(v) =
        load_control_version_for_entity(ctx.conn, "member_role", &format!("member_role:{our_b64}"))?
    {
        pack.control_versions.push(v);
    }
    if let Some(ref o) = our_ovlast
        && let Some(v) = load_control_version_for_entity(
            ctx.conn,
            "ovlast",
            &format!("ovlast:{}", o.device_key),
        )?
    {
        pack.control_versions.push(v);
    }

    if !admit {
        return Ok(pack);
    }

    let member_role = match role {
        PeerRole::OwnDevice => MemberRole::OwnDevice,
        PeerRole::FamilyMember => MemberRole::FamilyMember,
    };

    // If the peer already has an active membership with the same role (e.g. unilateral finish retry),
    // reuse its principal_id and certificate so re-pairing is idempotent.
    if let Some(existing) = load_active_membership(ctx.conn, ctx.network_pub, &peer_b64, now)?
        && let Some(existing_role) =
            load_active_member_role(ctx.conn, ctx.network_pub, &peer_b64, now)?
        && existing_role.role == member_role
    {
        pack.for_you = Some(existing);
        pack.role_for_you = Some(existing_role);
        if let Some(v) = load_control_version_for_entity(
            ctx.conn,
            "membership",
            &format!("membership:{peer_b64}"),
        )? {
            pack.control_versions.push(v);
        }
        if let Some(v) = load_control_version_for_entity(
            ctx.conn,
            "member_role",
            &format!("member_role:{peer_b64}"),
        )? {
            pack.control_versions.push(v);
        }
        return Ok(pack);
    }

    let target_principal = match member_role {
        MemberRole::OwnDevice => ctx.principal.to_owned(),
        MemberRole::FamilyMember => {
            let existing_principal: Option<String> = ctx
                .conn
                .query_row(
                    "SELECT target_id FROM sync_trust_records WHERE kind = 'member_role' AND subject_key = ?1 LIMIT 1",
                    params![peer_b64],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            existing_principal.unwrap_or_else(|| principal_id_from_key(peer_key))
        }
    };

    let validity = Validity {
        issued_at: now,
        valid_for_secs: DEFAULT_MEMBERSHIP_VALIDITY_SECS,
    };

    let (for_you_member, for_you_role, signer_node, signer_sk) = if let Some(root_sk) =
        ctx.network_key
    {
        let net = NetworkKey::from_signing_key(ctx.network_name, root_sk.clone());
        let m = net
            .admit_member(&peer_b64, &peer_vk, "root", validity)
            .map_err(|e| e.to_string())?;
        let r = issue_member_role(
            &net,
            &peer_b64,
            &peer_vk,
            member_role,
            &target_principal,
            "root",
            validity,
        )
        .map_err(|e| e.to_string())?;
        (
            m,
            r,
            public_key_to_base64(ctx.network_pub.as_bytes()),
            root_sk,
        )
    } else if let Some(ref ovlast) = our_ovlast {
        let m = Membership::admit_as(
            ctx.device_key,
            ovlast.clone(),
            &peer_b64,
            &peer_vk,
            validity,
        )
        .map_err(|e| e.to_string())?;
        let r = MemberRoleRecord::admit_as(
            ctx.device_key,
            ovlast.clone(),
            &peer_b64,
            &peer_vk,
            member_role,
            &target_principal,
            validity,
        )
        .map_err(|e| e.to_string())?;
        (m, r, our_b64.clone(), ctx.device_key)
    } else {
        return Err(
            "Ovaj uredjaj ne drzi mrezni kljuc ni aktivnu ovlast za primanje novih clanova".into(),
        );
    };

    let ctrl_version = build_in_memory_control_version(
        ctx.conn,
        &signer_node,
        signer_sk,
        "membership",
        &format!("membership:{peer_b64}"),
        &ControlPayload::Membership(for_you_member.clone()),
    )?;
    let role_version = build_in_memory_control_version(
        ctx.conn,
        &signer_node,
        signer_sk,
        "member_role",
        &format!("member_role:{peer_b64}"),
        &ControlPayload::MemberRole(for_you_role.clone()),
    )?;
    pack.for_you = Some(for_you_member);
    pack.role_for_you = Some(for_you_role);
    pack.control_versions.push(ctrl_version);
    pack.control_versions.push(role_version);

    if member_role == MemberRole::FamilyMember {
        let list_ids: Vec<&str> = shared_lists
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty() && *s != "*")
            .collect();
        for list_id in list_ids {
            let grant_id = format!("grant:{peer_b64}:{list_id}");
            let g = if let Some(root_sk) = ctx.network_key {
                let net = NetworkKey::from_signing_key(ctx.network_name, root_sk.clone());
                issue_list_grant(
                    &net,
                    &grant_id,
                    list_id,
                    &peer_vk,
                    ListPermission::Write,
                    "root",
                    validity,
                )
                .map_err(|e| e.to_string())?
            } else if let Some(ref ovlast) = our_ovlast {
                use base64::{Engine, engine::general_purpose::STANDARD};
                use ed25519_dalek::Signer;
                let mut g = ListGrant {
                    network: public_key_to_base64(ctx.network_pub.as_bytes()),
                    grant_id: grant_id.clone(),
                    list_sync_id: list_id.to_owned(),
                    recipient_pub_key: peer_b64.clone(),
                    permission: ListPermission::Write,
                    issued_by: our_b64.clone(),
                    issued_at: validity.issued_at,
                    expires_at: validity.expires_at().map_err(|e| e.to_string())?,
                    signature: String::new(),
                    admitter: Some(ovlast.clone()),
                };
                let sig = ctx.device_key.sign(&g.signed_bytes());
                g.signature = STANDARD.encode(sig.to_bytes());
                g
            } else {
                continue;
            };
            let gv = build_in_memory_control_version(
                ctx.conn,
                &signer_node,
                signer_sk,
                "list_grant",
                &format!("list_grant:{}", g.grant_id),
                &ControlPayload::ListGrant(g.clone()),
            )?;
            pack.grants.push(g);
            pack.control_versions.push(gv);
        }
    }

    Ok(pack)
}

fn build_in_memory_control_version(
    conn: &Connection,
    node_id: &str,
    sk: &SigningKey,
    entity: &str,
    entity_id: &str,
    ctrl: &ControlPayload,
) -> Result<Version, String> {
    let supersedes: String = conn
        .query_row(
            "SELECT COALESCE(MAX(version_id), '') FROM record_versions
             WHERE entity = ?1 AND entity_id = ?2 AND signature != ''",
            params![entity, entity_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let id = uuid::Uuid::now_v7();
    let version_id = id.to_string();
    let (seconds, nanos) = id.get_timestamp().expect("UUIDv7").to_unix();
    let created = chrono::DateTime::<Utc>::from_timestamp(seconds as i64, nanos)
        .ok_or("UUIDv7 vrijeme izvan raspona")?;
    let created_at = created.to_rfc3339();
    let created_at_unix = created.timestamp();
    let payload = serde_json::to_string(ctrl).map_err(|e| e.to_string())?;
    let env = syncnet::trust::SignedVersionEnvelope {
        version_id: version_id.clone(),
        entity: entity.to_owned(),
        entity_id: entity_id.to_owned(),
        node_id: node_id.to_owned(),
        supersedes: supersedes.clone(),
        archived: false,
        channel: "control".to_owned(),
        created_at: created_at.clone(),
        created_at_unix,
        schema_version: 1,
        author_pub_key: public_key_to_base64(sk.verifying_key().as_bytes()),
        grant_ref: String::new(),
        payload: payload.clone(),
        signature: String::new(),
    }
    .sign(crate::apply::PROTOCOL_NAME, sk);

    Ok(Version {
        version_id,
        entity: entity.to_owned(),
        entity_id: entity_id.to_owned(),
        node_id: node_id.to_owned(),
        supersedes,
        archived: false,
        payload,
        created_at,
        schema_version: 1,
        channel: "control".to_owned(),
        author_pub_key: env.author_pub_key,
        grant_ref: String::new(),
        signature: env.signature,
        created_at_unix,
    })
}

fn insert_control_version_if_missing(conn: &Connection, v: &Version) -> Result<(), String> {
    if v.channel != "control" || v.signature.is_empty() {
        return Ok(());
    }
    conn.execute(
        "INSERT OR IGNORE INTO record_versions
         (version_id, entity, entity_id, node_id, supersedes, archived, payload, created_at,
          schema_version, channel, author_pub_key, grant_ref, signature, created_at_unix, applied_at, legacy_imported)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'control', ?10, ?11, ?12, ?13, ?14, 0)",
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
            v.author_pub_key,
            v.grant_ref,
            v.signature,
            v.created_at_unix,
            Utc::now().to_rfc3339(),
        ],
    )
    .map_err(|e| format!("Greska spremanja kontrolne verzije: {e}"))?;
    Ok(())
}

pub struct JoinedNetworkUpdate {
    pub network_public: VerifyingKey,
    pub principal: String,
}

/// Atomically persists the pairing outcome after mutual SAS confirmation.
/// Returns `Some(JoinedNetworkUpdate)` when a fresh local database joined the peer's network.
#[allow(clippy::too_many_arguments)]
pub fn finalize_pairing_in_db(
    ctx: &PairingContext<'_>,
    peer_hello: &PairHello,
    peer_key: &[u8; 32],
    sas_code: &str,
    role: PeerRole,
    shared_lists: &str,
    given: Option<&WelcomePack>,
    received: Option<&WelcomePack>,
) -> Result<(PairOutcomeView, Option<JoinedNetworkUpdate>), String> {
    let now = Utc::now().timestamp();
    let our_b64 = public_key_to_base64(ctx.device_key.verifying_key().as_bytes());
    let peer_b64 = public_key_to_base64(peer_key);
    let peer_vk = parse_public_key(&peer_b64).map_err(|e| e.to_string())?;
    let our_net_b64 = public_key_to_base64(ctx.network_pub.as_bytes());

    let tx = ctx
        .conn
        .unchecked_transaction()
        .map_err(|e| format!("Greska transakcije uparivanja: {e}"))?;

    let other_members_count: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM sync_trust_records WHERE kind = 'membership' AND subject_key != ?1",
            params![our_b64],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;

    let mut joined_update: Option<JoinedNetworkUpdate> = None;
    let mut peer_is_member = false;
    let mut peer_admitted_by_us = false;
    let mut joined_peer_role: Option<PeerRole> = None;

    let we_are_admitting_peer = given.and_then(|g| g.for_you.as_ref()).is_some();

    if we_are_admitting_peer
        && let Some(theirs) = received
        && !theirs.network_key.is_empty()
        && theirs.network_key != our_net_b64
        && theirs.for_you.is_some()
    {
        return Err(
            "Obje strane su pokusale primiti jedna drugu u razlicite mreze; na uredjaju koji se pridruzuje odaberite 'Pridruzi se mrezi partnera'."
                .into(),
        );
    }

    if !we_are_admitting_peer
        && let Some(theirs) = received
        && !theirs.network_key.is_empty()
        && theirs.network_key != our_net_b64
    {
        if other_members_count > 0 {
            return Err(format!(
                "Uredjaj {} pripada drugoj mrezi ({}) i ne moze se spojiti s postojecom mrezom",
                peer_hello.name, theirs.network_name
            ));
        }
        let Some(ref for_me) = theirs.for_you else {
            return Err(
                "Druga strana nije izdala potvrdu clanstva za ulazak u njezinu mrezu".into(),
            );
        };
        let Some(ref my_role_rec) = theirs.role_for_you else {
            return Err(
                "Druga strana nije izdala potvrdu uloge clanstva za ulazak u njezinu mrezu".into(),
            );
        };
        let new_net_vk = parse_public_key(&theirs.network_key).map_err(|e| e.to_string())?;
        for_me
            .verify(&new_net_vk, &ctx.device_key.verifying_key(), now)
            .map_err(|e| format!("Potvrda clanstva za ovaj uredjaj nije valjana: {e}"))?;
        my_role_rec
            .verify(&new_net_vk, &ctx.device_key.verifying_key(), now)
            .map_err(|e| format!("Potvrda uloge clanstva za ovaj uredjaj nije valjana: {e}"))?;

        let new_principal = my_role_rec.principal_id.clone();
        let old_principal = ctx.principal.to_owned();

        if my_role_rec.role == MemberRole::OwnDevice && new_principal != old_principal {
            tx.execute(
                "DELETE FROM stores
                 WHERE sync_id NOT IN (SELECT store_sync_id FROM receipts WHERE store_sync_id != '')
                   AND sync_id NOT IN (SELECT COALESCE(store_sync_id, '') FROM shopping_lists)
                   AND sync_id NOT IN (SELECT store_sync_id FROM price_history WHERE store_sync_id != '')",
                [],
            )
            .map_err(|e| format!("Greska ciscenja pocetnih trgovina: {e}"))?;
        }

        tx.execute("DELETE FROM sync_trust_records", [])
            .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM record_versions", [])
            .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM sync_backfill_revisions", [])
            .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM sync_backfill_ack", [])
            .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM sync_peer_state", [])
            .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM sync_holes", [])
            .map_err(|e| e.to_string())?;

        for (k, v) in [
            ("sync_network_public", theirs.network_key.as_str()),
            ("sync_principal", new_principal.as_str()),
            (
                "network_name",
                if theirs.network_name.trim().is_empty() {
                    ctx.network_name
                } else {
                    theirs.network_name.trim()
                },
            ),
        ] {
            tx.execute(
                "INSERT INTO settings(key,value) VALUES (?1,?2)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![k, v],
            )
            .map_err(|e| e.to_string())?;
        }

        if let Some(ref o) = theirs.ovlast {
            store_control_record(&tx, &new_net_vk, &ControlPayload::Ovlast(o.clone()), now)?;
        }
        if let Some(ref admitter) = for_me.admitter {
            store_control_record(
                &tx,
                &new_net_vk,
                &ControlPayload::Ovlast(admitter.clone()),
                now,
            )?;
        }
        store_control_record(
            &tx,
            &new_net_vk,
            &ControlPayload::Membership(for_me.clone()),
            now,
        )?;
        store_control_record(
            &tx,
            &new_net_vk,
            &ControlPayload::MemberRole(my_role_rec.clone()),
            now,
        )?;
        if let Some(ref peer_m) = theirs.mine
            && peer_m.verify(&new_net_vk, &peer_vk, now).is_ok()
        {
            store_control_record(
                &tx,
                &new_net_vk,
                &ControlPayload::Membership(peer_m.clone()),
                now,
            )?;
            if let Some(ref peer_r) = theirs.my_role
                && peer_r.verify(&new_net_vk, &peer_vk, now).is_ok()
            {
                store_control_record(
                    &tx,
                    &new_net_vk,
                    &ControlPayload::MemberRole(peer_r.clone()),
                    now,
                )?;
            }
            peer_is_member = true;
        } else if peer_b64 == theirs.network_key {
            peer_is_member = true;
        }
        for g in &theirs.grants {
            store_control_record(&tx, &new_net_vk, &ControlPayload::ListGrant(g.clone()), now)?;
        }
        for cv in &theirs.control_versions {
            insert_control_version_if_missing(&tx, cv)?;
        }

        if my_role_rec.role == MemberRole::OwnDevice {
            resign_local_domain_records(&tx, ctx.device_key, &new_net_vk, &new_principal, now)?;
        }

        joined_peer_role = Some(theirs.my_role.as_ref().map_or(
            PeerRole::OwnDevice,
            |r| match r.role {
                MemberRole::OwnDevice => PeerRole::OwnDevice,
                MemberRole::FamilyMember => PeerRole::FamilyMember,
            },
        ));

        joined_update = Some(JoinedNetworkUpdate {
            network_public: new_net_vk,
            principal: new_principal,
        });
    } else {
        if let Some(theirs) = received {
            if let Some(ref o) = theirs.ovlast {
                let _ = store_control_record(
                    &tx,
                    ctx.network_pub,
                    &ControlPayload::Ovlast(o.clone()),
                    now,
                );
            }
            if let Some(ref peer_m) = theirs.mine
                && peer_m.verify(ctx.network_pub, &peer_vk, now).is_ok()
            {
                store_control_record(
                    &tx,
                    ctx.network_pub,
                    &ControlPayload::Membership(peer_m.clone()),
                    now,
                )?;
                peer_is_member = true;
            }
            if let Some(ref for_me) = theirs.for_you
                && for_me
                    .verify(ctx.network_pub, &ctx.device_key.verifying_key(), now)
                    .is_ok()
            {
                if let Some(ref admitter) = for_me.admitter {
                    store_control_record(
                        &tx,
                        ctx.network_pub,
                        &ControlPayload::Ovlast(admitter.clone()),
                        now,
                    )?;
                }
                store_control_record(
                    &tx,
                    ctx.network_pub,
                    &ControlPayload::Membership(for_me.clone()),
                    now,
                )?;
            }
        }

        if let Some(our_pack) = given
            && let Some(ref for_peer) = our_pack.for_you
        {
            let ctrl = ControlPayload::Membership(for_peer.clone());
            store_control_record(&tx, ctx.network_pub, &ctrl, now)?;
            if let Some(ref role_for_peer) = our_pack.role_for_you {
                let role_ctrl = ControlPayload::MemberRole(role_for_peer.clone());
                store_control_record(&tx, ctx.network_pub, &role_ctrl, now)?;
            }
            for g in &our_pack.grants {
                store_control_record(
                    &tx,
                    ctx.network_pub,
                    &ControlPayload::ListGrant(g.clone()),
                    now,
                )?;
            }
            for cv in &our_pack.control_versions {
                insert_control_version_if_missing(&tx, cv)?;
            }
            let payload_str = serde_json::to_string(&ctrl).map_err(|e| e.to_string())?;
            let existing_v = load_control_version_for_entity(
                &tx,
                "membership",
                &format!("membership:{peer_b64}"),
            )?;
            let needs_record = existing_v.is_none_or(|v| v.payload != payload_str);
            if needs_record {
                let (signer_node, signer_sk) = if let Some(root_sk) = ctx.network_key {
                    (our_net_b64.as_str(), root_sk)
                } else {
                    (our_b64.as_str(), ctx.device_key)
                };
                record_signed_version(
                    &tx,
                    signer_node,
                    signer_sk,
                    "membership",
                    &format!("membership:{peer_b64}"),
                    false,
                    &payload_str,
                    "control",
                    "",
                )?;
            }
            peer_is_member = true;
            peer_admitted_by_us = true;
        }
    }

    let clean_name = if peer_hello.name.trim().is_empty() {
        peer_hello.device_id.trim()
    } else {
        peer_hello.name.trim()
    };
    let clean_lists = if shared_lists.trim().is_empty() {
        "Sve obiteljske liste"
    } else {
        shared_lists.trim()
    };
    let stored_role = joined_peer_role.unwrap_or(role);
    let stored_lists = if stored_role == PeerRole::OwnDevice {
        "*"
    } else {
        clean_lists
    };
    tx.execute(
        "INSERT INTO network_members (peer_id, name, role, sas_code, channels, shared_lists, added_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(peer_id) DO UPDATE SET
            name = excluded.name,
            role = excluded.role,
            sas_code = excluded.sas_code,
            channels = excluded.channels,
            shared_lists = excluded.shared_lists",
        params![
            peer_b64,
            clean_name,
            stored_role.as_str(),
            sas_code,
            stored_role.channels(),
            stored_lists,
            Utc::now().to_rfc3339(),
        ],
    )
    .map_err(|e| format!("Greska upisa uparenog clana: {e}"))?;

    tx.commit()
        .map_err(|e| format!("Greska potvrde transakcije uparivanja: {e}"))?;

    let message = if joined_update.is_some() {
        format!("Ovaj uredjaj je uparen i primljen u mrezu s uredjajem {clean_name}.")
    } else if peer_admitted_by_us {
        format!("Uredjaj {clean_name} je uspjesno uparen i primljen u mrezu.")
    } else if peer_is_member {
        format!("Uredjaj {clean_name} je uparen i vec je clan mreze.")
    } else {
        format!("Uredjaj {clean_name} je uparen, ali jos ceka potvrdu clanstva administratora.")
    };

    Ok((
        PairOutcomeView {
            paired: true,
            member: peer_is_member || joined_update.is_some(),
            admitted: peer_admitted_by_us,
            peer_public_key: peer_b64,
            message,
        },
        joined_update,
    ))
}

fn resign_local_domain_records(
    conn: &Connection,
    device_key: &SigningKey,
    network_pub: &VerifyingKey,
    principal: &str,
    now_unix: i64,
) -> Result<(), String> {
    let device_b64 = public_key_to_base64(device_key.verifying_key().as_bytes());
    let channel = format!("private:{principal}");
    for (table, entity) in [
        ("stores", "stores"),
        ("products", "products"),
        ("price_history", "price_history"),
        ("receipts", "receipts"),
        ("receipt_items", "receipt_items"),
        ("shopping_lists", "shopping_lists"),
        ("planned_items", "planned_items"),
    ] {
        let mut stmt = conn
            .prepare(&format!("SELECT sync_id FROM {table} ORDER BY rowid"))
            .map_err(|e| e.to_string())?;
        let ids: Vec<String> = stmt
            .query_map([], |r| r.get(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        for id in ids {
            let (payload, archived) = crate::db::local_snapshot_for_pairing(conn, entity, &id)?;
            let v = record_signed_version(
                conn,
                &device_b64,
                device_key,
                entity,
                &id,
                archived,
                &payload,
                &channel,
                "",
            )?;
            verify_domain_version_auth(conn, network_pub, &v, now_unix)?;
        }
    }
    Ok(())
}

/// Checks whether a peer with TLS-proven `peer_key` is an active member of our network.
/// If the peer is not yet in `sync_trust_records` (e.g., because the network connection
/// dropped at the very end of `Finish` after the peer received our `WelcomePack`),
/// inspects `credentials` (`NodeCredentials` from the peer's TLS 1.3 X.509 certificate SAN)
/// and atomically admits the valid signed membership into our database.
pub fn verify_or_recover_peer_access(
    ctx: &PairingContext<'_>,
    peer_key: &[u8; 32],
    credentials: Option<&[u8]>,
) -> Result<bool, String> {
    let now = Utc::now().timestamp();
    let peer_b64 = public_key_to_base64(peer_key);
    let net_b64 = public_key_to_base64(ctx.network_pub.as_bytes());

    if crate::acl::revoked(ctx.conn, "membership", &peer_b64, "")? {
        return Ok(false);
    }
    if peer_b64 == net_b64 {
        return Ok(true);
    }
    if load_active_membership(ctx.conn, ctx.network_pub, &peer_b64, now)?.is_some() {
        return Ok(true);
    }

    let Some(cred_bytes) = credentials else {
        return Ok(false);
    };
    let Ok(creds) = serde_json::from_slice::<NodeCredentials>(cred_bytes) else {
        return Ok(false);
    };
    if creds.membership.device_key != peer_b64 {
        return Ok(false);
    }
    let peer_vk = parse_public_key(&peer_b64).map_err(|e| e.to_string())?;
    if creds
        .membership
        .verify(ctx.network_pub, &peer_vk, now)
        .is_err()
    {
        return Ok(false);
    }

    let tx = ctx
        .conn
        .unchecked_transaction()
        .map_err(|e| format!("Greska transakcije oporavka clanstva: {e}"))?;

    if let Some(ref admitter) = creds.membership.admitter
        && store_control_record(
            &tx,
            ctx.network_pub,
            &ControlPayload::Ovlast(admitter.clone()),
            now,
        )
        .is_err()
    {
        return Ok(false);
    }
    if store_control_record(
        &tx,
        ctx.network_pub,
        &ControlPayload::Membership(creds.membership.clone()),
        now,
    )
    .is_err()
    {
        return Ok(false);
    }
    if let Some(ref role_rec) = creds.member_role
        && role_rec.device_key == peer_b64
        && role_rec.verify(ctx.network_pub, &peer_vk, now).is_ok()
    {
        let _ = store_control_record(
            &tx,
            ctx.network_pub,
            &ControlPayload::MemberRole(role_rec.clone()),
            now,
        );
    }

    let our_b64 = public_key_to_base64(ctx.device_key.verifying_key().as_bytes());
    let ctrl_payload = ControlPayload::Membership(creds.membership.clone());
    if load_control_version_for_entity(&tx, "membership", &format!("membership:{peer_b64}"))?
        .is_none()
    {
        if let Some(root_sk) = ctx.network_key {
            let payload_str = serde_json::to_string(&ctrl_payload).map_err(|e| e.to_string())?;
            record_signed_version(
                &tx,
                &net_b64,
                root_sk,
                "membership",
                &format!("membership:{peer_b64}"),
                false,
                &payload_str,
                "control",
                "",
            )?;
        } else if creds
            .membership
            .admitter
            .as_ref()
            .is_some_and(|a| a.device_key == our_b64)
        {
            let payload_str = serde_json::to_string(&ctrl_payload).map_err(|e| e.to_string())?;
            record_signed_version(
                &tx,
                &our_b64,
                ctx.device_key,
                "membership",
                &format!("membership:{peer_b64}"),
                false,
                &payload_str,
                "control",
                "",
            )?;
        }
    }

    tx.commit()
        .map_err(|e| format!("Greska spremanja oporavljenog clanstva: {e}"))?;

    Ok(load_active_membership(ctx.conn, ctx.network_pub, &peer_b64, now)?.is_some())
}

/// Thread-safe coordinator for live TLS 1.3 + commit-reveal SAS pairing sessions.
/// Never exposes private keys over IPC or status views.
#[derive(Clone, Default)]
pub struct PairingController {
    slot: PairingSessionSlot,
    confirmation: Arc<Mutex<Option<CancellationToken>>>,
}

// The active token is retained through finalization, so stop_listening and the
// final database commit have a single ordering even when IPC calls race.
struct ConfirmationGuard(Arc<Mutex<Option<CancellationToken>>>);

impl Drop for ConfirmationGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = self.0.lock() {
            active.take();
        }
    }
}

impl PairingController {
    pub fn new() -> Self {
        Self {
            slot: PairingSessionSlot::new(),
            confirmation: Arc::new(Mutex::new(None)),
        }
    }

    pub fn status(&self, can_admit: bool) -> PairStatusView {
        let snap = self.slot.snapshot();
        PairStatusView {
            waiting: snap.waiting,
            pending: snap.pending,
            listen_port: snap.listen_port,
            sas: snap.sas,
            peer_device_id: snap.peer_device_id,
            peer_name: snap.peer_name,
            peer_public_key: snap.peer_public_key,
            peer_host: snap.peer_host,
            error: snap.error,
            can_admit,
        }
    }

    pub async fn start_listening(
        &self,
        bind_addr: SocketAddr,
        sk: SigningKey,
        identity: PairIdentity,
        can_admit: bool,
    ) -> Result<PairStatusView, String> {
        let _ = self.slot.start_listening(bind_addr, sk, identity).await?;
        Ok(self.status(can_admit))
    }

    pub fn stop_listening(&self, can_admit: bool) -> PairStatusView {
        let active = self.confirmation.lock().expect("pairing confirmation lock");
        if let Some(cancel) = active.as_ref() {
            cancel.cancel();
        }
        let _ = self.slot.stop_listening();
        self.status(can_admit)
    }

    pub async fn dial(
        &self,
        addr: SocketAddr,
        sk: SigningKey,
        identity: PairIdentity,
        can_admit: bool,
    ) -> Result<PairStatusView, String> {
        let _ = self.slot.dial(addr, sk, identity).await?;
        Ok(self.status(can_admit))
    }

    pub async fn confirm(
        &self,
        db_mutex: &Mutex<crate::db::Database>,
        approved: bool,
        role: PeerRole,
        shared_lists: &str,
        admit: bool,
    ) -> Result<PairOutcomeView, String> {
        let (pending, cancel) = {
            let mut active = self
                .confirmation
                .lock()
                .map_err(|_| "Greska zakljucavanja potvrde uparivanja".to_owned())?;
            if active.is_some() {
                return Err("Potvrda uparivanja je vec u tijeku".to_owned());
            }
            let pending = self
                .slot
                .take_pending()
                .ok_or_else(|| "Nema uparivanja koje ceka odluku".to_owned())?;
            let cancel = CancellationToken::new();
            *active = Some(cancel.clone());
            (pending, cancel)
        };
        let _confirmation = ConfirmationGuard(Arc::clone(&self.confirmation));
        let peer_hello = pending.peer_hello().clone();
        let peer_key = *pending.peer_key();
        let peer_b64 = pending.peer_key_base64();
        let sas_code = pending.sas().to_owned();

        let given = if approved {
            let db = db_mutex
                .lock()
                .map_err(|_| "Greska zakljucavanja baze".to_owned())?;
            match db.prepare_welcome_pack(&peer_key, role, shared_lists, admit) {
                Ok(pack) => Some(pack),
                Err(err) => {
                    drop(db);
                    reject_pending_in_background(pending);
                    return Err(err);
                }
            }
        } else {
            None
        };

        let finish_res = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Err("Uparivanje je otkazano".to_owned()),
            result = pending.finish(approved, given.as_ref()) => result.map_err(|e| e.to_string())?,
        };

        if !finish_res.approved {
            return Ok(PairOutcomeView {
                paired: false,
                member: false,
                admitted: false,
                peer_public_key: peer_b64,
                message: "Uparivanje odbijeno ili nepotvrdeno s druge strane.".into(),
            });
        }

        let received: Option<WelcomePack> = match finish_res.payload {
            Some(val) => Some(
                serde_json::from_value(val)
                    .map_err(|e| format!("Paket dobrodoslice se ne moze procitati: {e}"))?,
            ),
            None => None,
        };

        let mut db = db_mutex
            .lock()
            .map_err(|_| "Greska zakljucavanja baze".to_owned())?;
        let _active = self
            .confirmation
            .lock()
            .map_err(|_| "Greska zakljucavanja potvrde uparivanja".to_owned())?;
        if cancel.is_cancelled() {
            return Err("Uparivanje je otkazano".to_owned());
        }
        db.finalize_pairing(
            &peer_hello,
            &peer_key,
            &sas_code,
            role,
            shared_lists,
            given.as_ref(),
            received.as_ref(),
        )
    }

    pub fn build_pair_identity(
        device_public_key: &str,
        device_name: &str,
        protocol: &str,
    ) -> PairIdentity {
        PairIdentity {
            protocol: protocol.to_owned(),
            device_id: device_public_key.to_owned(),
            name: if device_name.trim().is_empty() {
                "List2Cart uredjaj".to_owned()
            } else {
                device_name.trim().to_owned()
            },
            version: env!("CARGO_PKG_VERSION").to_owned(),
            meta: BTreeMap::new(),
        }
    }
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    use crate::db::Database;
    use std::time::Duration;
    use tokio::time::timeout;

    struct Peers {
        left: PairingController,
        right: PairingController,
        left_db: Arc<Mutex<Database>>,
        right_db: Arc<Mutex<Database>>,
    }

    fn identity(db: &Mutex<Database>, name: &str) -> (SigningKey, PairIdentity) {
        let db = db.lock().unwrap();
        let key = db.clone_device_signing_key();
        let public = public_key_to_base64(key.verifying_key().as_bytes());
        let identity =
            PairingController::build_pair_identity(&public, name, crate::apply::PROTOCOL_NAME);
        (key, identity)
    }

    async fn connected_peers() -> Peers {
        let peers = Peers {
            left: PairingController::new(),
            right: PairingController::new(),
            left_db: Arc::new(Mutex::new(Database::open_in_memory().unwrap())),
            right_db: Arc::new(Mutex::new(Database::open_in_memory().unwrap())),
        };
        let (key, hello) = identity(&peers.left_db, "left");
        let status = peers
            .left
            .start_listening("127.0.0.1:0".parse().unwrap(), key, hello, true)
            .await
            .unwrap();
        let addr = format!("127.0.0.1:{}", status.listen_port.unwrap())
            .parse()
            .unwrap();
        let (key, hello) = identity(&peers.right_db, "right");
        peers.right.dial(addr, key, hello, true).await.unwrap();
        timeout(Duration::from_secs(5), async {
            while !peers.left.status(true).pending {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(peers.left.status(true).sas, peers.right.status(true).sas);
        peers
    }

    #[tokio::test]
    async fn stop_closes_waiting_listener() {
        let controller = PairingController::new();
        let db = Mutex::new(Database::open_in_memory().unwrap());
        let (key, hello) = identity(&db, "waiting");
        let status = controller
            .start_listening("127.0.0.1:0".parse().unwrap(), key, hello, true)
            .await
            .unwrap();
        let addr = format!("127.0.0.1:{}", status.listen_port.unwrap());
        let stopped = controller.stop_listening(true);
        assert!(!stopped.waiting && !stopped.pending);
        timeout(Duration::from_secs(2), async {
            while tokio::net::TcpStream::connect(&addr).await.is_ok() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("listener must close after cancellation");
    }

    #[tokio::test]
    async fn stop_rejects_pending_partner_without_adding_members() {
        let peers = connected_peers().await;
        let stopped = peers.left.stop_listening(true);
        assert!(!stopped.waiting && !stopped.pending);
        let result = timeout(
            Duration::from_secs(2),
            peers
                .right
                .confirm(&peers.right_db, true, PeerRole::OwnDevice, "*", false),
        )
        .await
        .unwrap();
        assert!(result.is_err() || !result.unwrap().paired);
        assert!(
            peers
                .left_db
                .lock()
                .unwrap()
                .list_members()
                .unwrap()
                .is_empty()
        );
        assert!(
            peers
                .right_db
                .lock()
                .unwrap()
                .list_members()
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn stop_interrupts_confirmation_already_waiting_for_partner() {
        let peers = connected_peers().await;
        let controller = peers.left.clone();
        let db = Arc::clone(&peers.left_db);
        let task = tokio::spawn(async move {
            controller
                .confirm(&db, true, PeerRole::OwnDevice, "*", true)
                .await
        });
        timeout(Duration::from_secs(2), async {
            while peers.left.confirmation.lock().unwrap().is_none() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        peers.left.stop_listening(true);
        let result = timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.unwrap_err(), "Uparivanje je otkazano");
        assert!(peers.left.confirmation.lock().unwrap().is_none());
        peers.right.stop_listening(true);
        assert!(
            peers
                .left_db
                .lock()
                .unwrap()
                .list_members()
                .unwrap()
                .is_empty()
        );
        assert!(
            peers
                .right_db
                .lock()
                .unwrap()
                .list_members()
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn normal_confirmation_still_completes_for_both_partners() {
        let peers = connected_peers().await;
        let (left, right) = timeout(Duration::from_secs(5), async {
            tokio::join!(
                peers
                    .left
                    .confirm(&peers.left_db, true, PeerRole::OwnDevice, "*", true),
                peers
                    .right
                    .confirm(&peers.right_db, true, PeerRole::OwnDevice, "*", false),
            )
        })
        .await
        .unwrap();
        assert!(left.unwrap().paired);
        assert!(right.unwrap().paired);
        assert_eq!(
            peers.left_db.lock().unwrap().list_members().unwrap().len(),
            1
        );
        assert_eq!(
            peers.right_db.lock().unwrap().list_members().unwrap().len(),
            1
        );
    }
}
