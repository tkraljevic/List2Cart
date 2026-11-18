//! Source-local channel revisions invalidate MAX-only knowledge after remote admission.
use rusqlite::{Connection, params};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

pub type BackfillRevisions = BTreeMap<String, String>;

pub(crate) fn current(
    conn: &Connection,
    allowed: &BTreeSet<String>,
) -> Result<BackfillRevisions, String> {
    let mut stmt = conn
        .prepare("SELECT channel,revision FROM sync_backfill_revisions")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?;
    let mut result = BTreeMap::new();
    for row in rows {
        let (channel, revision) = row.map_err(|e| e.to_string())?;
        if allowed.contains(&channel) {
            result.insert(channel, revision);
        }
    }
    Ok(result)
}

pub(crate) fn acknowledged(conn: &Connection, peer: &str) -> Result<BackfillRevisions, String> {
    let mut stmt = conn
        .prepare(
            "SELECT channel,revision FROM sync_backfill_ack WHERE peer_key=?1 AND revision!=''",
        )
        .map_err(|e| e.to_string())?;
    stmt.query_map([peer], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })
    .map_err(|e| e.to_string())?
    .map(|r| r.map_err(|e| e.to_string()))
    .collect()
}

pub(crate) fn needs_recovery(
    conn: &Connection,
    allowed: &BTreeSet<String>,
    peer: &BackfillRevisions,
) -> Result<bool, String> {
    let local = current(conn, allowed)?;
    Ok(allowed.iter().any(|channel| {
        local.get(channel).map(String::as_str).unwrap_or("")
            != peer.get(channel).map(String::as_str).unwrap_or("")
    }))
}

pub(crate) fn mark_channels(conn: &Connection, channels: &BTreeSet<String>) -> Result<(), String> {
    if conn.is_autocommit() {
        return Err("Oznaka dopune zahtijeva transakciju".into());
    }
    for channel in channels {
        conn.execute("INSERT INTO sync_backfill_revisions(channel,revision) VALUES(?1,?2) ON CONFLICT(channel) DO UPDATE SET revision=excluded.revision",params![channel,Uuid::new_v4().to_string()]).map_err(|e|format!("Greska oznake naknadne dopune: {e}"))?;
    }
    Ok(())
}

pub(crate) fn validate(revisions: &BackfillRevisions) -> bool {
    revisions.len() <= 4096
        && revisions.iter().all(|(channel, revision)| {
            channel.len() <= 256
                && (revision.is_empty()
                    || (revision.len() == 36
                        && Uuid::parse_str(revision).is_ok_and(|id| id.to_string() == *revision)))
        })
}

pub(crate) fn acknowledge(
    conn: &Connection,
    peer: &str,
    channels: &BTreeSet<String>,
    revisions: &BackfillRevisions,
) -> Result<(), String> {
    if conn.is_autocommit() {
        return Err("Potvrda dopune zahtijeva transakciju".into());
    }
    if !validate(revisions) || revisions.keys().any(|channel| !channels.contains(channel)) {
        return Err("Neispravne oznake dopune snapshota".into());
    }
    for channel in channels {
        conn.execute("INSERT INTO sync_backfill_ack(peer_key,channel,revision) VALUES(?1,?2,?3) ON CONFLICT(peer_key,channel) DO UPDATE SET revision=excluded.revision",params![peer,channel,revisions.get(channel).map(String::as_str).unwrap_or("")]).map_err(|e|format!("Greska potvrde naknadne dopune: {e}"))?;
    }
    Ok(())
}
