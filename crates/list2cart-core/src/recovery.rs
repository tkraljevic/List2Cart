//! Immutable, recipient-bound snapshot pages. Partial pages never enter projections.
use crate::apply::{
    ApplyContext, PROTOCOL_NAME, allowed_channels_for_recipient,
    apply_remote_versions_in_transaction, sender_is_own_principal,
};
use crate::ledger::{
    Version, delta_byte_budget, frontier_key, list_holes, narrow_holes, serialized_json_len,
    update_peer_ack,
};
use ed25519_dalek::{SigningKey, VerifyingKey};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use syncnet::identity::{parse_public_key, public_key_to_base64};
use syncnet::wire::bundle_v2::ProtectedBundleV2;
use uuid::Uuid;

const MAX_ITEMS: usize = 100_000;
const MAX_BYTES: usize = 64 * 1024 * 1024;
const MAX_SESSIONS: i64 = 8;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotManifest {
    pub format: u32,
    pub snapshot_id: String,
    pub sender: String,
    pub recipient: String,
    pub channels: BTreeSet<String>,
    pub frontier: BTreeMap<String, String>,
    pub total_versions: usize,
    pub total_bytes: usize,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub backfill_revisions: crate::backfill::BackfillRevisions,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotPage {
    pub manifest: SnapshotManifest,
    pub offset: usize,
    pub versions: Vec<Version>,
    pub done: bool,
}

#[derive(Serialize, Deserialize)]
struct SnapshotEnvelope {
    snapshot_page: SnapshotPage,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotProgress {
    pub snapshot_id: String,
    pub next_offset: usize,
    pub total_versions: usize,
    pub complete: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct RecoveryContext<'a> {
    pub conn: &'a Connection,
    pub key: &'a SigningKey,
    pub network: &'a VerifyingKey,
    pub principal: &'a str,
    pub now: i64,
}

pub(crate) fn own_peer(ctx: &RecoveryContext<'_>, peer: &str) -> Result<(), String> {
    parse_public_key(peer).map_err(|e| e.to_string())?;
    if !sender_is_own_principal(ctx.conn, ctx.network, peer, ctx.principal, ctx.now)?
        || !allowed_channels_for_recipient(ctx.conn, ctx.network, peer, ctx.now)?
            .contains(&format!("private:{}", ctx.principal))
    {
        return Err("Snapshot oporavak dopušten je samo vlastitom ovlaštenom uređaju".into());
    }
    Ok(())
}

fn quota(conn: &Connection, direction: &str) -> Result<(), String> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sync_snapshot_sessions WHERE direction=?1 AND complete=0",
            [direction],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if count >= MAX_SESSIONS {
        return Err("Previše nedovršenih snapshot prijenosa; odbacite stari prijenos".into());
    }
    Ok(())
}

pub(crate) fn hash_versions(versions: &[Version]) -> Result<(String, usize), String> {
    let bytes = serde_json::to_vec(versions).map_err(|e| e.to_string())?;
    Ok((hex::encode(Sha256::digest(&bytes)), bytes.len()))
}

pub(crate) fn local_acl_hash(conn: &Connection) -> Result<String, String> {
    let mut stmt = conn
        .prepare("SELECT record_id,raw_json,revoked FROM sync_trust_records ORDER BY record_id")
        .map_err(|e| e.to_string())?;
    let values = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(hex::encode(Sha256::digest(
        serde_json::to_vec(&values).map_err(|e| e.to_string())?,
    )))
}

fn frontier(versions: &[Version]) -> BTreeMap<String, String> {
    let mut result: BTreeMap<String, String> = BTreeMap::new();
    for v in versions {
        result
            .entry(frontier_key(&v.node_id, &v.channel))
            .and_modify(|id| *id = id.clone().max(v.version_id.clone()))
            .or_insert_with(|| v.version_id.clone());
    }
    result
}

fn snapshot_frontier(versions: &[Version], format: u32) -> BTreeMap<String, String> {
    let mut result = frontier(versions);
    if format == 2 {
        result.retain(|key, _| crate::ledger::split_frontier_key(key).1 != "control");
    }
    result
}

pub(crate) fn start(ctx: RecoveryContext<'_>, recipient: &str) -> Result<SnapshotManifest, String> {
    start_inner(ctx, recipient, false)
}

pub(crate) fn start_family(
    ctx: RecoveryContext<'_>,
    recipient: &str,
) -> Result<SnapshotManifest, String> {
    start_inner(ctx, recipient, true)
}

pub(crate) fn family_peer(ctx: &RecoveryContext<'_>, peer: &str) -> Result<(), String> {
    parse_public_key(peer).map_err(|e| e.to_string())?;
    let local = public_key_to_base64(ctx.key.verifying_key().as_bytes());
    if peer == local
        || !allowed_channels_for_recipient(ctx.conn, ctx.network, peer, ctx.now)?
            .contains("control")
        || !allowed_channels_for_recipient(ctx.conn, ctx.network, &local, ctx.now)?
            .contains("control")
    {
        return Err("Obiteljski prijenos zahtijeva dva aktivna člana mreže".into());
    }
    Ok(())
}

pub(crate) fn family_scope(
    ctx: &RecoveryContext<'_>,
    peer: &str,
) -> Result<BTreeSet<String>, String> {
    family_peer(ctx, peer)?;
    let local = public_key_to_base64(ctx.key.verifying_key().as_bytes());
    let root = public_key_to_base64(ctx.network.as_bytes());
    let ours = allowed_channels_for_recipient(ctx.conn, ctx.network, &local, ctx.now)?;
    let theirs = allowed_channels_for_recipient(ctx.conn, ctx.network, peer, ctx.now)?;
    Ok(if local == root {
        theirs
    } else if peer == root {
        ours
    } else {
        ours.intersection(&theirs).cloned().collect()
    }
    .into_iter()
    .filter(|ch| ch == "control" || ch.starts_with("list:"))
    .collect())
}

fn authorize(ctx: &RecoveryContext<'_>, peer: &str, format: u32) -> Result<(), String> {
    if format == 2 {
        family_peer(ctx, peer)
    } else {
        own_peer(ctx, peer)
    }
}

fn family_access(ctx: &RecoveryContext<'_>, manifest: &SnapshotManifest) -> Result<(), String> {
    let root = public_key_to_base64(ctx.network.as_bytes());
    for key in [&manifest.sender, &manifest.recipient] {
        if key == &root {
            continue;
        }
        let allowed = allowed_channels_for_recipient(ctx.conn, ctx.network, key, ctx.now)?;
        if !manifest.channels.is_subset(&allowed) {
            return Err("Član više nema prava na obiteljski snapshot".into());
        }
    }
    Ok(())
}

fn start_inner(
    ctx: RecoveryContext<'_>,
    recipient: &str,
    family: bool,
) -> Result<SnapshotManifest, String> {
    let tx = ctx
        .conn
        .unchecked_transaction()
        .map_err(|e| e.to_string())?;
    quota(&tx, "out")?;
    authorize(&ctx, recipient, if family { 2 } else { 1 })?;
    let mut channels = allowed_channels_for_recipient(&tx, ctx.network, recipient, ctx.now)?;
    if family {
        channels = family_scope(&ctx, recipient)?;
    }
    let relayable_controls = if !family && channels.contains("control") {
        crate::apply::relayable_control_versions(&tx, ctx.network, ctx.now)?
    } else {
        Vec::new()
    };
    let relayable_control_authors: BTreeSet<&str> = relayable_controls
        .iter()
        .map(|v| v.node_id.as_str())
        .collect();
    let relayable_domains =
        crate::apply::relayable_domain_versions(&tx, ctx.network, &channels, ctx.now)?;
    let relayable_authors: BTreeSet<(&str, &str)> = relayable_domains
        .iter()
        .map(|v| (v.node_id.as_str(), v.channel.as_str()))
        .collect();
    if list_holes(&tx)?.iter().any(|hole| {
        channels.contains(&hole.channel)
            && (!family || hole.channel != "control")
            && if hole.channel == "control" {
                relayable_control_authors.contains(hole.author_node_id.as_str())
            } else {
                relayable_authors.contains(&(hole.author_node_id.as_str(), hole.channel.as_str()))
            }
    }) {
        return Err("Izvor ima nepotvrđene rupe; ne može jamčiti potpuni snapshot".into());
    }
    let versions = {
        let mut result = if family {
            crate::family::proofs(
                &tx,
                ctx.network,
                recipient,
                &public_key_to_base64(ctx.network.as_bytes()),
                &channels,
                ctx.now,
            )?
        } else {
            relayable_controls
        };
        let mut bytes = serialized_json_len(&result)?;
        for v in relayable_domains {
            bytes += serialized_json_len(&v)? + usize::from(!result.is_empty());
            if result.len() >= MAX_ITEMS || bytes > MAX_BYTES {
                return Err("Snapshot prelazi ograničenje od 100.000 verzija / 64 MiB".into());
            }
            result.push(v);
        }
        result
    };
    if versions.len() > MAX_ITEMS || serialized_json_len(&versions)? > MAX_BYTES {
        return Err("Snapshot prelazi ograničenje od 100.000 verzija / 64 MiB".into());
    }
    if versions.is_empty() {
        return Err("Nema potpisanih zapisa za snapshot".into());
    }
    let (sha256, total_bytes) = hash_versions(&versions)?;
    let manifest = SnapshotManifest {
        format: if family { 2 } else { 1 },
        snapshot_id: Uuid::new_v4().to_string(),
        sender: public_key_to_base64(ctx.key.verifying_key().as_bytes()),
        recipient: recipient.into(),
        backfill_revisions: crate::backfill::current(
            &tx,
            &channels
                .iter()
                .filter(|c| !family || c.as_str() != "control")
                .cloned()
                .collect(),
        )?,
        channels,
        frontier: snapshot_frontier(&versions, if family { 2 } else { 1 }),
        proof_sha256: if family {
            Some(
                hash_versions(
                    &versions
                        .iter()
                        .filter(|v| v.channel == "control")
                        .cloned()
                        .collect::<Vec<_>>(),
                )?
                .0,
            )
        } else {
            None
        },
        total_versions: versions.len(),
        total_bytes,
        sha256,
    };
    validate_manifest(&manifest)?;
    tx.execute("INSERT INTO sync_snapshot_sessions(direction,snapshot_id,peer_key,manifest) VALUES('out',?1,?2,?3)", params![manifest.snapshot_id, recipient, serde_json::to_string(&manifest).map_err(|e| e.to_string())?]).map_err(|e| e.to_string())?;
    for (index, version) in versions.iter().enumerate() {
        tx.execute(
            "INSERT INTO sync_snapshot_items VALUES('out',?1,?2,?3,?4)",
            params![
                manifest.snapshot_id,
                recipient,
                index as u32,
                serde_json::to_string(version).map_err(|e| e.to_string())?
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())?;
    Ok(manifest)
}

fn validate_manifest(m: &SnapshotManifest) -> Result<(), String> {
    if !crate::backfill::validate(&m.backfill_revisions)
        || m.backfill_revisions
            .keys()
            .any(|ch| !m.channels.contains(ch))
        || !matches!(m.format, 1 | 2)
        || (m.format == 2
            && (m
                .channels
                .iter()
                .any(|c| c != "control" && !c.starts_with("list:"))
                || m.frontier
                    .keys()
                    .any(|k| crate::ledger::split_frontier_key(k).1 == "control")
                || m.backfill_revisions.contains_key("control")
                || m.proof_sha256
                    .as_ref()
                    .is_none_or(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()))))
        || Uuid::parse_str(&m.snapshot_id).is_err()
        || m.total_versions == 0
        || m.total_versions > MAX_ITEMS
        || m.total_bytes > MAX_BYTES
        || m.sha256.len() != 64
        || !m.sha256.bytes().all(|b| b.is_ascii_hexdigit())
        || m.channels.is_empty()
        || m.channels.len() > 128
        || m.channels.iter().any(|s| s.len() > 256)
        || m.frontier.len() > 4096
    {
        return Err("Neispravan ili prevelik snapshot manifest".into());
    }
    Ok(())
}

pub(crate) fn load_session(
    conn: &Connection,
    direction: &str,
    id: &str,
    peer: &str,
) -> Result<Option<(SnapshotManifest, usize, bool)>, String> {
    let row: Option<(String, u32, bool)> = conn.query_row("SELECT manifest,next_offset,complete FROM sync_snapshot_sessions WHERE direction=?1 AND snapshot_id=?2 AND peer_key=?3", params![direction,id,peer], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional().map_err(|e| e.to_string())?;
    row.map(|(manifest, offset, complete)| {
        serde_json::from_str(&manifest)
            .map(|m| (m, offset as usize, complete))
            .map_err(|e| e.to_string())
    })
    .transpose()
}

pub(crate) fn export_page(
    ctx: RecoveryContext<'_>,
    id: &str,
    recipient: &str,
    offset: usize,
    max_bytes: usize,
) -> Result<ProtectedBundleV2, String> {
    if !ctx.conn.is_autocommit() {
        return export_page_in_snapshot(ctx, id, recipient, offset, max_bytes);
    }
    let transaction = ctx
        .conn
        .unchecked_transaction()
        .map_err(|e| e.to_string())?;
    let result = export_page_in_snapshot(ctx, id, recipient, offset, max_bytes)?;
    transaction.commit().map_err(|e| e.to_string())?;
    Ok(result)
}

fn export_page_in_snapshot(
    ctx: RecoveryContext<'_>,
    id: &str,
    recipient: &str,
    offset: usize,
    max_bytes: usize,
) -> Result<ProtectedBundleV2, String> {
    let (manifest, _, _) =
        load_session(ctx.conn, "out", id, recipient)?.ok_or("Snapshot prijenos nije pronađen")?;
    authorize(&ctx, recipient, manifest.format)?;
    if manifest.format == 2 {
        family_access(&ctx, &manifest)?;
        let proofs = crate::family::proofs(
            ctx.conn,
            ctx.network,
            recipient,
            &public_key_to_base64(ctx.network.as_bytes()),
            &manifest.channels,
            ctx.now,
        )?;
        if Some(hash_versions(&proofs)?.0) != manifest.proof_sha256 {
            return Err(
                "Kontrolne ovlasti su promijenjene; započnite novi obiteljski snapshot".into(),
            );
        }
    }
    let allowed = allowed_channels_for_recipient(ctx.conn, ctx.network, recipient, ctx.now)?;
    if !manifest.channels.is_subset(&allowed) {
        return Err("Prava su promijenjena; započnite novi snapshot".into());
    }
    let mut relayable_ids: BTreeSet<String> = if manifest.channels.contains("control") {
        crate::apply::relayable_control_versions(ctx.conn, ctx.network, ctx.now)?
            .into_iter()
            .map(|v| v.version_id)
            .collect()
    } else {
        BTreeSet::new()
    };
    relayable_ids.extend(
        crate::apply::relayable_domain_versions(
            ctx.conn,
            ctx.network,
            &manifest.channels,
            ctx.now,
        )?
        .into_iter()
        .map(|v| v.version_id),
    );
    let frozen = load_items(ctx.conn, "out", id, recipient, 0)?;
    if frozen
        .iter()
        .any(|v| !relayable_ids.contains(&v.version_id))
    {
        return Err("Kontrolne ovlasti su promijenjene; započnite novi snapshot".into());
    }
    if offset >= manifest.total_versions {
        return Err("Neispravan nastavak snapshota".into());
    }
    let count: u32 = ctx.conn.query_row("SELECT COUNT(*) FROM sync_snapshot_items WHERE direction='out' AND snapshot_id=?1 AND peer_key=?2",params![id,recipient],|r|r.get(0)).map_err(|e|e.to_string())?;
    if count as usize != manifest.total_versions {
        return Err("Oštećen lokalni snapshot prijenos".into());
    }
    let budget = delta_byte_budget(max_bytes);
    // Read at most one page's candidate data (plus its first overflowing row),
    // rather than loading the entire remaining snapshot on every request.
    let versions = {
        let mut stmt=ctx.conn.prepare("SELECT version_json FROM sync_snapshot_items WHERE direction='out' AND snapshot_id=?1 AND peer_key=?2 AND ordinal>=?3 ORDER BY ordinal").map_err(|e|e.to_string())?;
        let rows = stmt
            .query_map(params![id, recipient, offset as u32], |r| {
                r.get::<_, String>(0)
            })
            .map_err(|e| e.to_string())?;
        let mut versions = Vec::new();
        let mut bytes = 0;
        for row in rows {
            let json = row.map_err(|e| e.to_string())?;
            bytes += json.len();
            versions.push(serde_json::from_str::<Version>(&json).map_err(|e| e.to_string())?);
            if bytes >= budget {
                break;
            }
        }
        if versions.is_empty() {
            return Err("Nedostaju snapshot stranice".into());
        }
        versions
    };
    let envelope = |count: usize| SnapshotEnvelope {
        snapshot_page: SnapshotPage {
            manifest: manifest.clone(),
            offset,
            done: offset + count == manifest.total_versions,
            versions: versions[..count].to_vec(),
        },
    };
    if serialized_json_len(&envelope(1))? > budget {
        return Err("Prva snapshot verzija i manifest ne stanu u granicu stranice".into());
    }
    let (mut low, mut high) = (1, versions.len());
    while low < high {
        let middle = low + (high - low).div_ceil(2);
        if serialized_json_len(&envelope(middle))? <= budget {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    let plaintext = serde_json::to_vec(&envelope(low)).map_err(|e| e.to_string())?;
    if plaintext.len() > budget {
        return Err("Snapshot stranica prelazi granicu".into());
    }
    let key = parse_public_key(recipient).map_err(|e| e.to_string())?;
    ProtectedBundleV2::seal(PROTOCOL_NAME, ctx.key, key.as_bytes(), &plaintext)
        .map_err(|e| e.to_string())
}

fn load_items(
    conn: &Connection,
    direction: &str,
    id: &str,
    peer: &str,
    offset: usize,
) -> Result<Vec<Version>, String> {
    let mut stmt = conn.prepare("SELECT version_json FROM sync_snapshot_items WHERE direction=?1 AND snapshot_id=?2 AND peer_key=?3 AND ordinal>=?4 ORDER BY ordinal").map_err(|e| e.to_string())?;
    stmt.query_map(params![direction, id, peer, offset as u32], |r| {
        r.get::<_, String>(0)
    })
    .map_err(|e| e.to_string())?
    .map(|row| {
        row.map_err(|e| e.to_string())
            .and_then(|json| serde_json::from_str(&json).map_err(|e| e.to_string()))
    })
    .collect()
}

pub(crate) fn progress(
    conn: &Connection,
    sender: &str,
    id: &str,
) -> Result<Option<SnapshotProgress>, String> {
    Ok(
        load_session(conn, "in", id, sender)?.map(|(m, next_offset, complete)| SnapshotProgress {
            snapshot_id: m.snapshot_id,
            next_offset,
            total_versions: m.total_versions,
            complete,
        }),
    )
}

pub(crate) fn import_page(
    ctx: RecoveryContext<'_>,
    bundle: &ProtectedBundleV2,
) -> Result<SnapshotProgress, String> {
    let (plaintext, sender) = bundle
        .open(PROTOCOL_NAME, ctx.key)
        .map_err(|e| e.to_string())?;
    let sender = public_key_to_base64(&sender);
    let page: SnapshotPage = serde_json::from_slice::<SnapshotEnvelope>(&plaintext)
        .map_err(|e| e.to_string())?
        .snapshot_page;
    let m = &page.manifest;
    validate_manifest(m)?;
    authorize(&ctx, &sender, m.format)?;
    if m.format == 2 {
        // A new signed transfer may deliver a newly granted list. Staging is
        // bounded and grants are verified before final admission. An existing
        // transfer cannot continue after locally observed loss of access.
        if let Err(error) = family_access(&ctx, m) {
            let sender_allowed =
                allowed_channels_for_recipient(ctx.conn, ctx.network, &sender, ctx.now)?;
            if sender != public_key_to_base64(ctx.network.as_bytes())
                && !m.channels.is_subset(&sender_allowed)
            {
                return Err(error);
            }
        }
    }
    if m.sender != sender || m.recipient != public_key_to_base64(ctx.key.verifying_key().as_bytes())
    {
        return Err("Snapshot pripada drugom uređaju".into());
    }
    let end = page
        .offset
        .checked_add(page.versions.len())
        .ok_or("Neispravan nastavak")?;
    if page.versions.is_empty()
        || end > m.total_versions
        || page.done != (end == m.total_versions)
        || page
            .versions
            .iter()
            .any(|v| !m.channels.contains(&v.channel))
    {
        return Err("Neispravan redoslijed ili opseg snapshot stranice".into());
    }
    let tx = ctx
        .conn
        .unchecked_transaction()
        .map_err(|e| e.to_string())?;
    let session = load_session(&tx, "in", &m.snapshot_id, &sender)?;
    let next = if let Some((existing, next, complete)) = session {
        if existing != *m {
            return Err("Promijenjen manifest postojećeg snapshota".into());
        }
        if complete {
            return Ok(SnapshotProgress {
                snapshot_id: m.snapshot_id.clone(),
                next_offset: next,
                total_versions: m.total_versions,
                complete,
            });
        }
        if m.format == 2 {
            let baseline:Option<String>=tx.query_row("SELECT json_extract(manifest,'$._local_acl_hash') FROM sync_snapshot_sessions WHERE direction='in' AND snapshot_id=?1 AND peer_key=?2",params![m.snapshot_id,sender],|r|r.get(0)).map_err(|e|e.to_string())?;
            if baseline.as_deref() != Some(local_acl_hash(&tx)?.as_str()) {
                return Err(
                    "Lokalna prava su promijenjena; potreban je novi obiteljski snapshot".into(),
                );
            }
        }
        next
    } else {
        if page.offset != 0 {
            return Err("Nedostaje početak snapshot prijenosa".into());
        }
        if m.format == 2 {
            // A new source epoch replaces only this peer's incomplete staging.
            tx.execute("DELETE FROM sync_snapshot_items WHERE direction='in' AND peer_key=?1 AND snapshot_id IN (SELECT snapshot_id FROM sync_snapshot_sessions WHERE direction='in' AND peer_key=?1 AND complete=0 AND json_extract(manifest,'$.format')=2)",[&sender]).map_err(|e|e.to_string())?;
            tx.execute("DELETE FROM sync_snapshot_sessions WHERE direction='in' AND peer_key=?1 AND complete=0 AND json_extract(manifest,'$.format')=2",[&sender]).map_err(|e|e.to_string())?;
        }
        quota(&tx, "in")?;
        let mut stored_manifest = serde_json::to_value(m).map_err(|e| e.to_string())?;
        if m.format == 2 {
            stored_manifest["_local_acl_hash"] = serde_json::Value::String(local_acl_hash(&tx)?);
        }
        tx.execute("INSERT INTO sync_snapshot_sessions(direction,snapshot_id,peer_key,manifest) VALUES('in',?1,?2,?3)",params![m.snapshot_id,sender,serde_json::to_string(&stored_manifest).map_err(|e| e.to_string())?]).map_err(|e| e.to_string())?;
        0
    };
    if page.offset < next && end <= next {
        for (i, v) in page.versions.iter().enumerate() {
            let stored: String = tx.query_row("SELECT version_json FROM sync_snapshot_items WHERE direction='in' AND snapshot_id=?1 AND peer_key=?2 AND ordinal=?3",params![m.snapshot_id,sender,(page.offset+i) as u32],|r|r.get(0)).map_err(|e|e.to_string())?;
            if stored != serde_json::to_string(v).map_err(|e| e.to_string())? {
                return Err("Promijenjena ponovljena stranica".into());
            }
        }
        return Ok(SnapshotProgress {
            snapshot_id: m.snapshot_id.clone(),
            next_offset: next,
            total_versions: m.total_versions,
            complete: false,
        });
    }
    if page.offset != next {
        return Err("Stranica preskače nedostajući dio snapshota".into());
    }
    for (i, v) in page.versions.iter().enumerate() {
        tx.execute(
            "INSERT INTO sync_snapshot_items VALUES('in',?1,?2,?3,?4)",
            params![
                m.snapshot_id,
                sender,
                (page.offset + i) as u32,
                serde_json::to_string(v).map_err(|e| e.to_string())?
            ],
        )
        .map_err(|e| e.to_string())?;
    }
    let stored_bytes: i64 = tx.query_row("SELECT COALESCE(SUM(length(CAST(version_json AS BLOB))),0)+COUNT(*)+1 FROM sync_snapshot_items WHERE direction='in' AND snapshot_id=?1 AND peer_key=?2",params![m.snapshot_id,sender],|r|r.get(0)).map_err(|e|e.to_string())?;
    if stored_bytes > m.total_bytes as i64 {
        return Err("Snapshot prelazi deklariranu veličinu".into());
    }
    if page.done {
        let versions = load_items(&tx, "in", &m.snapshot_id, &sender, 0)?;
        let (hash, bytes) = hash_versions(&versions)?;
        if versions.len() != m.total_versions
            || hash != m.sha256
            || bytes != m.total_bytes
            || snapshot_frontier(&versions, m.format) != m.frontier
        {
            return Err("Snapshot nije potpun ili kontrolni sažetak nije valjan".into());
        }
        if m.format == 2
            && Some(
                hash_versions(
                    &versions
                        .iter()
                        .filter(|v| v.channel == "control")
                        .cloned()
                        .collect::<Vec<_>>(),
                )?
                .0,
            ) != m.proof_sha256
        {
            return Err("Neispravan sažetak kontrolnih dokaza".into());
        }
        let recipient = public_key_to_base64(ctx.key.verifying_key().as_bytes());
        let report = apply_remote_versions_in_transaction(
            &tx,
            ApplyContext {
                network_pub: ctx.network,
                peer_key: &sender,
                recipient_b64: Some(&recipient),
                now_unix: ctx.now,
            },
            &versions,
            &BTreeMap::new(),
            &BTreeMap::new(),
        )?;
        if !report.rejected.is_empty() {
            return Err(format!(
                "Snapshot ima {} odbijenih verzija; nije potvrđen",
                report.rejected.len()
            ));
        }
        authorize(&ctx, &sender, m.format)?;
        if m.format == 2 {
            family_access(&ctx, m)?;
        }
        let allowed = allowed_channels_for_recipient(&tx, ctx.network, &recipient, ctx.now)?;
        if !m.channels.is_subset(&allowed) {
            return Err("Primatelj više nema prava na snapshot".into());
        }
        for v in &versions {
            let pending: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM pending_projections WHERE version_id=?1)",
                    [&v.version_id],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if pending {
                return Err("Snapshot ima nedostajuće ovisnosti".into());
            }
        }
        for (key, to) in &m.frontier {
            let (author, channel) = crate::ledger::split_frontier_key(key);
            narrow_holes(&tx, author, channel, "", to)?;
            update_peer_ack(&tx, &sender, author, channel, to, to)?;
        }
        crate::backfill::acknowledge(&tx, &sender, &m.channels, &m.backfill_revisions)?;
        tx.execute("DELETE FROM sync_snapshot_items WHERE direction='in' AND snapshot_id=?1 AND peer_key=?2",params![m.snapshot_id,sender]).map_err(|e|e.to_string())?;
    }
    tx.execute("UPDATE sync_snapshot_sessions SET next_offset=?1,complete=?2 WHERE direction='in' AND snapshot_id=?3 AND peer_key=?4",params![end as u32,page.done,m.snapshot_id,sender]).map_err(|e|e.to_string())?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(SnapshotProgress {
        snapshot_id: m.snapshot_id.clone(),
        next_offset: end,
        total_versions: m.total_versions,
        complete: page.done,
    })
}

pub(crate) fn discard(
    conn: &Connection,
    id: &str,
    peer: &str,
    outgoing: bool,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM sync_snapshot_sessions WHERE direction=?1 AND snapshot_id=?2 AND peer_key=?3",
        params![if outgoing { "out" } else { "in" }, id, peer],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
