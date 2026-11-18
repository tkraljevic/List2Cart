//! Transport-independent authenticated request -> delta/recovery selection.
use crate::apply::{
    ApplyReport, ExportRequest, PROTOCOL_NAME, ProtectedSyncPayload,
    allowed_channels_for_recipient, export_protected_bundle_for_recipient, import_protected_bundle,
};
use crate::ledger::{
    HoleRecord, get_confirmed_frontier, list_holes, recovery_keys, serialized_json_len,
};
use crate::recovery::{self, RecoveryContext, SnapshotProgress};
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signature, Signer};
use list2cart_sync::identity::{parse_public_key, public_key_to_base64};
use list2cart_sync::wire::bundle_v2::ProtectedBundleV2;
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncRequest {
    pub format: u8,
    pub sender: String,
    pub recipient: String,
    pub created_at: i64,
    pub nonce: String,
    pub frontier: BTreeMap<String, String>,
    pub holes: Vec<HoleRecord>,
    pub recovery: Option<SnapshotProgress>,
    #[serde(default)]
    pub backfill_revisions: crate::backfill::BackfillRevisions,
    pub signature: String,
}

impl SyncRequest {
    pub(crate) fn signed_bytes(&self) -> Result<Vec<u8>, String> {
        let mut bytes = if self.format == 3 {
            b"list2cart-family-request-v3|".to_vec()
        } else {
            b"list2cart-request-v2|".to_vec()
        };
        bytes.extend(
            serde_json::to_vec(&(
                self.format,
                &self.sender,
                &self.recipient,
                self.created_at,
                &self.nonce,
                &self.frontier,
                &self.holes,
                &self.recovery,
                &self.backfill_revisions,
            ))
            .map_err(|e| e.to_string())?,
        );
        Ok(bytes)
    }
    pub(crate) fn validate(&self, ctx: &RecoveryContext<'_>) -> Result<(), String> {
        if serialized_json_len(self)? > 65536
            || !matches!(self.format, 2 | 3)
            || (self.format == 3
                && (self
                    .frontier
                    .keys()
                    .any(|k| !crate::ledger::split_frontier_key(k).1.starts_with("list:"))
                    || self.holes.iter().any(|h| !h.channel.starts_with("list:"))
                    || self
                        .backfill_revisions
                        .keys()
                        .any(|ch| !ch.starts_with("list:"))))
            || !crate::backfill::validate(&self.backfill_revisions)
            || self.created_at.abs_diff(ctx.now) > 600
            || Uuid::parse_str(&self.nonce).is_err()
            || self.recipient != public_key_to_base64(ctx.key.verifying_key().as_bytes())
            || self.sender == self.recipient
            || self.frontier.len() > 4096
            || self.holes.len() > 4096
            || self
                .frontier
                .iter()
                .any(|(k, v)| k.len() > 256 || v.len() > 36)
            || self.holes.iter().any(|h| {
                h.author_node_id.len() > 44
                    || h.channel.len() > 256
                    || h.from_version_id.len() > 36
                    || h.to_version_id.len() > 36
            })
        {
            return Err("Neispravan, zastario ili prevelik zahtjev za razmjenu".into());
        }
        let key = parse_public_key(&self.sender).map_err(|e| e.to_string())?;
        if public_key_to_base64(key.as_bytes()) != self.sender {
            return Err("Nekanonski ključ zahtjeva".into());
        }
        let signature = STANDARD
            .decode(&self.signature)
            .map_err(|e| e.to_string())?;
        let signature = Signature::from_slice(&signature).map_err(|e| e.to_string())?;
        key.verify_strict(&self.signed_bytes()?, &signature)
            .map_err(|_| "Neispravan potpis zahtjeva".to_owned())?;
        if self.format == 3 {
            recovery::family_peer(ctx, &self.sender)
        } else {
            recovery::own_peer(ctx, &self.sender)
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SyncReceiveOutcome {
    Delta { report: ApplyReport, has_more: bool },
    Snapshot { progress: SnapshotProgress },
}

pub(crate) fn request(ctx: RecoveryContext<'_>, peer: &str) -> Result<SyncRequest, String> {
    if recovery::own_peer(&ctx, peer).is_err() {
        return crate::family_exchange::request(ctx, peer);
    }
    recovery::own_peer(&ctx, peer)?;
    let local = public_key_to_base64(ctx.key.verifying_key().as_bytes());
    if peer == local {
        return Err("Uređaj se ne može sinkronizirati sam sa sobom".into());
    }
    let allowed = allowed_channels_for_recipient(ctx.conn, ctx.network, peer, ctx.now)?;
    let local_allowed = allowed_channels_for_recipient(ctx.conn, ctx.network, &local, ctx.now)?;
    let allowed: std::collections::BTreeSet<_> =
        allowed.intersection(&local_allowed).cloned().collect();
    let mut frontier = get_confirmed_frontier(ctx.conn, peer)?;
    frontier.retain(|key, _| allowed.contains(crate::ledger::split_frontier_key(key).1));
    let holes = list_holes(ctx.conn)?
        .into_iter()
        .filter(|h| allowed.contains(&h.channel))
        .collect();
    // The newest session wins even when complete: older abandoned sessions must
    // not regain priority after a replacement snapshot has finished.
    let latest:Option<String>=ctx.conn.query_row("SELECT snapshot_id FROM sync_snapshot_sessions WHERE direction='in' AND peer_key=?1 ORDER BY rowid DESC LIMIT 1",[peer],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
    let progress = latest
        .map(|id| recovery::progress(ctx.conn, peer, &id))
        .transpose()?
        .flatten();
    let mut revisions = crate::backfill::acknowledged(ctx.conn, peer)?;
    revisions.retain(|channel, _| allowed.contains(channel));
    let mut request = SyncRequest {
        format: 2,
        sender: local,
        recipient: peer.into(),
        created_at: ctx.now,
        nonce: Uuid::new_v4().to_string(),
        frontier,
        holes,
        recovery: progress,
        backfill_revisions: revisions,
        signature: String::new(),
    };
    request.signature = STANDARD.encode(ctx.key.sign(&request.signed_bytes()?).to_bytes());
    if serialized_json_len(&request)? > 65536 {
        return Err("Znanje za razmjenu prelazi 64 KiB".into());
    }
    Ok(request)
}

pub(crate) fn reply(
    ctx: RecoveryContext<'_>,
    request: &SyncRequest,
    max_bytes: usize,
) -> Result<ProtectedBundleV2, String> {
    request.validate(&ctx)?;
    if request.format == 3 {
        return crate::family_exchange::reply(ctx, request, max_bytes);
    }
    let peer = &request.sender;
    let mut replace_lost_snapshot = false;
    if let Some(progress) = &request.recovery {
        if let Some((manifest, _, already_complete)) =
            recovery::load_session(ctx.conn, "out", &progress.snapshot_id, peer)?
        {
            if progress.total_versions != manifest.total_versions
                || progress.next_offset > manifest.total_versions
                || progress.complete != (progress.next_offset == manifest.total_versions)
            {
                return Err("Neispravna potvrda nastavka snapshota".into());
            }
            if progress.complete {
                let tx = ctx
                    .conn
                    .unchecked_transaction()
                    .map_err(|e| e.to_string())?;
                tx.execute("UPDATE sync_snapshot_sessions SET next_offset=?1,complete=1 WHERE direction='out' AND snapshot_id=?2 AND peer_key=?3",params![progress.next_offset as u32,progress.snapshot_id,peer]).map_err(|e|e.to_string())?;
                // Only a signed post-commit receipt permits releasing the frozen copy.
                tx.execute("DELETE FROM sync_snapshot_items WHERE direction='out' AND snapshot_id=?1 AND peer_key=?2",params![progress.snapshot_id,peer]).map_err(|e|e.to_string())?;
                tx.commit().map_err(|e| e.to_string())?;
            } else if !already_complete {
                ctx.conn.execute("UPDATE sync_snapshot_sessions SET next_offset=MAX(next_offset,?1) WHERE direction='out' AND snapshot_id=?2 AND peer_key=?3",params![progress.next_offset as u32,progress.snapshot_id,peer]).map_err(|e|e.to_string())?;
                return recovery::export_page(
                    ctx,
                    &progress.snapshot_id,
                    peer,
                    progress.next_offset,
                    max_bytes,
                );
            } else {
                replace_lost_snapshot = true;
            }
        } else if !progress.complete {
            replace_lost_snapshot = true;
        }
    }
    // Reuse an unacknowledged frozen copy after a lost first response/restart.
    let active:Option<String>=ctx.conn.query_row("SELECT snapshot_id FROM sync_snapshot_sessions WHERE direction='out' AND peer_key=?1 AND complete=0 ORDER BY rowid DESC LIMIT 1",[peer],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
    if let Some(id) = active {
        return recovery::export_page(ctx, &id, peer, 0, max_bytes);
    }
    let allowed = allowed_channels_for_recipient(ctx.conn, ctx.network, peer, ctx.now)?;
    if replace_lost_snapshot
        || !recovery_keys(ctx.conn, &request.frontier, &request.holes, &allowed)?.is_empty()
        || crate::backfill::needs_recovery(ctx.conn, &allowed, &request.backfill_revisions)?
    {
        let manifest = recovery::start(ctx, peer)?;
        return recovery::export_page(ctx, &manifest.snapshot_id, peer, 0, max_bytes);
    }
    let recipient = parse_public_key(peer).map_err(|e| e.to_string())?;
    export_protected_bundle_for_recipient(
        ctx.conn,
        ExportRequest {
            local_node_id: &public_key_to_base64(ctx.key.verifying_key().as_bytes()),
            sender_sk: ctx.key,
            network_pub: ctx.network,
            recipient_pub: &recipient,
            peer_frontier: &request.frontier,
            requested_holes: &request.holes,
            max_bytes,
            now_unix: ctx.now,
        },
    )
}

pub(crate) fn receive(
    ctx: RecoveryContext<'_>,
    bundle: &ProtectedBundleV2,
) -> Result<SyncReceiveOutcome, String> {
    let (plaintext, sender) = bundle
        .open(PROTOCOL_NAME, ctx.key)
        .map_err(|e| e.to_string())?;
    let sender = public_key_to_base64(&sender);
    if recovery::own_peer(&ctx, &sender).is_err() {
        recovery::family_peer(&ctx, &sender)?;
    }
    let value: serde_json::Value = serde_json::from_slice(&plaintext).map_err(|e| e.to_string())?;
    if value.get("snapshot_page").is_some() {
        Ok(SyncReceiveOutcome::Snapshot {
            progress: recovery::import_page(ctx, bundle)?,
        })
    } else {
        let payload: ProtectedSyncPayload =
            serde_json::from_value(value).map_err(|e| e.to_string())?;
        if !payload.batch.snapshot_reset_keys.is_empty()
            || !payload.batch.snapshot_versions.is_empty()
        {
            return Err("Automatski oporavak zahtijeva snapshot stranice s manifestom".into());
        }
        let has_more = payload.batch.has_more;
        let report = import_protected_bundle(
            ctx.conn,
            ctx.key,
            ctx.network,
            ctx.principal,
            bundle,
            ctx.now,
        )?;
        Ok(SyncReceiveOutcome::Delta { report, has_more })
    }
}
