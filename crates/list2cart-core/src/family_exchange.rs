//! Conservative family dispatcher: immutable snapshots on changes, tiny idle replies.
use crate::apply::{PROTOCOL_NAME, ProtectedSyncPayload};
use crate::exchange::SyncRequest;
use crate::ledger::{
    DeltaBatch, get_confirmed_frontier, get_frontier, list_holes, split_frontier_key,
};
use crate::recovery::{self, RecoveryContext};
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::Signer;
use list2cart_sync::identity::{parse_public_key, public_key_to_base64};
use list2cart_sync::wire::bundle_v2::ProtectedBundleV2;
use rusqlite::{OptionalExtension, params};
use std::collections::BTreeMap;
use uuid::Uuid;

pub(crate) fn request(ctx: RecoveryContext<'_>, peer: &str) -> Result<SyncRequest, String> {
    let allowed = recovery::family_scope(&ctx, peer)?;
    let mut frontier = get_confirmed_frontier(ctx.conn, peer)?;
    frontier.retain(|k, _| {
        let ch = split_frontier_key(k).1;
        ch != "control" && allowed.contains(ch)
    });
    let holes = list_holes(ctx.conn)?
        .into_iter()
        .filter(|h| h.channel != "control" && allowed.contains(&h.channel))
        .collect();
    let latest:Option<String>=ctx.conn.query_row("SELECT snapshot_id FROM sync_snapshot_sessions WHERE direction='in' AND peer_key=?1 AND json_extract(manifest,'$.format')=2 ORDER BY rowid DESC LIMIT 1",[peer],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
    let mut progress = latest
        .map(|id| recovery::progress(ctx.conn, peer, &id))
        .transpose()?
        .flatten();
    if let Some(current) = &progress
        && !current.complete
    {
        let baseline:Option<String>=ctx.conn.query_row("SELECT json_extract(manifest,'$._local_acl_hash') FROM sync_snapshot_sessions WHERE direction='in' AND snapshot_id=?1 AND peer_key=?2",params![current.snapshot_id,peer],|r|r.get(0)).map_err(|e|e.to_string())?;
        if baseline.as_deref() != Some(recovery::local_acl_hash(ctx.conn)?.as_str()) {
            recovery::discard(ctx.conn, &current.snapshot_id, peer, false)?;
            progress = None;
        }
    }
    let mut revisions = crate::backfill::acknowledged(ctx.conn, peer)?;
    revisions.retain(|ch, _| ch != "control" && allowed.contains(ch));
    let mut request = SyncRequest {
        format: 3,
        sender: public_key_to_base64(ctx.key.verifying_key().as_bytes()),
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
    if crate::ledger::serialized_json_len(&request)? > 65536 {
        return Err("Znanje za razmjenu prelazi 64 KiB".into());
    }
    Ok(request)
}

pub(crate) fn reply(
    ctx: RecoveryContext<'_>,
    request: &SyncRequest,
    budget: usize,
) -> Result<ProtectedBundleV2, String> {
    let peer = &request.sender;
    let channels = recovery::family_scope(&ctx, peer)?;
    let proofs = crate::family::proofs(
        ctx.conn,
        peer,
        &public_key_to_base64(ctx.network.as_bytes()),
        &channels,
    )?;
    let proof_hash = Some(recovery::hash_versions(&proofs)?.0);
    let matches = |m: &recovery::SnapshotManifest| {
        m.format == 2 && m.channels == channels && m.proof_sha256 == proof_hash
    };
    let mut confirmed = false;
    if let Some(progress) = &request.recovery
        && let Some((manifest, _, complete)) =
            recovery::load_session(ctx.conn, "out", &progress.snapshot_id, peer)?
    {
        if manifest.format != 2
            || progress.total_versions != manifest.total_versions
            || progress.next_offset > manifest.total_versions
            || progress.complete != (progress.next_offset == manifest.total_versions)
        {
            return Err("Neispravna potvrda obiteljskog snapshota".into());
        }
        if progress.complete {
            let tx = ctx
                .conn
                .unchecked_transaction()
                .map_err(|e| e.to_string())?;
            tx.execute("UPDATE sync_snapshot_sessions SET next_offset=?1,complete=1 WHERE direction='out' AND snapshot_id=?2 AND peer_key=?3",params![progress.next_offset as u32,progress.snapshot_id,peer]).map_err(|e|e.to_string())?;
            tx.execute("DELETE FROM sync_snapshot_items WHERE direction='out' AND snapshot_id=?1 AND peer_key=?2",params![progress.snapshot_id,peer]).map_err(|e|e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
            confirmed = matches(&manifest);
        } else if !complete && matches(&manifest) {
            return recovery::export_page(
                ctx,
                &progress.snapshot_id,
                peer,
                progress.next_offset,
                budget,
            );
        } else if !complete {
            recovery::discard(ctx.conn, &progress.snapshot_id, peer, true)?;
        }
    }
    let active: Vec<String> = {
        let mut stmt=ctx.conn.prepare("SELECT snapshot_id FROM sync_snapshot_sessions WHERE direction='out' AND peer_key=?1 AND complete=0 AND json_extract(manifest,'$.format')=2 ORDER BY rowid DESC").map_err(|e|e.to_string())?;
        stmt.query_map([peer], |r| r.get(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<_, _>>()
            .map_err(|e| e.to_string())?
    };
    for id in active {
        let (manifest, _, _) =
            recovery::load_session(ctx.conn, "out", &id, peer)?.ok_or("Nedostaje snapshot")?;
        if matches(&manifest) {
            return recovery::export_page(ctx, &id, peer, 0, budget);
        }
        recovery::discard(ctx.conn, &id, peer, true)?;
    }
    let domains: std::collections::BTreeSet<String> = channels
        .iter()
        .filter(|ch| ch.as_str() != "control")
        .cloned()
        .collect();
    let newer = get_frontier(ctx.conn)?.iter().any(|(key, value)| {
        domains.contains(split_frontier_key(key).1)
            && request.frontier.get(key).is_none_or(|old| value > old)
    });
    if !confirmed
        || newer
        || !crate::ledger::recovery_keys(ctx.conn, &request.frontier, &request.holes, &domains)?
            .is_empty()
        || crate::backfill::needs_recovery(ctx.conn, &domains, &request.backfill_revisions)?
    {
        let manifest = recovery::start_family(ctx, peer)?;
        return recovery::export_page(ctx, &manifest.snapshot_id, peer, 0, budget);
    }
    let inner = ProtectedSyncPayload {
        sender_node_id: public_key_to_base64(ctx.key.verifying_key().as_bytes()),
        sender_pub_key: public_key_to_base64(ctx.key.verifying_key().as_bytes()),
        batch: DeltaBatch {
            versions: Vec::new(),
            covered_ranges: BTreeMap::new(),
            sender_holes: BTreeMap::new(),
            has_more: false,
            snapshot_reset_keys: Vec::new(),
            snapshot_versions: Vec::new(),
        },
    };
    let plain = serde_json::to_vec(&inner).map_err(|e| e.to_string())?;
    if plain.len() > crate::ledger::delta_byte_budget(budget) {
        return Err("Odgovor ne stane u granicu stranice".into());
    }
    ProtectedBundleV2::seal(
        PROTOCOL_NAME,
        ctx.key,
        parse_public_key(peer)
            .map_err(|e| e.to_string())?
            .as_bytes(),
        &plain,
    )
    .map_err(|e| e.to_string())
}
