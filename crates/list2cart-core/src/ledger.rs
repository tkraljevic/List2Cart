use chrono::{Duration, Utc};
use ed25519_dalek::SigningKey;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use syncnet::identity::public_key_to_base64;
use syncnet::trust::{MAX_CLOCK_SKEW_SECS, SignedVersionEnvelope};
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
    #[serde(default)]
    pub author_pub_key: String,
    #[serde(default)]
    pub grant_ref: String,
    #[serde(default)]
    pub signature: String,
    #[serde(default)]
    pub created_at_unix: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VersionRange {
    pub from_id: String,
    pub to_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HoleRecord {
    pub author_node_id: String,
    pub channel: String,
    pub from_version_id: String,
    pub to_version_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeltaBatch {
    pub versions: Vec<Version>,
    pub covered_ranges: BTreeMap<String, VersionRange>,
    pub sender_holes: BTreeMap<String, VersionRange>,
    pub has_more: bool,
    #[serde(default)]
    pub snapshot_reset_keys: Vec<String>,
    #[serde(default)]
    pub snapshot_versions: Vec<Version>,
}

/// Count the actual UTF-8 JSON representation without allocating another copy.
pub(crate) fn serialized_json_len(value: &impl Serialize) -> Result<usize, String> {
    struct Counter(usize);
    impl std::io::Write for Counter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0 = self
                .0
                .checked_add(bytes.len())
                .ok_or_else(|| std::io::Error::other("JSON size overflow"))?;
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut counter = Counter(0);
    serde_json::to_writer(&mut counter, value)
        .map_err(|e| format!("Greska mjerenja JSON-a: {e}"))?;
    Ok(counter.0)
}

pub(crate) fn delta_byte_budget(max_bytes: usize) -> usize {
    (if max_bytes == 0 {
        256 * 1024
    } else {
        max_bytes
    })
    .min(syncnet::wire::bundle_v2::MAX_BUNDLE_PLAINTEXT)
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

/// Checks whether a shopping list (`list_name` or `list_sync_id`) is permitted by a `shared_lists` filter.
pub fn shared_lists_allows(filter: &str, list_name: &str, list_sync_id: &str) -> bool {
    let trimmed = filter.trim();
    if trimmed.is_empty()
        || trimmed == "*"
        || trimmed.eq_ignore_ascii_case("sve")
        || trimmed.eq_ignore_ascii_case("sve obiteljske liste")
    {
        return true;
    }
    let target_name = list_name.trim().to_lowercase();
    let target_id = list_sync_id.trim().to_lowercase();
    trimmed
        .split([',', ';'])
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .any(|token| {
            token == "*"
                || token == "sve"
                || token == "sve obiteljske liste"
                || token == target_name
                || token == target_id
        })
}

/// Legacy v1 bundle role helper: NEVER grants network trust or modifies `sync_trust_records`.
/// Any unrecognized legacy JSON bundle is treated strictly as local-only import (`FamilyMember`
/// if explicitly restricted, otherwise no trust upgrade).
pub fn verify_peer_identity(
    conn: &Connection,
    local_node_id: &str,
    source_node_id: &str,
    sas_code: Option<&str>,
    claimed_role: Option<PeerRole>,
) -> Result<(PeerRole, Option<String>), String> {
    let members = list_network_members(conn)?;
    let clean_sas = sas_code.map(str::trim).filter(|s| !s.is_empty());

    if let Some(code) = clean_sas {
        if let Some(m) = members
            .iter()
            .find(|m| m.sas_code.eq_ignore_ascii_case(code))
        {
            return Ok((m.role, Some(m.shared_lists.clone())));
        }
        if !members.is_empty() {
            return Err(format!(
                "Nepoznat ili neispravan SAS kod paketa ({code}). Potvrdite SAS kod u postavkama sinkronizacije."
            ));
        }
    }

    if !source_node_id.is_empty()
        && source_node_id != local_node_id
        && let Some(m) = members.iter().find(|m| m.peer_id == source_node_id)
    {
        return Ok((m.role, Some(m.shared_lists.clone())));
    }

    Ok((claimed_role.unwrap_or(PeerRole::FamilyMember), None))
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

/// Generates a UUIDv7 after the known ledger, retaining entropy across independent nodes.
pub fn next_monotonic_version_id(conn: &Connection) -> Result<String, String> {
    next_version_uuid(conn, false).map(|id| id.to_string())
}

fn next_version_uuid(conn: &Connection, signed_only: bool) -> Result<Uuid, String> {
    let candidate = Uuid::now_v7();
    let query = if signed_only {
        "SELECT MAX(version_id) FROM record_versions WHERE signature != ''"
    } else {
        "SELECT MAX(version_id) FROM record_versions"
    };
    let current_max: Option<String> = conn
        .query_row(query, [], |r| r.get(0))
        .optional()
        .map_err(|e| format!("Greska citanja zadnje verzije: {e}"))?
        .flatten();

    if let Some(max_id) = current_max
        && candidate.to_string() <= max_id
    {
        let previous = Uuid::parse_str(&max_id).map_err(|e| format!("Neispravan UUID: {e}"))?;
        if previous.get_version_num() != 7 {
            return Err("Prethodna verzija nije UUIDv7".into());
        }
        // Increment the 48-bit logical millisecond, not the entire UUID: two
        // replicas following the same version must keep independent suffixes.
        let mut clock = [0u8; 8];
        clock[2..].copy_from_slice(&previous.as_bytes()[..6]);
        let next_ms = u64::from_be_bytes(clock) + 1;
        if next_ms >= 1 << 48 {
            return Err("UUIDv7 vrijeme je iscrpljeno".into());
        }
        let mut bytes = *candidate.as_bytes();
        bytes[..6].copy_from_slice(&next_ms.to_be_bytes()[2..]);
        return Ok(Uuid::from_bytes(bytes));
    }
    Ok(candidate)
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
    record_version_with_key(
        conn, node_id, None, entity, entity_id, archived, payload, channel, "",
    )
}

#[allow(clippy::too_many_arguments)]
pub fn record_signed_version(
    conn: &Connection,
    node_id: &str,
    signing_key: &SigningKey,
    entity: &str,
    entity_id: &str,
    archived: bool,
    payload: &str,
    channel: &str,
    grant_ref: &str,
) -> Result<Version, String> {
    record_version_with_key(
        conn,
        node_id,
        Some(signing_key),
        entity,
        entity_id,
        archived,
        payload,
        channel,
        grant_ref,
    )
}

#[allow(clippy::too_many_arguments)]
fn record_version_with_key(
    conn: &Connection,
    node_id: &str,
    signing_key: Option<&SigningKey>,
    entity: &str,
    entity_id: &str,
    archived: bool,
    payload: &str,
    channel: &str,
    grant_ref: &str,
) -> Result<Version, String> {
    if node_id.contains('|') || channel.contains('|') {
        return Err("node_id i channel ne smiju sadrzavati znak `|`".to_owned());
    }

    let supersedes: String = conn
        .query_row(
            "SELECT COALESCE(MAX(version_id), '') FROM record_versions WHERE entity = ?1 AND entity_id = ?2
             AND (?3 = 0 OR signature != '')",
            params![entity, entity_id, signing_key.is_some()],
            |r| r.get(0),
        )
        .map_err(|e| format!("Greska citanja prethodne verzije: {e}"))?;

    // Unsigned historical UUIDs must not control the new authenticated clock.
    let id = next_version_uuid(conn, signing_key.is_some())?;
    let version_id = id.to_string();
    let now_dt = Utc::now();
    let (seconds, nanos) = id.get_timestamp().expect("generated UUIDv7").to_unix();
    let logical_time = chrono::DateTime::<Utc>::from_timestamp(seconds as i64, nanos)
        .ok_or("UUIDv7 vrijeme je izvan raspona")?;
    let created = now_dt.max(logical_time);
    if signing_key.is_some()
        && created.timestamp() > now_dt.timestamp().saturating_add(MAX_CLOCK_SKEW_SECS)
    {
        return Err("Logicki sat je predaleko u buducnosti; provjerite vrijeme uredjaja".into());
    }
    let created_at = created.to_rfc3339();
    let created_at_unix = created.timestamp();

    let (author_pub_key, signature) = if let Some(sk) = signing_key {
        let env = SignedVersionEnvelope {
            version_id: version_id.clone(),
            entity: entity.to_owned(),
            entity_id: entity_id.to_owned(),
            node_id: node_id.to_owned(),
            supersedes: supersedes.clone(),
            archived,
            channel: channel.to_owned(),
            created_at: created_at.clone(),
            created_at_unix,
            schema_version: 1,
            author_pub_key: public_key_to_base64(sk.verifying_key().as_bytes()),
            grant_ref: grant_ref.to_owned(),
            payload: payload.to_owned(),
            signature: String::new(),
        }
        .sign(sk);
        (env.author_pub_key, env.signature)
    } else {
        (String::new(), String::new())
    };

    conn.execute(
        "INSERT INTO record_versions
         (version_id, entity, entity_id, node_id, supersedes, archived, payload, created_at, schema_version, channel, author_pub_key, grant_ref, signature, created_at_unix, applied_at, legacy_imported)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, ?9, ?10, ?11, ?12, ?13, ?14, 0)",
        params![
            version_id,
            entity,
            entity_id,
            node_id,
            supersedes,
            i32::from(archived),
            payload,
            created_at,
            channel,
            author_pub_key,
            grant_ref,
            signature,
            created_at_unix,
            now_dt.to_rfc3339(),
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
        author_pub_key,
        grant_ref: grant_ref.to_owned(),
        signature,
        created_at_unix,
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

/// Returns confirmed contiguous frontier per `(author_node_id, channel)` for `peer_key`,
/// ensuring holes prevent `acked_version_id` from skipping over missing versions.
pub fn get_confirmed_frontier(
    conn: &Connection,
    peer_key: &str,
) -> Result<BTreeMap<String, String>, String> {
    let mut out = get_frontier(conn)?;
    let holes = list_holes(conn)?;
    for h in holes {
        let k = frontier_key(&h.author_node_id, &h.channel);
        if let Some(cur) = out.get(&k)
            && cur.as_str() > h.from_version_id.as_str()
        {
            out.insert(k, h.from_version_id);
        }
    }

    let mut stmt = conn
        .prepare(
            "SELECT author_node_id, channel, acked_version_id
             FROM sync_peer_state
             WHERE peer_key = ?1 AND acked_version_id != ''",
        )
        .map_err(|e| format!("Greska citanja potvrda partnera: {e}"))?;
    let rows = stmt
        .query_map(params![peer_key], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| format!("Greska mapiranja potvrda partnera: {e}"))?;

    for row in rows {
        let (node, ch, acked) = row.map_err(|e| format!("Greska retka potvrde: {e}"))?;
        let k = frontier_key(&node, &ch);
        if let Some(existing) = out.get(&k) {
            if acked < *existing {
                out.insert(k, acked);
            }
        } else {
            out.insert(k, acked);
        }
    }
    Ok(out)
}

pub fn list_holes(conn: &Connection) -> Result<Vec<HoleRecord>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT author_node_id, channel, from_version_id, to_version_id
             FROM sync_holes
             ORDER BY author_node_id, channel, from_version_id",
        )
        .map_err(|e| format!("Greska citanja rupa: {e}"))?;
    let rows = stmt
        .query_map([], |r| {
            Ok(HoleRecord {
                author_node_id: r.get(0)?,
                channel: r.get(1)?,
                from_version_id: r.get(2)?,
                to_version_id: r.get(3)?,
            })
        })
        .map_err(|e| format!("Greska mapiranja rupa: {e}"))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| format!("Greska retka rupe: {e}"))?);
    }
    Ok(out)
}

pub fn record_hole(
    conn: &Connection,
    author_node_id: &str,
    channel: &str,
    from_version_id: &str,
    to_version_id: &str,
) -> Result<(), String> {
    if to_version_id <= from_version_id {
        return Ok(());
    }
    // Merge overlapping/adjacent holes for the same (author_node_id, channel)
    let existing: Option<(i64, String, String)> = conn
        .query_row(
            "SELECT id, from_version_id, to_version_id
             FROM sync_holes
             WHERE author_node_id = ?1 AND channel = ?2
             LIMIT 1",
            params![author_node_id, channel],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(|e| format!("Greska provjere rupe: {e}"))?;

    match existing {
        None => {
            conn.execute(
                "INSERT INTO sync_holes (author_node_id, channel, from_version_id, to_version_id)
                 VALUES (?1, ?2, ?3, ?4)",
                params![author_node_id, channel, from_version_id, to_version_id],
            )
            .map_err(|e| format!("Greska upisa rupe: {e}"))?;
        }
        Some((id, cur_from, cur_to)) => {
            let new_from = if from_version_id < cur_from.as_str() {
                from_version_id.to_owned()
            } else {
                cur_from
            };
            let new_to = if to_version_id > cur_to.as_str() {
                to_version_id.to_owned()
            } else {
                cur_to
            };
            conn.execute(
                "UPDATE sync_holes SET from_version_id = ?1, to_version_id = ?2 WHERE id = ?3",
                params![new_from, new_to, id],
            )
            .map_err(|e| format!("Greska spajanja rupe: {e}"))?;
        }
    }
    Ok(())
}

pub fn narrow_holes(
    conn: &Connection,
    author_node_id: &str,
    channel: &str,
    covered_from: &str,
    covered_to: &str,
) -> Result<(), String> {
    if covered_to <= covered_from {
        return Ok(());
    }
    let existing: Option<(i64, String, String)> = conn
        .query_row(
            "SELECT id, from_version_id, to_version_id
             FROM sync_holes
             WHERE author_node_id = ?1 AND channel = ?2
             LIMIT 1",
            params![author_node_id, channel],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(|e| format!("Greska citanja rupe za suzavanje: {e}"))?;

    let Some((id, hole_from, hole_to)) = existing else {
        return Ok(());
    };

    if covered_to <= hole_from.as_str() || covered_from >= hole_to.as_str() {
        return Ok(());
    }

    if covered_from <= hole_from.as_str() && covered_to >= hole_to.as_str() {
        conn.execute("DELETE FROM sync_holes WHERE id = ?1", params![id])
            .map_err(|e| format!("Greska brisanja zatvorene rupe: {e}"))?;
    } else if covered_from <= hole_from.as_str() {
        conn.execute(
            "UPDATE sync_holes SET from_version_id = ?1 WHERE id = ?2",
            params![covered_to, id],
        )
        .map_err(|e| format!("Greska suzavanja donje granice rupe: {e}"))?;
    } else if covered_to >= hole_to.as_str() {
        conn.execute(
            "UPDATE sync_holes SET to_version_id = ?1 WHERE id = ?2",
            params![covered_from, id],
        )
        .map_err(|e| format!("Greska suzavanja gornje granice rupe: {e}"))?;
    }
    Ok(())
}

pub fn record_rejected_range(
    conn: &Connection,
    peer_key: &str,
    author_node_id: &str,
    channel: &str,
    rejected_id: &str,
) -> Result<(), String> {
    let existing: Option<(String, String)> = conn
        .query_row(
            "SELECT rejected_min_id, rejected_max_id
             FROM sync_peer_state
             WHERE peer_key = ?1 AND author_node_id = ?2 AND channel = ?3",
            params![peer_key, author_node_id, channel],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| format!("Greska citanja odbijenog raspona: {e}"))?;

    match existing {
        None => {
            conn.execute(
                "INSERT INTO sync_peer_state (peer_key, author_node_id, channel, acked_version_id, highest_seen_id, rejected_min_id, rejected_max_id)
                 VALUES (?1, ?2, ?3, '', '', ?4, ?4)",
                params![peer_key, author_node_id, channel, rejected_id],
            )
            .map_err(|e| format!("Greska zapisa odbijenog raspona: {e}"))?;
        }
        Some((cur_min, cur_max)) => {
            let new_min = if cur_min.is_empty() || rejected_id < cur_min.as_str() {
                rejected_id.to_owned()
            } else {
                cur_min
            };
            let new_max = if cur_max.is_empty() || rejected_id > cur_max.as_str() {
                rejected_id.to_owned()
            } else {
                cur_max
            };
            conn.execute(
                "UPDATE sync_peer_state SET rejected_min_id = ?1, rejected_max_id = ?2
                 WHERE peer_key = ?3 AND author_node_id = ?4 AND channel = ?5",
                params![new_min, new_max, peer_key, author_node_id, channel],
            )
            .map_err(|e| format!("Greska azuriranja odbijenog raspona: {e}"))?;
        }
    }
    Ok(())
}

pub fn update_peer_ack(
    conn: &Connection,
    peer_key: &str,
    author_node_id: &str,
    channel: &str,
    acked_version_id: &str,
    highest_seen_id: &str,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO sync_peer_state (peer_key, author_node_id, channel, acked_version_id, highest_seen_id)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(peer_key, author_node_id, channel) DO UPDATE SET
            acked_version_id = CASE WHEN excluded.acked_version_id > sync_peer_state.acked_version_id THEN excluded.acked_version_id ELSE sync_peer_state.acked_version_id END,
            highest_seen_id = CASE WHEN excluded.highest_seen_id > sync_peer_state.highest_seen_id THEN excluded.highest_seen_id ELSE sync_peer_state.highest_seen_id END",
        params![peer_key, author_node_id, channel, acked_version_id, highest_seen_id],
    )
    .map_err(|e| format!("Greska potvrde granice partnera: {e}"))?;
    Ok(())
}

fn load_compaction_floors(conn: &Connection) -> Result<BTreeMap<String, String>, String> {
    let mut stmt = conn
        .prepare("SELECT key, value FROM settings WHERE key LIKE 'compaction_floor:%'")
        .map_err(|e| format!("Greska citanja granice sazimanja: {e}"))?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| format!("Greska mapiranja granice sazimanja: {e}"))?;
    let mut out = BTreeMap::new();
    for r in rows {
        let (k, v) = r.map_err(|e| format!("Greska retka granice sazimanja: {e}"))?;
        if let Some(fkey) = k.strip_prefix("compaction_floor:") {
            out.insert(fkey.to_owned(), v);
        }
    }
    Ok(out)
}

/// Shared decision for ordinary deltas and the automatic recovery dispatcher.
pub(crate) fn recovery_keys_filtered(
    conn: &Connection,
    peer_frontier: &BTreeMap<String, String>,
    requested_holes: &[HoleRecord],
    allowed_channels: &BTreeSet<String>,
    relayable_ids: Option<&BTreeSet<String>>,
) -> Result<Vec<String>, String> {
    let floors = load_compaction_floors(conn)?;
    let relayable_max = if let Some(ids) = relayable_ids {
        let mut map = BTreeMap::<String, String>::new();
        let mut stmt = conn
            .prepare("SELECT version_id, node_id, channel FROM record_versions")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        for row in rows {
            let (vid, node, ch) = row.map_err(|e| e.to_string())?;
            if ids.contains(&vid) {
                let key = frontier_key(&node, &ch);
                map.entry(key)
                    .and_modify(|cur| {
                        if vid > *cur {
                            *cur = vid.clone();
                        }
                    })
                    .or_insert(vid);
            }
        }
        Some(map)
    } else {
        None
    };
    Ok(floors
        .into_iter()
        .filter_map(|(key, floor)| {
            let (author, channel) = split_frontier_key(&key);
            if !allowed_channels.contains(channel) {
                return None;
            }
            if let Some(ref max_map) = relayable_max
                && !max_map.contains_key(&key)
            {
                return None;
            }
            let mut from = peer_frontier.get(&key).map(String::as_str).unwrap_or("");
            for hole in requested_holes {
                if hole.author_node_id == author
                    && hole.channel == channel
                    && hole.to_version_id > hole.from_version_id
                    && hole.from_version_id.as_str() < from
                {
                    from = &hole.from_version_id;
                }
            }
            let effective_floor = relayable_max
                .as_ref()
                .and_then(|m| m.get(&key))
                .map(|max_id| floor.as_str().min(max_id.as_str()))
                .unwrap_or(floor.as_str());
            (from < effective_floor).then_some(key)
        })
        .collect())
}

pub(crate) fn read_version_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Version> {
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
}

/// Computes bounded-byte paged delta across `allowed_channels` (plus any requested holes
/// that the local node can guarantee), and detects when a peer's frontier predates compaction.
pub fn compute_delta_paged(
    conn: &Connection,
    peer_frontier: &BTreeMap<String, String>,
    requested_holes: &[HoleRecord],
    allowed_channels: &BTreeSet<String>,
    max_bytes: usize,
) -> Result<DeltaBatch, String> {
    compute_delta_paged_filtered(
        conn,
        peer_frontier,
        requested_holes,
        allowed_channels,
        max_bytes,
        None,
    )
}

pub(crate) fn compute_delta_paged_filtered(
    conn: &Connection,
    peer_frontier: &BTreeMap<String, String>,
    requested_holes: &[HoleRecord],
    allowed_channels: &BTreeSet<String>,
    max_bytes: usize,
    relayable_ids: Option<&BTreeSet<String>>,
) -> Result<DeltaBatch, String> {
    let budget = delta_byte_budget(max_bytes);
    let mine = if let Some(ids) = relayable_ids {
        let mut map = BTreeMap::<String, String>::new();
        let mut stmt = conn
            .prepare("SELECT version_id, node_id, channel FROM record_versions")
            .map_err(|e| format!("Greska upita granice: {e}"))?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| format!("Greska citanja granice: {e}"))?;
        for row in rows {
            let (vid, node, ch) = row.map_err(|e| format!("Greska retka granice: {e}"))?;
            if ids.contains(&vid) {
                let key = frontier_key(&node, &ch);
                map.entry(key)
                    .and_modify(|cur| {
                        if vid > *cur {
                            *cur = vid.clone();
                        }
                    })
                    .or_insert(vid);
            }
        }
        map
    } else {
        get_frontier(conn)?
    };
    let local_holes = list_holes(conn)?;

    let mut holes_by_key: BTreeMap<String, VersionRange> = BTreeMap::new();
    for h in &local_holes {
        let k = frontier_key(&h.author_node_id, &h.channel);
        if relayable_ids.is_some() && !mine.contains_key(&k) {
            continue;
        }
        holes_by_key.insert(
            k,
            VersionRange {
                from_id: h.from_version_id.clone(),
                to_id: h.to_version_id.clone(),
            },
        );
    }

    // Replay from the earliest requested point, including the interval up to
    // the peer frontier. Combining a narrow old-hole query with a newer delta
    // would otherwise assert coverage for the unsent gap between them.
    let mut replay_frontier = peer_frontier.clone();
    for req in requested_holes {
        if !allowed_channels.contains(&req.channel) || req.to_version_id <= req.from_version_id {
            continue;
        }
        let key = frontier_key(&req.author_node_id, &req.channel);
        replay_frontier
            .entry(key)
            .and_modify(|from| {
                if req.from_version_id < *from {
                    *from = req.from_version_id.clone();
                }
            })
            .or_insert_with(|| req.from_version_id.clone());
    }

    let snapshot_reset_keys = recovery_keys_filtered(
        conn,
        peer_frontier,
        requested_holes,
        allowed_channels,
        relayable_ids,
    )?;
    let mut snapshot_versions = Vec::new();
    let mut reset_channels: BTreeSet<String> = BTreeSet::new();

    for key in &snapshot_reset_keys {
        let (_node, channel) = split_frontier_key(key);
        reset_channels.insert(channel.to_owned());
    }

    if !reset_channels.is_empty() {
        for ch in &reset_channels {
            let mut stmt = conn
                .prepare(
                    "SELECT r.version_id, r.entity, r.entity_id, r.node_id, r.supersedes, r.archived,
                            r.payload, r.created_at, r.schema_version, r.channel, r.author_pub_key,
                            r.grant_ref, r.signature, r.created_at_unix
                     FROM record_versions r
                     WHERE r.channel = ?1
                     ORDER BY r.version_id ASC",
                )
                .map_err(|e| format!("Greska pripreme snapshot reset upita: {e}"))?;
            let rows = stmt
                .query_map(params![ch], read_version_row)
                .map_err(|e| format!("Greska citanja snapshot reset zapisa: {e}"))?;
            let mut latest_by_entity = BTreeMap::<(String, String), Version>::new();
            for row in rows {
                let v = row.map_err(|e| format!("Greska snapshot retka: {e}"))?;
                if relayable_ids.is_some_and(|ids| !ids.contains(&v.version_id)) {
                    continue;
                }
                latest_by_entity.insert((v.entity.clone(), v.entity_id.clone()), v);
            }
            let mut ch_versions: Vec<Version> = latest_by_entity.into_values().collect();
            ch_versions.sort_by(|a, b| a.version_id.cmp(&b.version_id));
            snapshot_versions.extend(ch_versions);
        }
    }

    let mut candidates: Vec<Version> = Vec::new();
    let mut seen_ids: BTreeSet<String> = BTreeSet::new();

    // 2. Forward delta above peer_frontier per allowed (node, channel)
    for (key, my_max) in &mine {
        let (node, channel) = split_frontier_key(key);
        if !allowed_channels.contains(channel) || reset_channels.contains(channel) {
            continue;
        }
        let their_max = replay_frontier.get(key).map(String::as_str).unwrap_or("");
        if their_max >= my_max.as_str() {
            continue;
        }
        let mut stmt = conn
            .prepare(
                "SELECT version_id, entity, entity_id, node_id, supersedes, archived,
                        payload, created_at, schema_version, channel, author_pub_key,
                        grant_ref, signature, created_at_unix
                 FROM record_versions
                 WHERE node_id = ?1 AND channel = ?2 AND version_id > ?3
                 ORDER BY version_id ASC",
            )
            .map_err(|e| format!("Greska pripreme delte: {e}"))?;
        let rows = stmt
            .query_map(params![node, channel, their_max], read_version_row)
            .map_err(|e| format!("Greska citanja delte: {e}"))?;
        for row in rows {
            let v = row.map_err(|e| format!("Greska retka delte: {e}"))?;
            if relayable_ids.is_some_and(|ids| !ids.contains(&v.version_id)) {
                continue;
            }
            if seen_ids.insert(v.version_id.clone()) {
                candidates.push(v);
            }
        }
    }

    // Order control versions first, then ascending by version_id
    candidates.sort_by(|a, b| {
        let a_ctrl = a.channel == "control";
        let b_ctrl = b.channel == "control";
        b_ctrl
            .cmp(&a_ctrl)
            .then_with(|| a.version_id.cmp(&b.version_id))
    });

    let mut batch = DeltaBatch {
        versions: Vec::new(),
        covered_ranges: BTreeMap::new(),
        sender_holes: BTreeMap::new(),
        has_more: false,
        snapshot_reset_keys,
        snapshot_versions,
    };
    let base_bytes = serialized_json_len(&batch)?;
    if base_bytes > budget {
        return Err("Snapshot ili metapodaci ne stanu u granicu stranice; potreban je ograničeni snapshot oporavak".into());
    }
    // Prefix lengths include JSON escaping, UTF-8 and commas. Metadata is
    // measured separately for each probed prefix, including its map keys.
    let mut prefix_bytes = vec![0usize];
    for (i, version) in candidates.iter().enumerate() {
        prefix_bytes.push(prefix_bytes[i] + serialized_json_len(version)? + usize::from(i > 0));
    }
    let metadata = |count: usize| -> (
        BTreeMap<String, VersionRange>,
        BTreeMap<String, VersionRange>,
    ) {
        let mut covered: BTreeMap<String, VersionRange> = BTreeMap::new();
        let mut holes = BTreeMap::new();
        for v in &candidates[..count] {
            let key = frontier_key(&v.node_id, &v.channel);
            let from = replay_frontier.get(&key).cloned().unwrap_or_default();
            if let Some(hole) = holes_by_key.get(&key)
                && hole.to_id > from
                && hole.from_id < v.version_id
            {
                holes.insert(key, hole.clone());
                continue;
            }
            covered
                .entry(key)
                .and_modify(|range| range.to_id = range.to_id.clone().max(v.version_id.clone()))
                .or_insert_with(|| VersionRange {
                    from_id: from,
                    to_id: v.version_id.clone(),
                });
        }
        (covered, holes)
    };
    let size = |count: usize| -> Result<usize, String> {
        let (covered, holes) = metadata(count);
        // Empty maps already occupy four bytes in base_bytes. `true` is one
        // byte shorter than `false` when a continuation is required.
        Ok(base_bytes
            + prefix_bytes[count]
            + serialized_json_len(&covered)?
            + serialized_json_len(&holes)?
            - 4
            - usize::from(count < candidates.len()))
    };
    if !candidates.is_empty() && size(1)? > budget {
        return Err("Prva verzija s metapodacima ne stane u granicu stranice; povećajte granicu (najviše 1 MiB)".into());
    }
    let (mut low, mut high) = (0, candidates.len());
    while low < high {
        let middle = low + (high - low).div_ceil(2);
        if size(middle)? <= budget {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    let (covered, holes) = metadata(low);
    batch.covered_ranges = covered;
    batch.sender_holes = holes;
    batch.has_more = low < candidates.len();
    candidates.truncate(low);
    batch.versions = candidates;
    if serialized_json_len(&batch)? > budget {
        return Err("Serijalizirana stranica prelazi dopuštenu granicu".into());
    }
    Ok(batch)
}

/// Legacy delta helper retained for compatibility with existing callers/tests.
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
                "SELECT version_id, entity, entity_id, node_id, supersedes, archived,
                        payload, created_at, schema_version, channel, author_pub_key,
                        grant_ref, signature, created_at_unix
                 FROM record_versions
                 WHERE node_id = ?1 AND channel = ?2 AND version_id > ?3
                 ORDER BY version_id ASC
                 LIMIT ?4",
            )
            .map_err(|e| format!("Greska pripreme delte: {e}"))?;

        let rows = stmt
            .query_map(
                params![node, channel, their_max, limit as i64],
                read_version_row,
            )
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
    let now = Utc::now().to_rfc3339();
    for v in versions {
        let rows = conn
            .execute(
                "INSERT OR IGNORE INTO record_versions
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
                    now,
                ],
            )
            .map_err(|e| format!("Greska unosa udaljene verzije: {e}"))?;
        inserted += rows;
    }
    Ok(inserted)
}

/// Prunes redundant intermediate versions and records recovery floors atomically.
/// Preserves current snapshots, tombstones, control proofs, first signed identity
/// bindings, each author's latest record, pending projections and streams with holes.
/// The caller runs VACUUM separately, after this transaction has committed.
pub fn auto_compact(conn: &Connection, policy: CompactionPolicy) -> Result<usize, String> {
    if !conn.is_autocommit() {
        return Err("Sazimanje nije dopusteno unutar poslovne transakcije".into());
    }
    if !(1..=36500).contains(&policy.retention_days) || policy.max_versions_per_record == 0 {
        return Err("Neispravna politika cuvanja verzija".into());
    }
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let conn = &tx;
    let cutoff = (Utc::now() - Duration::days(policy.retention_days)).to_rfc3339();
    let keep_n = policy.max_versions_per_record.max(1) as i64;

    let deleted_floors: Vec<(String, String, String)> = {
        let mut stmt = conn
            .prepare(
                "
                SELECT node_id, channel, MAX(version_id)
                FROM record_versions
                WHERE version_id NOT IN (
                    SELECT MAX(version_id) FROM record_versions GROUP BY entity, entity_id
                )
                  AND channel != 'control' AND archived = 0
                  AND version_id NOT IN (SELECT MIN(version_id) FROM record_versions WHERE signature != '' AND legacy_imported=0 GROUP BY entity,entity_id)
                  AND version_id NOT IN (SELECT MAX(version_id) FROM record_versions GROUP BY entity,entity_id,node_id,channel)
                  AND version_id NOT IN (SELECT version_id FROM pending_projections)
                  AND NOT EXISTS (SELECT 1 FROM sync_holes h WHERE h.author_node_id=record_versions.node_id AND h.channel=record_versions.channel)
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
                GROUP BY node_id, channel
                ",
            )
            .map_err(|e| format!("Greska pripreme granice sazimanja: {e}"))?;
        let rows = stmt
            .query_map(params![cutoff, keep_n], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| format!("Greska citanja granice sazimanja: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Greska retka granice sazimanja: {e}"))?
    };

    let deleted = conn
        .execute(
            "
            DELETE FROM record_versions
            WHERE version_id NOT IN (
                SELECT MAX(version_id) FROM record_versions GROUP BY entity, entity_id
            )
              AND channel != 'control' AND archived = 0
              AND version_id NOT IN (SELECT MIN(version_id) FROM record_versions WHERE signature != '' AND legacy_imported=0 GROUP BY entity,entity_id)
              AND version_id NOT IN (SELECT MAX(version_id) FROM record_versions GROUP BY entity,entity_id,node_id,channel)
              AND version_id NOT IN (SELECT version_id FROM pending_projections)
              AND NOT EXISTS (SELECT 1 FROM sync_holes h WHERE h.author_node_id=record_versions.node_id AND h.channel=record_versions.channel)
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

    for (node, ch, max_deleted_vid) in deleted_floors {
        let fkey = format!("compaction_floor:{}", frontier_key(&node, &ch));
        conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = CASE WHEN excluded.value > settings.value THEN excluded.value ELSE settings.value END",
            params![fkey, max_deleted_vid],
        )
        .map_err(|e| format!("Greska zapisa granice sazimanja: {e}"))?;
    }

    tx.commit()
        .map_err(|e| format!("Greska potvrde sazimanja: {e}"))?;

    Ok(deleted)
}

/// Reclaims unused SQLite pages (`VACUUM` + `PRAGMA optimize`) so the physical `.sqlite` file shrinks on disk.
pub fn vacuum_db(conn: &Connection) -> Result<(), String> {
    if !conn.is_autocommit() {
        return Err("VACUUM nije dopusten unutar aktivne poslovne transakcije".to_owned());
    }
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
    use crate::migrations::run_migrations;

    #[test]
    fn auto_compact_preserves_latest_tombstones_and_frontier() {
        let conn = Connection::open_in_memory().expect("mem db");
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS stores (id INTEGER PRIMARY KEY, sync_id TEXT UNIQUE, name TEXT UNIQUE, location TEXT DEFAULT '', color TEXT DEFAULT '', updated_at TEXT);
             CREATE TABLE IF NOT EXISTS products (barcode TEXT PRIMARY KEY, sync_id TEXT UNIQUE, name TEXT, category TEXT DEFAULT '', default_unit TEXT DEFAULT 'kom', last_price_cents INTEGER DEFAULT 0, last_store_name TEXT DEFAULT '', image_data_url TEXT, updated_at TEXT);
             CREATE TABLE IF NOT EXISTS price_history (id INTEGER PRIMARY KEY, barcode TEXT, store_name TEXT, price_cents INTEGER, recorded_at TEXT);
             CREATE TABLE IF NOT EXISTS receipts (id INTEGER PRIMARY KEY, sync_id TEXT UNIQUE, store_id INTEGER, store_name TEXT, created_at TEXT, cart_total_cents INTEGER, receipt_total_cents INTEGER, difference_cents INTEGER);
             CREATE TABLE IF NOT EXISTS receipt_items (id INTEGER PRIMARY KEY, receipt_id INTEGER, barcode TEXT, name TEXT, quantity REAL, unit TEXT, unit_price_cents INTEGER, total_cents INTEGER);
             CREATE TABLE IF NOT EXISTS shopping_lists (id INTEGER PRIMARY KEY, sync_id TEXT UNIQUE, name TEXT, store_id INTEGER, updated_at TEXT);
             CREATE TABLE IF NOT EXISTS planned_items (id INTEGER PRIMARY KEY, sync_id TEXT UNIQUE, list_id INTEGER, barcode TEXT, name TEXT, quantity REAL, unit TEXT, estimated_price_cents INTEGER, checked INTEGER, updated_at TEXT);
             CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )
        .unwrap();
        init_ledger_schema(&conn).expect("schema");
        run_migrations(&conn).expect("migrations");

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

        record_version(&conn, "node-a", "products", "p-2", false, "{}", "").expect("p2");
        let v_p2_tomb =
            record_version(&conn, "node-a", "products", "p-2", true, "{}", "").expect("p2 tomb");

        let frontier_before = get_frontier(&conn).expect("frontier before");

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

        vacuum_db(&conn).expect("vacuum after committed compaction");

        let freelist: i64 = conn
            .query_row("PRAGMA freelist_count", [], |r| r.get(0))
            .expect("freelist");
        assert_eq!(freelist, 0);
    }
}
