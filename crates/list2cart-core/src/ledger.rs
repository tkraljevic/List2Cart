use chrono::{Duration, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Version {
    pub version_id: String,
    pub entity: String,
    pub entity_id: String,
    pub node_id: String,
    pub supersedes: String,
    pub archived: bool,
    pub payload: String,
    pub created_at: String,
    pub schema_version: u32,
    pub channel: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerStats {
    pub node_id: String,
    pub network_name: String,
    pub total_versions: usize,
    pub unique_records: usize,
    pub tombstones: usize,
    pub frontier_nodes: usize,
    pub db_size_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerRole {
    /// Korisnikov vlastiti drugi uređaj (Laptop, PC, mobitel, Unraid) — dijeli SVE (račune, katalog, trgovine, liste).
    OwnDevice,
    /// Član obitelji — dijeli SAMO zajedničke liste za kupnju (kanal `obitelj`).
    FamilyMember,
}

impl PeerRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OwnDevice => "own_device",
            Self::FamilyMember => "family_member",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s.trim() {
            "family_member" => Self::FamilyMember,
            _ => Self::OwnDevice,
        }
    }

    pub fn channels(self) -> &'static str {
        match self {
            Self::OwnDevice => "osobno,obitelj",
            Self::FamilyMember => "obitelj",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkMember {
    pub peer_id: String,
    pub name: String,
    pub role: PeerRole,
    pub sas_code: String,
    pub channels: String,
    pub shared_lists: String,
    pub added_at: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct CompactionPolicy {
    /// Superseded versions older than this many days are removed.
    pub retention_days: i64,
    /// Even for recent edits, keep at most this many historical versions per (entity, entity_id).
    pub max_versions_per_record: usize,
}

impl Default for CompactionPolicy {
    fn default() -> Self {
        Self {
            retention_days: 30,
            max_versions_per_record: 3,
        }
    }
}

pub fn init_ledger_schema(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS record_versions (
            version_id TEXT PRIMARY KEY,
            entity TEXT NOT NULL,
            entity_id TEXT NOT NULL,
            node_id TEXT NOT NULL,
            supersedes TEXT NOT NULL DEFAULT '',
            archived INTEGER NOT NULL DEFAULT 0,
            payload TEXT NOT NULL,
            created_at TEXT NOT NULL,
            schema_version INTEGER NOT NULL DEFAULT 1,
            channel TEXT NOT NULL DEFAULT ''
        );
        CREATE INDEX IF NOT EXISTS idx_versions_entity
            ON record_versions(entity, entity_id, version_id);
        CREATE INDEX IF NOT EXISTS idx_versions_node
            ON record_versions(node_id, channel, version_id);

        CREATE TABLE IF NOT EXISTS network_members (
            peer_id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            role TEXT NOT NULL,
            sas_code TEXT NOT NULL,
            channels TEXT NOT NULL,
            shared_lists TEXT NOT NULL DEFAULT '*',
            added_at TEXT NOT NULL
        );
        ",
    )
    .map_err(|e| format!("Greska inicijalizacije knjige verzija: {e}"))
}

/// Generira 6-znamenkasti goCOP SAS kod za potvrdu uparivanja (npr. "482-915").
pub fn generate_sas_code() -> String {
    let raw = Uuid::new_v4().as_u128();
    let a = (raw % 900) + 100;
    let b = ((raw / 1000) % 900) + 100;
    format!("{a:03}-{b:03}")
}

pub fn add_network_member(
    conn: &Connection,
    name: &str,
    role: PeerRole,
    sas_code: &str,
    shared_lists: &str,
) -> Result<NetworkMember, String> {
    let peer_id = format!("peer-{}", &Uuid::new_v4().to_string()[..8]);
    let added_at = Utc::now().to_rfc3339();
    let channels = role.channels().to_owned();
    let clean_lists = if shared_lists.trim().is_empty() {
        "Sve obiteljske liste".to_owned()
    } else {
        shared_lists.trim().to_owned()
    };

    conn.execute(
        "INSERT INTO network_members (peer_id, name, role, sas_code, channels, shared_lists, added_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            peer_id,
            name.trim(),
            role.as_str(),
            sas_code.trim(),
            channels,
            clean_lists,
            added_at
        ],
    )
    .map_err(|e| format!("Greska dodavanja clana mreze: {e}"))?;

    Ok(NetworkMember {
        peer_id,
        name: name.trim().to_owned(),
        role,
        sas_code: sas_code.trim().to_owned(),
        channels,
        shared_lists: clean_lists,
        added_at,
    })
}

pub fn list_network_members(conn: &Connection) -> Result<Vec<NetworkMember>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT peer_id, name, role, sas_code, channels, shared_lists, added_at
             FROM network_members
             ORDER BY added_at ASC",
        )
        .map_err(|e| format!("Greska upita clanova mreze: {e}"))?;

    let rows = stmt
        .query_map([], |row| {
            let role_str: String = row.get(2)?;
            Ok(NetworkMember {
                peer_id: row.get(0)?,
                name: row.get(1)?,
                role: PeerRole::parse(&role_str),
                sas_code: row.get(3)?,
                channels: row.get(4)?,
                shared_lists: row.get(5)?,
                added_at: row.get(6)?,
            })
        })
        .map_err(|e| format!("Greska citanja clanova mreze: {e}"))?;

    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("Greska retka clana mreze: {e}"))?);
    }
    Ok(out)
}

pub fn remove_network_member(conn: &Connection, peer_id: &str) -> Result<(), String> {
    conn.execute(
        "DELETE FROM network_members WHERE peer_id = ?1",
        params![peer_id],
    )
    .map_err(|e| format!("Greska uklanjanja clana mreze: {e}"))?;
    Ok(())
}

pub fn frontier_key(node_id: &str, channel: &str) -> String {
    if channel.is_empty() {
        node_id.to_owned()
    } else {
        format!("{node_id}|{channel}")
    }
}

pub fn split_frontier_key(key: &str) -> (&str, &str) {
    match key.split_once('|') {
        Some((node, ch)) => (node, ch),
        None => (key, ""),
    }
}

/// Generates a strictly increasing UUIDv7 string even if called multiple times in the same millisecond.
pub fn next_monotonic_version_id(conn: &Connection) -> Result<String, String> {
    let candidate = Uuid::now_v7().to_string();
    let current_max: Option<String> = conn
        .query_row("SELECT MAX(version_id) FROM record_versions", [], |r| {
            r.get(0)
        })
        .optional()
        .map_err(|e| format!("Greska citanja zadnje verzije: {e}"))?
        .flatten();

    if let Some(max_id) = current_max
        && candidate <= max_id
    {
        return bump_uuid_str(&max_id);
    }
    Ok(candidate)
}

fn bump_uuid_str(id: &str) -> Result<String, String> {
    let parsed = Uuid::parse_str(id).map_err(|e| format!("Neispravan UUID: {e}"))?;
    let next_u128 = parsed.as_u128().saturating_add(1);
    Ok(Uuid::from_u128(next_u128).to_string())
}

pub fn record_version(
    conn: &Connection,
    node_id: &str,
    entity: &str,
    entity_id: &str,
    archived: bool,
    payload: &str,
    channel: &str,
) -> Result<Version, String> {
    let supersedes: String = conn
        .query_row(
            "SELECT COALESCE(MAX(version_id), '') FROM record_versions WHERE entity = ?1 AND entity_id = ?2",
            params![entity, entity_id],
            |r| r.get(0),
        )
        .map_err(|e| format!("Greska citanja prethodne verzije: {e}"))?;

    let version_id = next_monotonic_version_id(conn)?;
    let created_at = Utc::now().to_rfc3339();

    conn.execute(
        "INSERT INTO record_versions
         (version_id, entity, entity_id, node_id, supersedes, archived, payload, created_at, schema_version, channel)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, ?9)",
        params![
            version_id,
            entity,
            entity_id,
            node_id,
            supersedes,
            i32::from(archived),
            payload,
            created_at,
            channel
        ],
    )
    .map_err(|e| format!("Greska upisa u knjigu verzija: {e}"))?;

    Ok(Version {
        version_id,
        entity: entity.to_owned(),
        entity_id: entity_id.to_owned(),
        node_id: node_id.to_owned(),
        supersedes,
        archived,
        payload: payload.to_owned(),
        created_at,
        schema_version: 1,
        channel: channel.to_owned(),
    })
}

/// Returns the highest `version_id` per `(node_id, channel)` — the exact goCOP Frontier.
pub fn get_frontier(conn: &Connection) -> Result<BTreeMap<String, String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT node_id, channel, MAX(version_id)
             FROM record_versions
             GROUP BY node_id, channel",
        )
        .map_err(|e| format!("Greska upita granice (Frontier): {e}"))?;

    let rows = stmt
        .query_map([], |row| {
            let node: String = row.get(0)?;
            let ch: String = row.get(1)?;
            let max_v: String = row.get(2)?;
            Ok((frontier_key(&node, &ch), max_v))
        })
        .map_err(|e| format!("Greska citanja granice: {e}"))?;

    let mut out = BTreeMap::new();
    for r in rows {
        let (k, v) = r.map_err(|e| format!("Greska retka granice: {e}"))?;
        out.insert(k, v);
    }
    Ok(out)
}

/// Computes the exact delta of versions that `theirs` does not yet have.
pub fn compute_delta(
    conn: &Connection,
    theirs: &BTreeMap<String, String>,
    limit_per_node: usize,
) -> Result<Vec<Version>, String> {
    let mine = get_frontier(conn)?;
    let limit = if limit_per_node == 0 {
        1000
    } else {
        limit_per_node
    };
    let mut out = Vec::new();

    for (key, my_max) in mine {
        let their_max = theirs.get(&key).map(String::as_str).unwrap_or("");
        if their_max >= my_max.as_str() {
            continue;
        }
        let (node, channel) = split_frontier_key(&key);
        let mut stmt = conn
            .prepare(
                "SELECT version_id, entity, entity_id, node_id, supersedes, archived, payload, created_at, schema_version, channel
                 FROM record_versions
                 WHERE node_id = ?1 AND channel = ?2 AND version_id > ?3
                 ORDER BY version_id ASC
                 LIMIT ?4",
            )
            .map_err(|e| format!("Greska pripreme delte: {e}"))?;

        let rows = stmt
            .query_map(params![node, channel, their_max, limit as i64], |row| {
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
                })
            })
            .map_err(|e| format!("Greska citanja delte: {e}"))?;

        for row in rows {
            out.push(row.map_err(|e| format!("Greska retka delte: {e}"))?);
        }
    }

    out.sort_by(|a, b| a.version_id.cmp(&b.version_id));
    Ok(out)
}

/// Idempotently inserts incoming versions from another device into `record_versions`.
pub fn ingest_remote_versions(conn: &Connection, versions: &[Version]) -> Result<usize, String> {
    let mut inserted = 0usize;
    for v in versions {
        let rows = conn
            .execute(
                "INSERT OR IGNORE INTO record_versions
                 (version_id, entity, entity_id, node_id, supersedes, archived, payload, created_at, schema_version, channel)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
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
                ],
            )
            .map_err(|e| format!("Greska unosa udaljene verzije: {e}"))?;
        inserted += rows;
    }
    Ok(inserted)
}

/// Automatically compacts the version ledger using goCOP's two core invariants:
/// 1. Never delete `MAX(version_id)` for any `(entity, entity_id)` (preserves current surface & tombstones).
/// 2. Never delete `MAX(version_id)` for any `(node_id, channel)` (preserves sync Frontier so peers never re-send old versions).
///
/// Removes superseded versions that are either:
/// - older than `policy.retention_days`, OR
/// - beyond the newest `policy.max_versions_per_record` versions of the same `(entity, entity_id)`.
pub fn auto_compact(conn: &Connection, policy: CompactionPolicy) -> Result<usize, String> {
    let cutoff = (Utc::now() - Duration::days(policy.retention_days)).to_rfc3339();
    let keep_n = policy.max_versions_per_record.max(1) as i64;

    let deleted = conn
        .execute(
            "
            DELETE FROM record_versions
            WHERE version_id NOT IN (
                SELECT MAX(version_id) FROM record_versions GROUP BY entity, entity_id
            )
            AND version_id NOT IN (
                SELECT MAX(version_id) FROM record_versions GROUP BY node_id, channel
            )
            AND (
                created_at < ?1
                OR version_id NOT IN (
                    SELECT v2.version_id
                    FROM record_versions v2
                    WHERE v2.entity = record_versions.entity
                      AND v2.entity_id = record_versions.entity_id
                    ORDER BY v2.version_id DESC
                    LIMIT ?2
                )
            )
            ",
            params![cutoff, keep_n],
        )
        .map_err(|e| format!("Greska automatskog sazimanja knjige verzija: {e}"))?;

    if deleted > 0 {
        vacuum_db(conn)?;
    }

    Ok(deleted)
}

/// Reclaims unused SQLite pages (`VACUUM` + `PRAGMA optimize`) so the physical `.sqlite` file shrinks on disk.
pub fn vacuum_db(conn: &Connection) -> Result<(), String> {
    conn.execute_batch("VACUUM; PRAGMA optimize;")
        .map_err(|e| format!("Greska VACUUM optimizacije baze: {e}"))
}

pub fn get_stats(
    conn: &Connection,
    node_id: &str,
    network_name: &str,
) -> Result<LedgerStats, String> {
    let total_versions: i64 = conn
        .query_row("SELECT COUNT(*) FROM record_versions", [], |r| r.get(0))
        .map_err(|e| format!("Greska statistike: {e}"))?;

    let unique_records: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM (SELECT DISTINCT entity, entity_id FROM record_versions)",
            [],
            |r| r.get(0),
        )
        .map_err(|e| format!("Greska statistike: {e}"))?;

    let tombstones: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM record_versions r
             WHERE r.archived = 1
               AND r.version_id = (
                   SELECT MAX(v.version_id) FROM record_versions v
                   WHERE v.entity = r.entity AND v.entity_id = r.entity_id
               )",
            [],
            |r| r.get(0),
        )
        .map_err(|e| format!("Greska statistike: {e}"))?;

    let page_count: i64 = conn
        .query_row("PRAGMA page_count", [], |r| r.get(0))
        .unwrap_or(0);
    let page_size: i64 = conn
        .query_row("PRAGMA page_size", [], |r| r.get(0))
        .unwrap_or(4096);
    let freelist_count: i64 = conn
        .query_row("PRAGMA freelist_count", [], |r| r.get(0))
        .unwrap_or(0);
    let used_pages = (page_count - freelist_count).max(0) as u64;
    let db_size_bytes = used_pages.saturating_mul(page_size.max(0) as u64);

    let frontier = get_frontier(conn)?;

    Ok(LedgerStats {
        node_id: node_id.to_owned(),
        network_name: network_name.to_owned(),
        total_versions: total_versions as usize,
        unique_records: unique_records as usize,
        tombstones: tombstones as usize,
        frontier_nodes: frontier.len(),
        db_size_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_compact_preserves_latest_tombstones_and_frontier() {
        let conn = Connection::open_in_memory().expect("mem db");
        init_ledger_schema(&conn).expect("schema");

        // Node A writes 5 versions of product p-1
        for price in [110, 120, 130, 140, 150] {
            record_version(
                &conn,
                "node-a",
                "products",
                "p-1",
                false,
                &format!("{{\"price\":{price}}}"),
                "",
            )
            .expect("rec");
        }

        // Node B writes 1 version of p-1 earlier, then Node A overwrote p-1.
        // Even if Node B's version of p-1 is superseded by Node A, it is Node B's ONLY version,
        // so Frontier invariant MUST keep it!
        let v_b = record_version(
            &conn,
            "node-b",
            "products",
            "p-1",
            false,
            "{\"price\":155}",
            "",
        )
        .expect("rec b");
        let v_a_final = record_version(
            &conn,
            "node-a",
            "products",
            "p-1",
            false,
            "{\"price\":160}",
            "",
        )
        .expect("rec a final");

        // Also create and archive (delete) p-2
        record_version(&conn, "node-a", "products", "p-2", false, "{}", "").expect("p2");
        let v_p2_tomb =
            record_version(&conn, "node-a", "products", "p-2", true, "{}", "").expect("p2 tomb");

        let frontier_before = get_frontier(&conn).expect("frontier before");

        // Compact keeping at most 2 versions per record
        let deleted = auto_compact(
            &conn,
            CompactionPolicy {
                retention_days: 30,
                max_versions_per_record: 2,
            },
        )
        .expect("compact");
        assert!(deleted >= 4);

        let frontier_after = get_frontier(&conn).expect("frontier after");
        assert_eq!(frontier_before, frontier_after);
        assert_eq!(
            frontier_after.get("node-b").map(String::as_str),
            Some(v_b.version_id.as_str())
        );
        assert_eq!(
            frontier_after.get("node-a").map(String::as_str),
            Some(v_p2_tomb.version_id.as_str())
        );
        let _ = v_a_final;

        let stats = get_stats(&conn, "node-a", "Obitelj Kraljevic").expect("stats");
        assert_eq!(stats.unique_records, 2);
        assert_eq!(stats.tombstones, 1);
        assert!(stats.db_size_bytes > 0);

        let freelist: i64 = conn
            .query_row("PRAGMA freelist_count", [], |r| r.get(0))
            .expect("freelist");
        assert_eq!(freelist, 0);
    }
}
