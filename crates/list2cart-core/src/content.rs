use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use base64::{Engine as _, engine::general_purpose::STANDARD as B64_STD};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use syncnet::wire::conversation::ContentBlobItem;

/// Maximum size of a single stored image/content object (1.5 MiB, leaving room for base64/JSON
/// inside the 2 MiB `sadrzaj` conversation frame).
pub const MAX_CONTENT_BYTES: usize = 1_500_000;

/// Maximum aggregate decoded bytes returned in a single `ContentMessage` round.
pub const MAX_CONTENT_RESPONSE_BYTES: usize = 1_400_000;

/// Maximum number of content hashes requested in a single `WantsMessage`.
pub const MAX_WANTS_PER_ROUND: usize = 32;

/// Minimum age before an orphaned `.part.` staging file is deleted by GC.
const STALE_PART_AGE: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContentReceiveReport {
    pub stored: usize,
    pub rejected: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContentGcReport {
    pub removed_blobs: usize,
    pub reclaimed_bytes: usize,
    pub removed_part_files: usize,
}

/// Computes lowercase hex SHA-256 digest of `bytes`.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Validates whether a hex string is a well-formed 64-char lowercase SHA-256 digest.
#[must_use]
pub fn is_valid_sha256_hex(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Returns true only for safe raster image MIME types (SVG and HTML are intentionally excluded).
#[must_use]
pub fn is_supported_image_mime(mime: &str) -> bool {
    matches!(
        mime,
        "image/png" | "image/jpeg" | "image/webp" | "image/gif"
    )
}

/// Validates MIME type, size bounds, and magic-byte file header of an image payload.
pub fn verify_image_bytes(mime: &str, bytes: &[u8]) -> Result<(), String> {
    if !is_supported_image_mime(mime) {
        return Err(format!("Nepodrzani format slike: {mime}"));
    }
    if bytes.is_empty() {
        return Err("Prazan sadrzaj slike".to_owned());
    }
    if bytes.len() > MAX_CONTENT_BYTES {
        return Err(format!(
            "Slika premasuje dopustenu velicinu ({} > {MAX_CONTENT_BYTES} B)",
            bytes.len()
        ));
    }

    let valid_magic = match mime {
        "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => {
            bytes.len() >= 3 && bytes[0] == 0xff && bytes[1] == 0xd8 && bytes[2] == 0xff
        }
        "image/gif" => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
        "image/webp" => bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP",
        _ => false,
    };

    if !valid_magic {
        return Err(format!("Bajtovi sadrzaja ne odgovaraju formatu {mime}"));
    }
    Ok(())
}

/// Parses and validates a `data:image/...;base64,...` URI.
pub fn parse_image_data_url(data_url: &str) -> Result<(String, Vec<u8>), String> {
    let rest = data_url
        .trim()
        .strip_prefix("data:")
        .ok_or_else(|| "Neispravan data URL prefiks".to_owned())?;
    let (meta, payload) = rest
        .split_once(',')
        .ok_or_else(|| "Neispravan data URL format".to_owned())?;
    let mime = meta
        .strip_suffix(";base64")
        .ok_or_else(|| "Slika mora biti base64 kodirana".to_owned())?
        .trim()
        .to_ascii_lowercase();
    if !is_supported_image_mime(&mime) {
        return Err(format!("Nepodrzani MIME slike: {mime}"));
    }
    // Reject absurdly large base64 strings before allocating the decoded buffer.
    if payload.len() > MAX_CONTENT_BYTES.saturating_mul(2) {
        return Err("Kodirana slika premasuje dopustenu velicinu".to_owned());
    }
    let bytes = B64_STD
        .decode(payload.trim())
        .map_err(|e| format!("Neispravan base64 sadrzaj slike: {e}"))?;
    verify_image_bytes(&mime, &bytes)?;
    Ok((mime, bytes))
}

/// Formats verified image bytes back into a `data:<mime>;base64,...` string for UI display.
#[must_use]
pub fn format_image_data_url(mime: &str, bytes: &[u8]) -> String {
    format!("data:{mime};base64,{}", B64_STD.encode(bytes))
}

/// Resolves the `blobs/` directory alongside a file-backed SQLite database.
#[must_use]
pub fn blobs_dir_for_db(db_path: Option<&Path>) -> Option<PathBuf> {
    db_path.and_then(|p| p.parent()).map(|d| d.join("blobs"))
}

fn write_blob_atomic(dir: &Path, hash: &str, bytes: &[u8]) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("Greska kreiranja mape sadrzaja: {e}"))?;
    let final_path = dir.join(hash);
    if final_path.exists()
        && let Ok(existing) = fs::read(&final_path)
        && sha256_hex(&existing) == hash
    {
        return Ok(());
    }

    let part_path = dir.join(format!("{hash}.part.{}", uuid::Uuid::new_v4()));
    {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&part_path)
            .map_err(|e| format!("Greska otvaranja privremene datoteke sadrzaja: {e}"))?;
        if let Err(err) = file.write_all(bytes).and_then(|()| file.sync_all()) {
            let _ = fs::remove_file(&part_path);
            return Err(format!("Greska upisa sadrzaja na disk: {err}"));
        }
    }
    if let Err(err) = fs::rename(&part_path, &final_path) {
        let _ = fs::remove_file(&part_path);
        return Err(format!("Greska potvrde datoteke sadrzaja: {err}"));
    }
    Ok(())
}

/// Validates, verifies hash (if expected), persists atomically (file-backed `blobs_dir` when
/// available or inline SQLite BLOB for `:memory:`), and optionally records a `content_refs` link.
pub fn store_verified_blob(
    conn: &Connection,
    blobs_dir: Option<&Path>,
    expected_hash: Option<&str>,
    mime: &str,
    bytes: &[u8],
    source: &str,
    ref_info: Option<(&str, &str, &str)>,
) -> Result<String, String> {
    verify_image_bytes(mime, bytes)?;
    let actual_hash = sha256_hex(bytes);
    if let Some(expected) = expected_hash
        && actual_hash != expected
    {
        return Err(format!(
            "Otisak sadrzaja se ne slaze (ocekivano {expected}, dobiveno {actual_hash})"
        ));
    }

    let inline_data: Option<&[u8]> = match blobs_dir {
        Some(dir) => {
            write_blob_atomic(dir, &actual_hash, bytes)?;
            None
        }
        None => Some(bytes),
    };

    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO content_blobs (hash, mime, size_bytes, data, source, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(hash) DO NOTHING",
        params![
            &actual_hash,
            mime,
            bytes.len() as i64,
            inline_data,
            source,
            &now
        ],
    )
    .map_err(|e| format!("Greska pohrane metapodataka sadrzaja: {e}"))?;

    conn.execute("DELETE FROM content_wants WHERE hash = ?1", [&actual_hash])
        .map_err(|e| format!("Greska uklanjanja zelje sadrzaja: {e}"))?;

    if let Some((entity, entity_id, channel)) = ref_info {
        conn.execute(
            "INSERT INTO content_refs (hash, entity, entity_id, channel)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(hash, entity, entity_id) DO UPDATE SET channel = excluded.channel",
            params![&actual_hash, entity, entity_id, channel],
        )
        .map_err(|e| format!("Greska povezivanja sadrzaja: {e}"))?;
    }

    Ok(actual_hash)
}

/// Reads and integrity-checks a stored blob by its SHA-256 hash.
pub fn load_verified_blob(
    conn: &Connection,
    blobs_dir: Option<&Path>,
    hash: &str,
) -> Result<Option<(String, Vec<u8>)>, String> {
    if !is_valid_sha256_hex(hash) {
        return Ok(None);
    }
    let row: Option<(String, Option<Vec<u8>>)> = conn
        .query_row(
            "SELECT mime, data FROM content_blobs WHERE hash = ?1",
            [hash],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| format!("Greska citanja sadrzaja: {e}"))?;

    let Some((mime, inline_data)) = row else {
        return Ok(None);
    };

    let bytes = match inline_data {
        Some(b) => b,
        None => {
            let Some(dir) = blobs_dir else {
                return Ok(None);
            };
            let path = dir.join(hash);
            if !path.exists() {
                return Ok(None);
            }
            fs::read(&path).map_err(|e| format!("Greska citanja datoteke sadrzaja: {e}"))?
        }
    };

    if sha256_hex(&bytes) != hash || verify_image_bytes(&mime, &bytes).is_err() {
        return Err(format!("Sadrzaj {hash} je ostecen ili neispravan"));
    }
    Ok(Some((mime, bytes)))
}

/// Records a reference from `(entity, entity_id)` to `hash` and queues a want if the blob is
/// not yet stored locally.
pub fn record_content_ref_and_want(
    conn: &Connection,
    hash: &str,
    mime: &str,
    expected_bytes: usize,
    entity: &str,
    entity_id: &str,
    channel: &str,
) -> Result<(), String> {
    if !is_valid_sha256_hex(hash) || !is_supported_image_mime(mime) {
        return Err("Neispravan otisak ili MIME tip sadrzaja".to_owned());
    }
    if expected_bytes > MAX_CONTENT_BYTES {
        return Err("Najavljena velicina sadrzaja premasuje dopusteni limit".to_owned());
    }

    conn.execute(
        "INSERT INTO content_refs (hash, entity, entity_id, channel)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(hash, entity, entity_id) DO UPDATE SET channel = excluded.channel",
        params![hash, entity, entity_id, channel],
    )
    .map_err(|e| format!("Greska upisa veze sadrzaja: {e}"))?;

    let exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM content_blobs WHERE hash = ?1)",
            [hash],
            |r| r.get(0),
        )
        .map_err(|e| format!("Greska provjere sadrzaja: {e}"))?;

    if !exists {
        conn.execute(
            "INSERT INTO content_wants (hash, mime, expected_bytes, channel, requested_at)
             VALUES (?1, ?2, ?3, ?4, '')
             ON CONFLICT(hash) DO NOTHING",
            params![hash, mime, expected_bytes as i64, channel],
        )
        .map_err(|e| format!("Greska upisa zelje sadrzaja: {e}"))?;
    }
    Ok(())
}

/// Migrates inline `products.image_data_url` entries (that lack `image_hash`) into the content
/// store and clears the inline column once persisted.
pub fn migrate_inline_product_images(
    conn: &Connection,
    blobs_dir: Option<&Path>,
) -> Result<usize, String> {
    let mut stmt = conn
        .prepare(
            "SELECT sync_id, image_data_url FROM products
             WHERE image_data_url IS NOT NULL AND image_data_url != '' AND image_hash = ''",
        )
        .map_err(|e| format!("Greska citanja slika artikala: {e}"))?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| format!("Greska upita slika artikala: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Greska reda slika artikala: {e}"))?;

    let mut migrated = 0usize;
    for (sync_id, data_url) in rows {
        let Ok((mime, bytes)) = parse_image_data_url(&data_url) else {
            continue;
        };
        let hash = store_verified_blob(
            conn,
            blobs_dir,
            None,
            &mime,
            &bytes,
            "local",
            Some(("products", &sync_id, "catalog")),
        )?;
        conn.execute(
            "UPDATE products SET image_hash = ?1, image_data_url = NULL WHERE sync_id = ?2",
            params![hash, sync_id],
        )
        .map_err(|e| format!("Greska azuriranja otiska slike: {e}"))?;
        migrated += 1;
    }
    Ok(migrated)
}

/// Returns up to `limit` pending content hashes that are not yet present in `content_blobs`.
pub fn pending_wants(conn: &Connection, limit: usize) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT w.hash FROM content_wants w
             WHERE NOT EXISTS (SELECT 1 FROM content_blobs b WHERE b.hash = w.hash)
             ORDER BY w.requested_at ASC, w.hash ASC
             LIMIT ?1",
        )
        .map_err(|e| format!("Greska citanja zelja sadrzaja: {e}"))?;
    let rows = stmt
        .query_map([limit as i64], |r| r.get::<_, String>(0))
        .map_err(|e| format!("Greska mapiranja zelja sadrzaja: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Greska reda zelja sadrzaja: {e}"))?;
    Ok(rows)
}

/// Selects authorized blobs requested by `peer_key` up to `MAX_CONTENT_RESPONSE_BYTES`.
/// Unauthorized hashes (where `peer_key` has no active access to any referenced channel) and
/// missing hashes are silently omitted.
pub fn fulfill_wants_for_peer(
    conn: &Connection,
    blobs_dir: Option<&Path>,
    network_pub: &ed25519_dalek::VerifyingKey,
    peer_key: &[u8; 32],
    requested_hashes: &[String],
    now_unix: i64,
) -> Result<Vec<ContentBlobItem>, String> {
    if requested_hashes.is_empty() {
        return Ok(Vec::new());
    }
    let peer_b64 = syncnet::identity::verifying_key_to_base64(
        &ed25519_dalek::VerifyingKey::from_bytes(peer_key)
            .map_err(|e| format!("Neispravan kljuc partnera: {e}"))?,
    );
    let peer_channels =
        crate::apply::allowed_channels_for_recipient(conn, network_pub, &peer_b64, now_unix)?;
    if peer_channels.is_empty() {
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let mut total_bytes = 0usize;

    for hash in requested_hashes.iter().take(MAX_WANTS_PER_ROUND) {
        if !is_valid_sha256_hex(hash) || !seen.insert(hash.clone()) {
            continue;
        }
        let mut ref_stmt = conn
            .prepare("SELECT channel FROM content_refs WHERE hash = ?1")
            .map_err(|e| format!("Greska provjere kanala sadrzaja: {e}"))?;
        let channels = ref_stmt
            .query_map([hash], |r| r.get::<_, String>(0))
            .map_err(|e| format!("Greska upita kanala sadrzaja: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Greska reda kanala sadrzaja: {e}"))?;

        let authorized = channels.iter().any(|ch| peer_channels.contains(ch));
        if !authorized {
            continue;
        }

        let Ok(Some((mime, bytes))) = load_verified_blob(conn, blobs_dir, hash) else {
            continue;
        };
        if total_bytes + bytes.len() > MAX_CONTENT_RESPONSE_BYTES && !out.is_empty() {
            break;
        }
        total_bytes += bytes.len();
        out.push(ContentBlobItem {
            otisak: hash.clone(),
            mime,
            data_b64: B64_STD.encode(&bytes),
        });
    }

    Ok(out)
}

/// Validates and stores received `ContentBlobItem` entries that match local pending wants.
/// Unwanted, oversized, corrupt, or hash-mismatched items are rejected without mutating state.
pub fn receive_wanted_content(
    conn: &Connection,
    blobs_dir: Option<&Path>,
    items: &[ContentBlobItem],
) -> Result<ContentReceiveReport, String> {
    let mut report = ContentReceiveReport::default();

    for item in items {
        if !is_valid_sha256_hex(&item.otisak) {
            report.rejected += 1;
            continue;
        }
        let is_wanted: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM content_wants WHERE hash = ?1)",
                [&item.otisak],
                |r| r.get(0),
            )
            .map_err(|e| format!("Greska provjere zelje: {e}"))?;
        if !is_wanted {
            report.rejected += 1;
            continue;
        }
        if item.data_b64.len() > MAX_CONTENT_BYTES.saturating_mul(2) {
            report.rejected += 1;
            continue;
        }
        let Ok(bytes) = B64_STD.decode(item.data_b64.trim()) else {
            report.rejected += 1;
            continue;
        };
        if store_verified_blob(
            conn,
            blobs_dir,
            Some(&item.otisak),
            &item.mime,
            &bytes,
            "peer",
            None,
        )
        .is_ok()
        {
            report.stored += 1;
        } else {
            report.rejected += 1;
        }
    }

    Ok(report)
}

/// Garbage-collects unreferenced content blobs and stale `.part.` files outside the network
/// receive loop, preserving blobs still referenced by active records or retained version history.
pub fn gc_unreferenced_content(
    conn: &Connection,
    blobs_dir: Option<&Path>,
) -> Result<ContentGcReport, String> {
    let mut report = ContentGcReport::default();

    // Prune content_refs whose product no longer points to that hash (and is not kept in active history).
    let mut stmt = conn
        .prepare("SELECT hash, size_bytes FROM content_blobs")
        .map_err(|e| format!("Greska pregleda sadrzaja: {e}"))?;
    let blobs = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
        .map_err(|e| format!("Greska upita sadrzaja: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Greska reda sadrzaja: {e}"))?;

    for (hash, size_bytes) in blobs {
        let referenced_by_product: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM products WHERE image_hash = ?1)",
                [&hash],
                |r| r.get(0),
            )
            .map_err(|e| format!("Greska provjere artikla: {e}"))?;
        let referenced_by_history: bool = if referenced_by_product {
            true
        } else {
            let pattern = format!("%{hash}%");
            conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM record_versions WHERE entity = 'products' AND payload LIKE ?1)",
                [pattern],
                |r| r.get(0),
            )
            .map_err(|e| format!("Greska provjere povijesti sadrzaja: {e}"))?
        };

        if referenced_by_history {
            continue;
        }

        conn.execute("DELETE FROM content_refs WHERE hash = ?1", [&hash])
            .map_err(|e| format!("Greska brisanja veza sadrzaja: {e}"))?;
        conn.execute("DELETE FROM content_blobs WHERE hash = ?1", [&hash])
            .map_err(|e| format!("Greska brisanja metapodataka sadrzaja: {e}"))?;
        if let Some(dir) = blobs_dir {
            let _ = fs::remove_file(dir.join(&hash));
        }
        report.removed_blobs += 1;
        report.reclaimed_bytes += size_bytes.max(0) as usize;
    }

    if let Some(dir) = blobs_dir
        && dir.is_dir()
        && let Ok(entries) = fs::read_dir(dir)
    {
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if !name.contains(".part.") {
                continue;
            }
            let is_stale = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|m| SystemTime::now().duration_since(m).ok())
                .is_some_and(|age| age >= STALE_PART_AGE);
            if is_stale && fs::remove_file(&path).is_ok() {
                report.removed_part_files += 1;
            }
        }
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apply::PROTOCOL_NAME;
    use crate::db::{Database, UpsertProductInput};
    use crate::ledger::PeerRole;
    use crate::models::Unit;
    use crate::pairing_session::PairingController;
    use crate::session::{SessionConfig, run_conversation_until_idle};
    use std::net::{Ipv4Addr, SocketAddr};
    use std::sync::{Arc, Mutex};

    const TINY_PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=";

    fn tiny_png_data_url() -> String {
        format!("data:image/png;base64,{TINY_PNG_B64}")
    }

    fn temp_dir(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("l2c-content-{label}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    async fn pair_two_dbs(db_a: Arc<Mutex<Database>>, db_b: Arc<Mutex<Database>>, role: PeerRole) {
        let ctrl_a = PairingController::default();
        let ctrl_b = PairingController::default();
        let (sk_a, id_a) = {
            let db = db_a.lock().unwrap();
            (
                db.clone_device_signing_key(),
                PairingController::build_pair_identity(
                    &db.sync_identity().device_public_key,
                    "Uredaj A",
                    PROTOCOL_NAME,
                ),
            )
        };
        let (sk_b, id_b) = {
            let db = db_b.lock().unwrap();
            (
                db.clone_device_signing_key(),
                PairingController::build_pair_identity(
                    &db.sync_identity().device_public_key,
                    "Uredaj B",
                    PROTOCOL_NAME,
                ),
            )
        };

        let listen = ctrl_b
            .start_listening(
                SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
                sk_b,
                id_b,
                false,
            )
            .await
            .unwrap();
        let port = listen.listen_port.unwrap();
        ctrl_a
            .dial(
                SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
                sk_a,
                id_a,
                true,
            )
            .await
            .unwrap();

        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        while !ctrl_b.status(false).pending {
            assert!(tokio::time::Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        let (ra, rb) = tokio::join!(
            ctrl_a.confirm(&db_a, true, role, "*", true),
            ctrl_b.confirm(&db_b, true, role, "*", false),
        );
        assert!(ra.unwrap().paired);
        assert!(rb.unwrap().paired);
    }

    #[test]
    fn content_store_rejects_corrupt_oversized_and_svg_formats() {
        let svg = "data:image/svg+xml;base64,PHN2Zz48L3N2Zz4=";
        assert!(parse_image_data_url(svg).is_err());

        let fake_png = format!("data:image/png;base64,{}", B64_STD.encode(b"not-a-png"));
        assert!(parse_image_data_url(&fake_png).is_err());

        let mut oversized = b"\x89PNG\r\n\x1a\n".to_vec();
        oversized.resize(MAX_CONTENT_BYTES + 1, 0);
        assert!(verify_image_bytes("image/png", &oversized).is_err());

        let dir = temp_dir("corrupt");
        let db_path = dir.join("list2cart.sqlite");
        let db = Database::open(&db_path).unwrap();
        let (_mime, bytes) = parse_image_data_url(&tiny_png_data_url()).unwrap();
        let hash = store_verified_blob(
            db.test_connection(),
            db.blobs_dir().as_deref(),
            None,
            "image/png",
            &bytes,
            "local",
            Some(("products", "p1", "catalog")),
        )
        .unwrap();

        // Corrupt the stored file on disk and verify load rejects it.
        let blob_path = db.blobs_dir().unwrap().join(&hash);
        fs::write(&blob_path, b"corrupted").unwrap();
        assert!(
            load_verified_blob(db.test_connection(), db.blobs_dir().as_deref(), &hash).is_err()
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn partial_staging_file_is_never_treated_as_complete_and_gc_preserves_referenced() {
        let dir = temp_dir("atomic-gc");
        let db_path = dir.join("list2cart.sqlite");
        let db = Database::open(&db_path).unwrap();
        let blobs_dir = db.blobs_dir().unwrap();
        fs::create_dir_all(&blobs_dir).unwrap();

        let fake_hash = "a".repeat(64);
        fs::write(blobs_dir.join(format!("{fake_hash}.part.123")), b"partial").unwrap();
        assert_eq!(
            load_verified_blob(db.test_connection(), Some(&blobs_dir), &fake_hash).unwrap(),
            None
        );

        let prod = db
            .upsert_product(&UpsertProductInput {
                barcode: "385111222",
                name: "Maslinovo ulje",
                category: "Hrana",
                default_unit: Unit::L,
                price_cents: 899,
                store_name: "Konzum",
                image_data_url: Some(&tiny_png_data_url()),
            })
            .unwrap();
        assert!(prod.image_data_url.is_some());

        let gc1 = db.gc_unreferenced_content().unwrap();
        assert_eq!(gc1.removed_blobs, 0);

        // Insert an unreferenced orphan blob and verify GC removes only the orphan.
        let orphan_bytes = b"\x89PNG\r\n\x1a\nORPHAN";
        let orphan_hash = store_verified_blob(
            db.test_connection(),
            Some(&blobs_dir),
            None,
            "image/png",
            orphan_bytes,
            "peer",
            None,
        )
        .unwrap();
        assert!(blobs_dir.join(&orphan_hash).exists());

        let gc2 = db.gc_unreferenced_content().unwrap();
        assert_eq!(gc2.removed_blobs, 1);
        assert!(!blobs_dir.join(&orphan_hash).exists());
        assert!(
            db.get_product("385111222")
                .unwrap()
                .unwrap()
                .image_data_url
                .is_some()
        );

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn migrates_existing_inline_product_data_urls_on_open() {
        let dir = temp_dir("migrate");
        let db_path = dir.join("list2cart.sqlite");
        {
            let db = Database::open(&db_path).unwrap();
            let conn = db.test_connection();
            let data_url = tiny_png_data_url();
            conn.execute(
                "INSERT INTO products (barcode, sync_id, name, category, default_unit, last_price_cents, last_store_name, image_data_url, image_hash, updated_at, archived)
                 VALUES ('385999000', 'legacy-prod-1', 'Sir', 'Mlijecno', 'kom', 250, 'Lidl', ?1, '', '2026-11-18T10:00:00Z', 0)",
                [data_url],
            )
            .unwrap();
        }

        let reopened = Database::open(&db_path).unwrap();
        let prod = reopened.get_product("385999000").unwrap().unwrap();
        assert_eq!(
            prod.image_data_url.as_deref(),
            Some(tiny_png_data_url().as_str())
        );

        let inline_left: Option<String> = reopened
            .test_connection()
            .query_row(
                "SELECT image_data_url FROM products WHERE sync_id = 'legacy-prod-1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(inline_left.is_none());

        let _ = fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn paired_peers_exchange_product_photos_without_resending_or_leaking() {
        let dir_a = temp_dir("sync-a");
        let dir_b = temp_dir("sync-b");
        let db_a = Arc::new(Mutex::new(
            Database::open(&dir_a.join("list2cart.sqlite")).unwrap(),
        ));
        let db_b = Arc::new(Mutex::new(
            Database::open(&dir_b.join("list2cart.sqlite")).unwrap(),
        ));

        pair_two_dbs(Arc::clone(&db_a), Arc::clone(&db_b), PeerRole::OwnDevice).await;

        db_a.lock()
            .unwrap()
            .upsert_product(&UpsertProductInput {
                barcode: "385777888",
                name: "Kava 250g",
                category: "Napitci",
                default_unit: Unit::Kom,
                price_cents: 450,
                store_name: "Plodine",
                image_data_url: Some(&tiny_png_data_url()),
            })
            .unwrap();

        let pk_a = *db_a
            .lock()
            .unwrap()
            .clone_device_signing_key()
            .verifying_key()
            .as_bytes();
        let pk_b = *db_b
            .lock()
            .unwrap()
            .clone_device_signing_key()
            .verifying_key()
            .as_bytes();
        let (mut sa, mut sb) = tokio::io::duplex(64 * 1024);
        let cfg = SessionConfig::default();

        let (out_a, out_b) = tokio::join!(
            run_conversation_until_idle(&mut sa, &db_a, &pk_b, None, true, &cfg),
            run_conversation_until_idle(&mut sb, &db_b, &pk_a, None, false, &cfg),
        );
        out_a.unwrap();
        out_b.unwrap();

        {
            let guard_b = db_b.lock().unwrap();
            let prod_b = guard_b.get_product("385777888").unwrap().unwrap();
            assert_eq!(
                prod_b.image_data_url.as_deref(),
                Some(tiny_png_data_url().as_str())
            );
            assert!(guard_b.pending_content_wants(10).unwrap().is_empty());
        }

        // Verify unauthorized peer (FamilyMember without private channel access) cannot fetch A's photo.
        let dir_c = temp_dir("sync-c");
        let db_c = Arc::new(Mutex::new(
            Database::open(&dir_c.join("list2cart.sqlite")).unwrap(),
        ));
        pair_two_dbs(Arc::clone(&db_a), Arc::clone(&db_c), PeerRole::FamilyMember).await;
        let pk_c = *db_c
            .lock()
            .unwrap()
            .clone_device_signing_key()
            .verifying_key()
            .as_bytes();

        let (_mime, bytes) = parse_image_data_url(&tiny_png_data_url()).unwrap();
        let hash = sha256_hex(&bytes);
        let leaked = db_a
            .lock()
            .unwrap()
            .fulfill_content_wants(&pk_c, std::slice::from_ref(&hash))
            .unwrap();
        assert!(
            leaked.is_empty(),
            "Unauthorized peer must not receive private image blob"
        );

        // Verify corrupt ContentBlobItem is rejected on receive.
        db_c.lock()
            .unwrap()
            .test_connection()
            .execute(
                "INSERT INTO content_wants (hash, mime, expected_bytes, channel, requested_at) VALUES (?1, 'image/png', 68, 'catalog', '')",
                [&hash],
            )
            .unwrap();
        let corrupt_rep = db_c
            .lock()
            .unwrap()
            .receive_wanted_content(&[ContentBlobItem {
                otisak: hash.clone(),
                mime: "image/png".to_owned(),
                data_b64: B64_STD.encode(b"\x89PNG\r\n\x1a\nTAMPERED"),
            }])
            .unwrap();
        assert_eq!(corrupt_rep.stored, 0);
        assert_eq!(corrupt_rep.rejected, 1);

        let _ = fs::remove_dir_all(&dir_a);
        let _ = fs::remove_dir_all(&dir_b);
        let _ = fs::remove_dir_all(&dir_c);
    }
}
