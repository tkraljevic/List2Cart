use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const CURRENT_SCHEMA_VERSION: i64 = 7;

// Fixed UUID namespace for deterministic v1 -> v2 backfill of legacy rows without sync_id.
const MIGRATION_NS: Uuid = Uuid::from_u128(0x6c32632d_6d69_4772_8174_696f6e5f7632);

pub fn run_migrations(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL
        );
        ",
    )
    .map_err(|e| format!("Greska tablice migracija: {e}"))?;

    let current_ver: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 1) FROM schema_migrations",
            [],
            |r| r.get(0),
        )
        .map_err(|e| format!("Greska citanja verzije sheme: {e}"))?;

    if current_ver > CURRENT_SCHEMA_VERSION {
        return Err(format!(
            "Baza koristi noviju shemu {current_ver}; aplikacija podrzava {CURRENT_SCHEMA_VERSION}"
        ));
    }
    if current_ver < 2 {
        migrate_v1_to_v2(conn)?;
    }
    if current_ver < 3 {
        migrate_v2_to_v3(conn)?;
    }
    if current_ver < 4 {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        tx.execute_batch("
            CREATE TABLE sync_snapshot_sessions (
                direction TEXT NOT NULL CHECK(direction IN ('in','out')),
                snapshot_id TEXT NOT NULL, peer_key TEXT NOT NULL, manifest TEXT NOT NULL,
                next_offset INTEGER NOT NULL DEFAULT 0, complete INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY(direction,snapshot_id,peer_key));
            CREATE TABLE sync_snapshot_items (
                direction TEXT NOT NULL, snapshot_id TEXT NOT NULL, peer_key TEXT NOT NULL,
                ordinal INTEGER NOT NULL, version_json TEXT NOT NULL,
                PRIMARY KEY(direction,snapshot_id,peer_key,ordinal),
                FOREIGN KEY(direction,snapshot_id,peer_key) REFERENCES sync_snapshot_sessions(direction,snapshot_id,peer_key) ON DELETE CASCADE);
            INSERT INTO schema_migrations(version,applied_at) VALUES (4,datetime('now'));
        ").map_err(|e| format!("Greska migracije snapshot oporavka: {e}"))?;
        tx.commit().map_err(|e| e.to_string())?;
    }

    if current_ver < 5 {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        tx.execute_batch("
            CREATE TABLE sync_backfill_revisions(channel TEXT PRIMARY KEY,revision TEXT NOT NULL);
            CREATE TABLE sync_backfill_ack(peer_key TEXT NOT NULL,channel TEXT NOT NULL,revision TEXT NOT NULL,PRIMARY KEY(peer_key,channel));
            INSERT INTO schema_migrations(version,applied_at) VALUES(5,datetime('now'));
        ").map_err(|e|format!("Greska migracije naknadnih dopuna: {e}"))?;
        // An upgraded database may already contain previously unannounced late
        // arrivals. Force one replay of its signed channels, once at migration.
        let channels: Vec<String> = {
            let mut stmt=tx.prepare("SELECT DISTINCT channel FROM record_versions WHERE signature!='' AND legacy_imported=0").map_err(|e|e.to_string())?;
            stmt.query_map([], |r| r.get(0))
                .map_err(|e| e.to_string())?
                .collect::<Result<_, _>>()
                .map_err(|e| e.to_string())?
        };
        for channel in channels {
            tx.execute(
                "INSERT INTO sync_backfill_revisions(channel,revision) VALUES(?1,?2)",
                params![channel, Uuid::new_v4().to_string()],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
    }
    if current_ver < 6 {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        tx.execute_batch("UPDATE sync_trust_records SET permission=CASE WHEN json_valid(raw_json) THEN CASE json_extract(raw_json,'$.data.kind') WHEN 'list_grant' THEN 'list_grant' WHEN 'ovlast' THEN 'ovlast' WHEN 'membership' THEN 'membership' WHEN 'clanstvo' THEN 'membership' ELSE 'legacy_key' END ELSE 'legacy_key' END WHERE kind='revocation';").map_err(|e|e.to_string())?;
        tx.execute_batch("UPDATE sync_trust_records AS r SET permission='legacy_key' WHERE r.kind='revocation' AND r.permission='list_grant' AND EXISTS(SELECT 1 FROM sync_trust_records t WHERE t.record_id=r.target_id AND (t.kind!='list_grant' OR t.subject_key!=r.subject_key));").map_err(|e|e.to_string())?;
        crate::acl::apply_revocation_flags(&tx, true)?;
        tx.execute(
            "INSERT INTO schema_migrations(version,applied_at) VALUES(6,datetime('now'))",
            [],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
    }
    if current_ver < 7 {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        add_column_if_missing(&tx, "products", "image_hash", "TEXT NOT NULL DEFAULT ''")?;
        tx.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS content_blobs (
                hash TEXT PRIMARY KEY,
                mime TEXT NOT NULL,
                size_bytes INTEGER NOT NULL,
                data BLOB,
                source TEXT NOT NULL DEFAULT 'local',
                created_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS content_refs (
                hash TEXT NOT NULL,
                entity TEXT NOT NULL,
                entity_id TEXT NOT NULL,
                channel TEXT NOT NULL DEFAULT '',
                PRIMARY KEY (hash, entity, entity_id)
            );
            CREATE INDEX IF NOT EXISTS idx_content_refs_entity ON content_refs(entity, entity_id);
            CREATE TABLE IF NOT EXISTS content_wants (
                hash TEXT PRIMARY KEY,
                mime TEXT NOT NULL,
                expected_bytes INTEGER NOT NULL DEFAULT 0,
                channel TEXT NOT NULL DEFAULT '',
                requested_at TEXT NOT NULL DEFAULT ''
            );
            INSERT INTO schema_migrations(version, applied_at) VALUES (7, datetime('now'));
            ",
        )
        .map_err(|e| format!("Greska migracije spremista sadrzaja: {e}"))?;
        tx.commit().map_err(|e| e.to_string())?;
    }
    Ok(())
}

// Natural keys are lookup hints; stable IDs own identity. Rebuild without renaming
// the old parent tables so existing receipt/list foreign keys keep their targets.
fn migrate_v2_to_v3(conn: &Connection) -> Result<(), String> {
    if !conn.is_autocommit() {
        return Err("Migracija v3 zahtijeva zasebnu transakciju".into());
    }
    let foreign_keys: bool = conn
        .query_row("PRAGMA foreign_keys", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    conn.execute_batch("PRAGMA foreign_keys=OFF;")
        .map_err(|e| e.to_string())?;
    let result = (|| {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        tx.execute_batch("
            CREATE TABLE stores_v3 (
                id INTEGER PRIMARY KEY AUTOINCREMENT, sync_id TEXT NOT NULL UNIQUE,
                name TEXT NOT NULL, location TEXT NOT NULL DEFAULT '',
                color TEXT NOT NULL DEFAULT '#1768D1', updated_at TEXT NOT NULL,
                archived INTEGER NOT NULL DEFAULT 0);
            INSERT INTO stores_v3 SELECT id,sync_id,name,location,color,updated_at,archived FROM stores;
            CREATE TABLE products_v3 (
                barcode TEXT NOT NULL, sync_id TEXT NOT NULL PRIMARY KEY,
                name TEXT NOT NULL, category TEXT NOT NULL DEFAULT 'Opce',
                default_unit TEXT NOT NULL DEFAULT 'kom', last_price_cents INTEGER NOT NULL DEFAULT 0,
                last_store_name TEXT NOT NULL DEFAULT '', image_data_url TEXT,
                updated_at TEXT NOT NULL, archived INTEGER NOT NULL DEFAULT 0);
            INSERT INTO products_v3 SELECT barcode,sync_id,name,category,default_unit,last_price_cents,last_store_name,image_data_url,updated_at,archived FROM products;
            CREATE TABLE price_history_v3 (
                id INTEGER PRIMARY KEY AUTOINCREMENT, barcode TEXT NOT NULL,
                store_name TEXT NOT NULL, price_cents INTEGER NOT NULL, recorded_at TEXT NOT NULL,
                sync_id TEXT UNIQUE, store_sync_id TEXT NOT NULL DEFAULT '',
                product_sync_id TEXT NOT NULL DEFAULT '', updated_at TEXT NOT NULL DEFAULT '',
                archived INTEGER NOT NULL DEFAULT 0);
            INSERT INTO price_history_v3 SELECT id,barcode,store_name,price_cents,recorded_at,sync_id,store_sync_id,product_sync_id,updated_at,archived FROM price_history;
            DROP TABLE price_history;
            DROP TABLE products;
            DROP TABLE stores;
            ALTER TABLE stores_v3 RENAME TO stores;
            ALTER TABLE products_v3 RENAME TO products;
            ALTER TABLE price_history_v3 RENAME TO price_history;
            CREATE INDEX idx_stores_name ON stores(name COLLATE NOCASE);
            CREATE INDEX idx_products_barcode ON products(barcode);
            CREATE INDEX idx_price_history_product ON price_history(product_sync_id);
            INSERT INTO schema_migrations(version,applied_at) VALUES (3,datetime('now'));
        ").map_err(|e| format!("Greska migracije stabilnih identiteta: {e}"))?;
        let broken: bool = tx
            .prepare("PRAGMA foreign_key_check")
            .map_err(|e| e.to_string())?
            .exists([])
            .map_err(|e| e.to_string())?;
        if broken {
            return Err("Migracija v3 narusava veze podataka".into());
        }
        tx.commit().map_err(|e| e.to_string())
    })();
    let restore = conn
        .execute_batch(if foreign_keys {
            "PRAGMA foreign_keys=ON;"
        } else {
            "PRAGMA foreign_keys=OFF;"
        })
        .map_err(|e| e.to_string());
    result.and(restore)
}

fn has_column(conn: &Connection, table: &str, column: &str) -> Result<bool, String> {
    let sql = format!("PRAGMA table_info({table})");
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("Greska provjere stupaca za {table}: {e}"))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| format!("Greska citanja stupaca za {table}: {e}"))?;
    for col in rows {
        if col.map_err(|e| format!("Greska stupca {table}: {e}"))? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    definition: &str,
) -> Result<(), String> {
    if !has_column(conn, table, column)? {
        let sql = format!("ALTER TABLE {table} ADD COLUMN {column} {definition}");
        conn.execute(&sql, [])
            .map_err(|e| format!("Greska migracije {table}.{column}: {e}"))?;
    }
    Ok(())
}

fn migrate_v1_to_v2(conn: &Connection) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("Greska pokretanja migracije v2: {e}"))?;

    add_column_if_missing(&tx, "stores", "archived", "INTEGER NOT NULL DEFAULT 0")?;
    add_column_if_missing(&tx, "products", "archived", "INTEGER NOT NULL DEFAULT 0")?;
    add_column_if_missing(&tx, "receipts", "archived", "INTEGER NOT NULL DEFAULT 0")?;
    add_column_if_missing(&tx, "receipts", "store_sync_id", "TEXT NOT NULL DEFAULT ''")?;
    add_column_if_missing(
        &tx,
        "shopping_lists",
        "archived",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(&tx, "shopping_lists", "store_sync_id", "TEXT")?;

    add_column_if_missing(&tx, "price_history", "sync_id", "TEXT")?;
    add_column_if_missing(
        &tx,
        "price_history",
        "store_sync_id",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    add_column_if_missing(
        &tx,
        "price_history",
        "product_sync_id",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    add_column_if_missing(
        &tx,
        "price_history",
        "updated_at",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    add_column_if_missing(
        &tx,
        "price_history",
        "archived",
        "INTEGER NOT NULL DEFAULT 0",
    )?;

    add_column_if_missing(&tx, "receipt_items", "sync_id", "TEXT")?;
    add_column_if_missing(
        &tx,
        "receipt_items",
        "receipt_sync_id",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    add_column_if_missing(
        &tx,
        "receipt_items",
        "updated_at",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    add_column_if_missing(
        &tx,
        "receipt_items",
        "archived",
        "INTEGER NOT NULL DEFAULT 0",
    )?;

    add_column_if_missing(
        &tx,
        "planned_items",
        "list_sync_id",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    add_column_if_missing(
        &tx,
        "planned_items",
        "archived",
        "INTEGER NOT NULL DEFAULT 0",
    )?;

    add_column_if_missing(
        &tx,
        "record_versions",
        "author_pub_key",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    add_column_if_missing(
        &tx,
        "record_versions",
        "grant_ref",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    add_column_if_missing(
        &tx,
        "record_versions",
        "signature",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    add_column_if_missing(
        &tx,
        "record_versions",
        "created_at_unix",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    add_column_if_missing(
        &tx,
        "record_versions",
        "applied_at",
        "TEXT NOT NULL DEFAULT ''",
    )?;
    add_column_if_missing(
        &tx,
        "record_versions",
        "legacy_imported",
        "INTEGER NOT NULL DEFAULT 0",
    )?;

    // Backfill store_sync_id on receipts & shopping_lists
    tx.execute(
        "UPDATE receipts
         SET store_sync_id = COALESCE((SELECT s.sync_id FROM stores s WHERE s.id = receipts.store_id), '')
         WHERE store_sync_id = ''",
        [],
    )
    .map_err(|e| format!("Greska backfilla store_sync_id na racunima: {e}"))?;

    tx.execute(
        "UPDATE shopping_lists
         SET store_sync_id = (SELECT s.sync_id FROM stores s WHERE s.id = shopping_lists.store_id)
         WHERE store_id IS NOT NULL AND store_sync_id IS NULL",
        [],
    )
    .map_err(|e| format!("Greska backfilla store_sync_id na popisima: {e}"))?;

    tx.execute(
        "UPDATE planned_items
         SET list_sync_id = COALESCE((SELECT sl.sync_id FROM shopping_lists sl WHERE sl.id = planned_items.list_id), '')
         WHERE list_sync_id = ''",
        [],
    )
    .map_err(|e| format!("Greska backfilla list_sync_id na stavkama: {e}"))?;

    // Deterministically backfill price_history sync_id, store_sync_id, product_sync_id, updated_at
    {
        let mut stmt = tx
            .prepare(
                "SELECT ph.id, ph.barcode, ph.store_name, ph.price_cents, ph.recorded_at,
                        COALESCE((SELECT s.sync_id FROM stores s WHERE LOWER(s.name) = LOWER(ph.store_name) LIMIT 1), ''),
                        COALESCE((SELECT p.sync_id FROM products p WHERE p.barcode = ph.barcode LIMIT 1), '')
                 FROM price_history ph
                 WHERE ph.sync_id IS NULL OR ph.sync_id = ''",
            )
            .map_err(|e| format!("Greska citanja price_history za migraciju: {e}"))?;

        let rows: Vec<(i64, String, String, i64, String, String, String)> = stmt
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            })
            .map_err(|e| format!("Greska mapiranja price_history: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Greska retka price_history: {e}"))?;

        for (id, barcode, store_name, price_cents, recorded_at, store_sync_id, product_sync_id) in
            rows
        {
            let seed =
                format!("price_history|{id}|{barcode}|{store_name}|{price_cents}|{recorded_at}");
            let sync_id = Uuid::new_v5(&MIGRATION_NS, seed.as_bytes()).to_string();
            tx.execute(
                "UPDATE price_history
                 SET sync_id = ?1, store_sync_id = ?2, product_sync_id = ?3,
                     updated_at = CASE WHEN updated_at = '' THEN ?4 ELSE updated_at END
                 WHERE id = ?5",
                params![sync_id, store_sync_id, product_sync_id, recorded_at, id],
            )
            .map_err(|e| format!("Greska azuriranja price_history: {e}"))?;
        }
    }

    // Deterministically backfill receipt_items sync_id, receipt_sync_id, updated_at
    {
        let mut stmt = tx
            .prepare(
                "SELECT ri.id, COALESCE(r.sync_id, ''), ri.barcode, ri.name, ri.unit_price_cents, ri.total_cents, COALESCE(r.created_at, '')
                 FROM receipt_items ri
                 LEFT JOIN receipts r ON ri.receipt_id = r.id
                 WHERE ri.sync_id IS NULL OR ri.sync_id = ''",
            )
            .map_err(|e| format!("Greska citanja receipt_items za migraciju: {e}"))?;

        let rows: Vec<(i64, String, String, String, i64, i64, String)> = stmt
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            })
            .map_err(|e| format!("Greska mapiranja receipt_items: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Greska retka receipt_items: {e}"))?;

        for (id, receipt_sync_id, barcode, name, unit_price, total, created_at) in rows {
            let seed = format!(
                "receipt_items|{id}|{receipt_sync_id}|{barcode}|{name}|{unit_price}|{total}"
            );
            let sync_id = Uuid::new_v5(&MIGRATION_NS, seed.as_bytes()).to_string();
            tx.execute(
                "UPDATE receipt_items
                 SET sync_id = ?1, receipt_sync_id = ?2,
                     updated_at = CASE WHEN updated_at = '' THEN ?3 ELSE updated_at END
                 WHERE id = ?4",
                params![sync_id, receipt_sync_id, created_at, id],
            )
            .map_err(|e| format!("Greska azuriranja receipt_items: {e}"))?;
        }
    }

    // Backfill created_at_unix and legacy_imported on existing unsigned record_versions
    {
        let mut stmt = tx
            .prepare(
                "SELECT version_id, created_at, signature
                 FROM record_versions
                 WHERE created_at_unix = 0",
            )
            .map_err(|e| format!("Greska citanja record_versions za migraciju: {e}"))?;
        let rows: Vec<(String, String, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(|e| format!("Greska mapiranja record_versions: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Greska retka record_versions: {e}"))?;

        for (vid, created_at, signature) in rows {
            let unix = DateTime::parse_from_rfc3339(&created_at)
                .map(|dt| dt.timestamp())
                .unwrap_or(0);
            let legacy = i32::from(signature.is_empty());
            tx.execute(
                "UPDATE record_versions SET created_at_unix = ?1, legacy_imported = ?2 WHERE version_id = ?3",
                params![unix, legacy, vid],
            )
            .map_err(|e| format!("Greska azuriranja record_versions: {e}"))?;
        }
    }

    tx.execute_batch(
        "
        CREATE UNIQUE INDEX IF NOT EXISTS idx_price_history_sync_id
            ON price_history(sync_id);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_receipt_items_sync_id
            ON receipt_items(sync_id);
        CREATE INDEX IF NOT EXISTS idx_planned_items_list_sync
            ON planned_items(list_sync_id);
        CREATE INDEX IF NOT EXISTS idx_receipt_items_receipt_sync
            ON receipt_items(receipt_sync_id);

        CREATE TABLE IF NOT EXISTS sync_trust_records (
            record_id TEXT PRIMARY KEY,
            kind TEXT NOT NULL,
            subject_key TEXT NOT NULL,
            target_id TEXT NOT NULL,
            permission TEXT NOT NULL,
            issued_at INTEGER NOT NULL,
            expires_at INTEGER NOT NULL,
            revoked INTEGER NOT NULL DEFAULT 0,
            raw_json TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_trust_subject
            ON sync_trust_records(subject_key, kind, target_id);

        CREATE TABLE IF NOT EXISTS sync_peer_state (
            peer_key TEXT NOT NULL,
            author_node_id TEXT NOT NULL,
            channel TEXT NOT NULL,
            acked_version_id TEXT NOT NULL DEFAULT '',
            highest_seen_id TEXT NOT NULL DEFAULT '',
            rejected_min_id TEXT NOT NULL DEFAULT '',
            rejected_max_id TEXT NOT NULL DEFAULT '',
            PRIMARY KEY (peer_key, author_node_id, channel)
        );

        CREATE TABLE IF NOT EXISTS sync_holes (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            author_node_id TEXT NOT NULL,
            channel TEXT NOT NULL,
            from_version_id TEXT NOT NULL,
            to_version_id TEXT NOT NULL,
            UNIQUE(author_node_id, channel, from_version_id, to_version_id)
        );
        CREATE INDEX IF NOT EXISTS idx_sync_holes_author_channel
            ON sync_holes(author_node_id, channel);

        CREATE TABLE IF NOT EXISTS pending_projections (
            version_id TEXT PRIMARY KEY,
            entity TEXT NOT NULL,
            entity_id TEXT NOT NULL,
            parent_entity TEXT NOT NULL,
            parent_sync_id TEXT NOT NULL,
            payload TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_pending_parent
            ON pending_projections(parent_entity, parent_sync_id);
        ",
    )
    .map_err(|e| format!("Greska kreiranja indeksa i sync tablica: {e}"))?;

    let now = Utc::now().to_rfc3339();
    tx.execute(
        "INSERT OR REPLACE INTO schema_migrations (version, applied_at) VALUES (2, ?1)",
        params![now],
    )
    .map_err(|e| format!("Greska zapisa verzije migracije: {e}"))?;

    tx.commit()
        .map_err(|e| format!("Greska potvrde migracije v2: {e}"))?;

    Ok(())
}

pub fn key_fingerprint_hex(pub_key_b64: &str) -> String {
    let mut h = Sha256::new();
    h.update(pub_key_b64.trim().as_bytes());
    hex::encode(h.finalize())
}

/// Prevents snapshot/restore cloning: if the database was already bound to an external
/// Ed25519 key fingerprint, starting without that exact key is rejected instead of silently
/// generating a replacement key for an existing member.
pub fn bind_or_verify_device_key_fingerprint(
    conn: &Connection,
    external_pub_key_b64: Option<&str>,
) -> Result<(), String> {
    if let Some(key) = external_pub_key_b64 {
        syncnet::identity::parse_public_key(key)
            .map_err(|e| format!("Neispravan vanjski kljuc: {e}"))?;
    }
    let stored: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'device_key_fingerprint'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| format!("Greska citanja otiska kljuca uredjaja: {e}"))?;

    match (stored, external_pub_key_b64) {
        (Some(_), None) => Err(
            "Nedostaje vanjski kljuc uredjaja za postojecu bazu — moguca kopija/snapshot. Potreban je novi onboarding umjesto tihog stvaranja zamjenskog kljuca."
                .to_owned(),
        ),
        (Some(expected_fp), Some(actual_b64)) => {
            let actual_fp = key_fingerprint_hex(actual_b64);
            if actual_fp != expected_fp {
                return Err(
                    "Otkrivena nepodudarnost vanjskog kljuca i baze podataka (klon ili zamijenjen kljuc uredjaja)."
                        .to_owned(),
                );
            }
            Ok(())
        }
        (None, Some(actual_b64)) => {
            let fp = key_fingerprint_hex(actual_b64);
            conn.execute(
                "INSERT INTO settings (key, value) VALUES ('device_key_fingerprint', ?1)",
                params![fp],
            )
            .map_err(|e| format!("Greska pohrane otiska kljuca: {e}"))?;
            Ok(())
        }
        (None, None) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::init_ledger_schema;
    use ed25519_dalek::SigningKey;
    use syncnet::identity::verifying_key_to_base64;

    fn legacy_database() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
              "CREATE TABLE stores (id INTEGER PRIMARY KEY, sync_id TEXT, name TEXT, location TEXT NOT NULL DEFAULT '', color TEXT NOT NULL DEFAULT '#000000', updated_at TEXT NOT NULL DEFAULT '');
               CREATE TABLE products (barcode TEXT PRIMARY KEY, sync_id TEXT, name TEXT NOT NULL DEFAULT 'Item', category TEXT NOT NULL DEFAULT 'Opce', default_unit TEXT NOT NULL DEFAULT 'kom', last_price_cents INTEGER NOT NULL DEFAULT 0, last_store_name TEXT NOT NULL DEFAULT '', image_data_url TEXT, updated_at TEXT NOT NULL DEFAULT '');
             CREATE TABLE price_history (id INTEGER PRIMARY KEY, barcode TEXT, store_name TEXT, price_cents INTEGER, recorded_at TEXT);
             CREATE TABLE receipts (id INTEGER PRIMARY KEY, sync_id TEXT, store_id INTEGER, created_at TEXT);
             CREATE TABLE receipt_items (id INTEGER PRIMARY KEY, receipt_id INTEGER, barcode TEXT, name TEXT, unit_price_cents INTEGER, total_cents INTEGER);
             CREATE TABLE shopping_lists (id INTEGER PRIMARY KEY, sync_id TEXT, store_id INTEGER);
             CREATE TABLE planned_items (id INTEGER PRIMARY KEY, list_id INTEGER);
             CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
               INSERT INTO stores(id,sync_id,name) VALUES (1, 'store-1', 'Trgovina');
               INSERT INTO products(barcode,sync_id) VALUES ('38501', 'product-1');
             INSERT INTO receipts VALUES (1, 'receipt-1', 1, '2026-11-17T00:00:00Z');
             INSERT INTO receipt_items VALUES (1, 1, '38501', 'Kruh', 100, 100);
             INSERT INTO price_history VALUES (1, '38501', 'Trgovina', 100, '2026-11-17T00:00:00Z');
             INSERT INTO shopping_lists VALUES (1, 'list-1', 1);
             INSERT INTO planned_items VALUES (1, 1);"
        ).unwrap();
        init_ledger_schema(&conn).unwrap();
        conn
    }

    #[test]
    fn backfill_upgrade_announces_existing_signed_inventory_only_once() {
        let db = crate::db::Database::open_in_memory().unwrap();
        let conn = db.test_connection();
        conn.execute_batch("DROP TABLE sync_backfill_ack; DROP TABLE sync_backfill_revisions; DELETE FROM schema_migrations WHERE version>=5;").unwrap();
        let channels:usize=conn.query_row("SELECT COUNT(DISTINCT channel) FROM record_versions WHERE signature!='' AND legacy_imported=0",[],|r|r.get::<_,u32>(0)).unwrap() as usize;
        assert!(channels > 0);
        run_migrations(conn).unwrap();
        let revisions = || {
            let mut stmt = conn
                .prepare("SELECT channel,revision FROM sync_backfill_revisions ORDER BY channel")
                .unwrap();
            stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
        };
        let first = revisions();
        assert_eq!(first.len(), channels);
        run_migrations(conn).unwrap();
        assert_eq!(revisions(), first);
    }

    #[test]
    fn backfill_migration_rolls_back_both_tables_and_retries() {
        let conn = legacy_database();
        run_migrations(&conn).unwrap();
        conn.execute_batch("DROP TABLE sync_backfill_ack; DROP TABLE sync_backfill_revisions; DELETE FROM schema_migrations WHERE version>=5; CREATE TABLE sync_backfill_ack(conflict TEXT);").unwrap();
        assert!(run_migrations(&conn).is_err());
        let created: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='sync_backfill_revisions')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!created);
        let version: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(version, 4);
        conn.execute_batch("DROP TABLE sync_backfill_ack").unwrap();
        run_migrations(&conn).unwrap();
        run_migrations(&conn).unwrap();
        let name: String = conn
            .query_row("SELECT name FROM stores WHERE id=1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(name, "Trgovina");
        let version: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(version, CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn recovery_migration_is_atomic_and_repeatable() {
        let conn = legacy_database();
        conn.execute_batch(
            "CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,applied_at TEXT NOT NULL);",
        )
        .unwrap();
        migrate_v1_to_v2(&conn).unwrap();
        migrate_v2_to_v3(&conn).unwrap();
        conn.execute_batch("CREATE TABLE sync_snapshot_items(conflict TEXT);")
            .unwrap();
        assert!(run_migrations(&conn).is_err());
        let has_sessions: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='sync_snapshot_sessions')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!has_sessions);
        let version: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(version, 3);
        conn.execute_batch("DROP TABLE sync_snapshot_items")
            .unwrap();
        run_migrations(&conn).unwrap();
        run_migrations(&conn).unwrap();
        let version: i64 = conn
            .query_row("SELECT MAX(version) FROM schema_migrations", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(version, CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn legacy_backfill_preserves_relations_and_is_repeatable() {
        let conn = legacy_database();
        run_migrations(&conn).unwrap();
        let ids = || {
            conn.query_row(
            "SELECT ri.sync_id, ri.receipt_sync_id, ph.sync_id, ph.product_sync_id, pi.list_sync_id, r.store_sync_id
             FROM receipt_items ri, price_history ph, planned_items pi, receipts r",
            [], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?, r.get::<_, String>(4)?, r.get::<_, String>(5)?))
        ).unwrap()
        };
        let before = ids();
        assert!(!before.0.is_empty());
        assert_eq!(
            (&before.1, &before.3, &before.4, &before.5),
            (
                &"receipt-1".to_owned(),
                &"product-1".to_owned(),
                &"list-1".to_owned(),
                &"store-1".to_owned()
            )
        );
        run_migrations(&conn).unwrap();
        assert_eq!(ids(), before);
    }

    #[test]
    fn failed_migration_rolls_back_columns_and_keeps_source_data() {
        let conn = legacy_database();
        conn.execute_batch("ALTER TABLE products RENAME COLUMN sync_id TO broken")
            .unwrap();
        assert!(run_migrations(&conn).is_err());
        assert!(!has_column(&conn, "stores", "archived").unwrap());
        assert_eq!(
            conn.query_row("SELECT name FROM stores WHERE id = 1", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "Trgovina"
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM schema_migrations", [], |r| r
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[test]
    fn future_schema_and_cloned_or_malformed_identity_are_rejected() {
        let conn = legacy_database();
        run_migrations(&conn).unwrap();
        conn.execute("INSERT INTO schema_migrations VALUES (99, 'future')", [])
            .unwrap();
        assert!(run_migrations(&conn).is_err());
        assert!(bind_or_verify_device_key_fingerprint(&conn, Some("not-a-key")).is_err());
        let a = verifying_key_to_base64(&SigningKey::from_bytes(&[1; 32]).verifying_key());
        let b = verifying_key_to_base64(&SigningKey::from_bytes(&[2; 32]).verifying_key());
        bind_or_verify_device_key_fingerprint(&conn, Some(&a)).unwrap();
        assert!(bind_or_verify_device_key_fingerprint(&conn, None).is_err());
        assert!(bind_or_verify_device_key_fingerprint(&conn, Some(&b)).is_err());
        bind_or_verify_device_key_fingerprint(&conn, Some(&a)).unwrap();
    }

    #[test]
    fn identity_migration_keeps_foreign_keys_and_rolls_back_broken_relations() {
        for broken in [false, true] {
            let conn = legacy_database();
            conn.execute_batch("CREATE TABLE schema_migrations(version INTEGER PRIMARY KEY,applied_at TEXT NOT NULL);").unwrap();
            migrate_v1_to_v2(&conn).unwrap();
            conn.execute_batch("PRAGMA foreign_keys=OFF;").unwrap();
            conn.execute_batch(
                "CREATE TABLE related_store(store_id INTEGER REFERENCES stores(id));",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO related_store VALUES (?1)",
                [if broken { 999 } else { 1 }],
            )
            .unwrap();
            conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
            let result = migrate_v2_to_v3(&conn);
            assert_eq!(result.is_err(), broken);
            assert!(
                conn.query_row("PRAGMA foreign_keys", [], |r| r.get::<_, bool>(0))
                    .unwrap()
            );
            let version: i64 = conn
                .query_row("SELECT MAX(version) FROM schema_migrations", [], |r| {
                    r.get(0)
                })
                .unwrap();
            assert_eq!(version, if broken { 2 } else { 3 });
            let name: String = conn
                .query_row("SELECT name FROM stores WHERE id=1", [], |r| r.get(0))
                .unwrap();
            assert_eq!(name, "Trgovina");
            if !broken {
                assert!(
                    conn.execute("INSERT INTO related_store VALUES (999)", [])
                        .is_err()
                );
                conn.execute("INSERT INTO stores(sync_id,name,updated_at) VALUES ('store-2','Trgovina','now')", []).unwrap();
                conn.execute("INSERT INTO products(sync_id,barcode,name,updated_at) VALUES ('product-2','38501','Other','now')", []).unwrap();
                run_migrations(&conn).unwrap();
            }
        }
    }
}
