mod local;
pub(crate) use local::local_snapshot_for_pairing;
pub use local::{IdentityConflict, LocalEntity, SyncIdentity};

use crate::calc::calculate_line_total_cents;
use crate::ledger::{
    CompactionPolicy, LedgerStats, NetworkMember, PeerRole, add_network_member, auto_compact,
    compute_delta, get_frontier, get_stats, init_ledger_schema, list_network_members,
    remove_network_member, shared_lists_allows, vacuum_db, verify_peer_identity,
};
use crate::models::{
    CartItem, PlannedItem, Product, ReceiptSummary, ShoppingList, Store, StorePrice, SyncBundle,
    Unit,
};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use uuid::Uuid;

pub struct UpsertProductInput<'a> {
    pub barcode: &'a str,
    pub name: &'a str,
    pub category: &'a str,
    pub default_unit: Unit,
    pub price_cents: i64,
    pub store_name: &'a str,
    pub image_data_url: Option<&'a str>,
}

pub struct Database {
    conn: Connection,
    identity: local::LocalIdentity,
}

fn resolve_legacy_store(
    conn: &Connection,
    bundle: &SyncBundle,
    source_id: Option<i64>,
    name: Option<&str>,
) -> Result<Option<i64>, String> {
    if let Some(store) = source_id.and_then(|id| bundle.stores.iter().find(|s| s.id == id)) {
        return conn
            .query_row(
                "SELECT id FROM stores WHERE sync_id=?1",
                [&store.sync_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string());
    }
    let Some(name) = name else {
        return Ok(None);
    };
    let mut stmt = conn
        .prepare("SELECT id FROM stores WHERE LOWER(name)=LOWER(?1) ORDER BY sync_id")
        .map_err(|e| e.to_string())?;
    let ids = stmt
        .query_map([name.trim()], |r| r.get::<_, i64>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    if ids.len() > 1 {
        return Err("Legacy paket nema jednoznacnu referencu trgovine".into());
    }
    Ok(ids.first().copied())
}

impl Database {
    #[cfg(test)]
    pub(crate) fn test_connection(&self) -> &Connection {
        &self.conn
    }

    pub fn open(path: &Path) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| format!("Greska baze podataka: {e}"))?;
        let identity = local::LocalIdentity::load(&conn, path)?;
        let db = Self { conn, identity };
        db.init_schema()?;
        db.initialize_local_identity()?;
        crate::content::migrate_inline_product_images(&db.conn, db.blobs_dir().as_deref())?;
        Ok(db)
    }

    pub fn open_in_memory() -> Result<Self, String> {
        let conn =
            Connection::open_in_memory().map_err(|e| format!("Greska memorije baze: {e}"))?;
        let db = Self {
            conn,
            identity: local::LocalIdentity::ephemeral(),
        };
        db.init_schema()?;
        db.initialize_local_identity()?;
        crate::content::migrate_inline_product_images(&db.conn, db.blobs_dir().as_deref())?;
        Ok(db)
    }

    fn init_schema(&self) -> Result<(), String> {
        self.conn
            .execute_batch(
                "
                PRAGMA foreign_keys = ON;

                CREATE TABLE IF NOT EXISTS stores (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    sync_id TEXT NOT NULL UNIQUE,
                    name TEXT NOT NULL UNIQUE,
                    location TEXT NOT NULL DEFAULT '',
                    color TEXT NOT NULL DEFAULT '#1768D1',
                    updated_at TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS products (
                    barcode TEXT PRIMARY KEY,
                    sync_id TEXT NOT NULL UNIQUE,
                    name TEXT NOT NULL,
                    category TEXT NOT NULL DEFAULT 'Opce',
                    default_unit TEXT NOT NULL DEFAULT 'kom',
                    last_price_cents INTEGER NOT NULL DEFAULT 0,
                    last_store_name TEXT NOT NULL DEFAULT '',
                    image_data_url TEXT,
                    updated_at TEXT NOT NULL
                );

                CREATE TABLE IF NOT EXISTS price_history (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    barcode TEXT NOT NULL,
                    store_name TEXT NOT NULL,
                    price_cents INTEGER NOT NULL,
                    recorded_at TEXT NOT NULL,
                    FOREIGN KEY (barcode) REFERENCES products(barcode) ON DELETE CASCADE
                );

                CREATE TABLE IF NOT EXISTS receipts (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    sync_id TEXT NOT NULL UNIQUE,
                    store_id INTEGER NOT NULL,
                    store_name TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    cart_total_cents INTEGER NOT NULL,
                    receipt_total_cents INTEGER NOT NULL,
                    difference_cents INTEGER NOT NULL,
                    FOREIGN KEY (store_id) REFERENCES stores(id)
                );

                CREATE TABLE IF NOT EXISTS receipt_items (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    receipt_id INTEGER NOT NULL,
                    barcode TEXT NOT NULL,
                    name TEXT NOT NULL,
                    quantity REAL NOT NULL,
                    unit TEXT NOT NULL,
                    unit_price_cents INTEGER NOT NULL,
                    total_cents INTEGER NOT NULL,
                    FOREIGN KEY (receipt_id) REFERENCES receipts(id) ON DELETE CASCADE
                );

                CREATE TABLE IF NOT EXISTS shopping_lists (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    sync_id TEXT NOT NULL UNIQUE,
                    name TEXT NOT NULL,
                    store_id INTEGER,
                    updated_at TEXT NOT NULL,
                    FOREIGN KEY (store_id) REFERENCES stores(id) ON DELETE SET NULL
                );

                CREATE TABLE IF NOT EXISTS planned_items (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    sync_id TEXT NOT NULL UNIQUE,
                    list_id INTEGER NOT NULL,
                    barcode TEXT,
                    name TEXT NOT NULL,
                    quantity REAL NOT NULL,
                    unit TEXT NOT NULL,
                    estimated_price_cents INTEGER NOT NULL DEFAULT 0,
                    checked INTEGER NOT NULL DEFAULT 0,
                    updated_at TEXT NOT NULL,
                    FOREIGN KEY (list_id) REFERENCES shopping_lists(id) ON DELETE CASCADE
                );

                CREATE TABLE IF NOT EXISTS settings (
                    key TEXT PRIMARY KEY,
                    value TEXT NOT NULL
                );
                ",
            )
            .map_err(|e| format!("Greska inicijalizacije sheme: {e}"))?;

        init_ledger_schema(&self.conn)?;
        crate::migrations::run_migrations(&self.conn)?;
        Ok(())
    }

    pub fn node_id(&self) -> Result<String, String> {
        Ok(self.sync_identity().device_public_key)
    }

    pub fn ledger_stats(&self) -> Result<LedgerStats, String> {
        let node = self.node_id()?;
        let net = self.network_name()?;
        get_stats(&self.conn, &node, &net)
    }

    pub fn network_name(&self) -> Result<String, String> {
        if let Some(name) = self.get_setting("network_name")? {
            return Ok(name);
        }
        let default_name = "Obiteljska mreža".to_owned();
        self.set_setting("network_name", &default_name)?;
        Ok(default_name)
    }

    pub fn set_network_name(&self, name: &str) -> Result<String, String> {
        let trimmed = name.trim();
        let final_name = if trimmed.is_empty() {
            "Obiteljska mreža"
        } else {
            trimmed
        };
        self.set_setting("network_name", final_name)?;
        Ok(final_name.to_owned())
    }

    pub fn add_member(
        &self,
        name: &str,
        role: PeerRole,
        sas_code: &str,
        shared_lists: &str,
    ) -> Result<NetworkMember, String> {
        add_network_member(&self.conn, name, role, sas_code, shared_lists)
    }

    pub fn list_members(&self) -> Result<Vec<NetworkMember>, String> {
        list_network_members(&self.conn)
    }

    pub fn remove_member(&self, peer_id: &str) -> Result<(), String> {
        remove_network_member(&self.conn, peer_id)
    }

    pub fn compact_ledger(&self) -> Result<usize, String> {
        let deleted = auto_compact(&self.conn, CompactionPolicy::default())?;
        vacuum_db(&self.conn)?;
        self.set_setting(
            "maintenance_last_success",
            &Utc::now().timestamp().to_string(),
        )?;
        Ok(deleted)
    }

    /// At most once per day; a failed run remains due and can be retried.
    pub fn run_scheduled_maintenance(&self) -> Result<Option<usize>, String> {
        let now = Utc::now().timestamp();
        let last = self
            .get_setting("maintenance_last_success")?
            .and_then(|value| value.parse::<i64>().ok());
        if last.is_some_and(|last| now >= last && now - last < 86400) {
            return Ok(None);
        }
        let compacted = self.compact_ledger()?;
        let _ = self.gc_unreferenced_content();
        Ok(Some(compacted))
    }

    fn seed_default_stores(&self) -> Result<(), String> {
        self.local_seed_stores()
    }

    pub fn add_store(&self, name: &str, location: &str, color: &str) -> Result<Store, String> {
        self.local_add_store(name, location, color)
    }

    pub fn list_stores(&self) -> Result<Vec<Store>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "
                SELECT s.id, s.sync_id, s.name, s.location, s.color, s.updated_at,
                       (SELECT COUNT(DISTINCT ph.barcode) FROM price_history ph WHERE ph.store_name = s.name) as item_count
                FROM stores s
                  WHERE s.archived = 0
                ORDER BY s.id ASC
                ",
            )
            .map_err(|e| format!("Greska pripreme upita trgovina: {e}"))?;

        let rows = stmt
            .query_map([], |row| {
                Ok(Store {
                    id: row.get(0)?,
                    sync_id: row.get(1)?,
                    name: row.get(2)?,
                    location: row.get(3)?,
                    color: row.get(4)?,
                    updated_at: row.get(5)?,
                    item_count: row.get(6)?,
                })
            })
            .map_err(|e| format!("Greska citanja trgovina: {e}"))?;

        let mut stores = Vec::new();
        for row in rows {
            stores.push(row.map_err(|e| format!("Greska retka trgovine: {e}"))?);
        }
        Ok(stores)
    }

    pub fn upsert_product(&self, input: &UpsertProductInput<'_>) -> Result<Product, String> {
        self.local_upsert_product(input)
    }

    fn hydrate_product_image(&self, prod: &mut Product, image_hash: &str) {
        if prod.image_data_url.is_none()
            && !image_hash.is_empty()
            && let Ok(Some((mime, bytes))) = crate::content::load_verified_blob(
                &self.conn,
                self.blobs_dir().as_deref(),
                image_hash,
            )
        {
            prod.image_data_url = Some(crate::content::format_image_data_url(&mime, &bytes));
        }
    }

    pub fn get_product(&self, barcode: &str) -> Result<Option<Product>, String> {
        let matches: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM products WHERE barcode=?1 AND archived=0",
                [barcode.trim()],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if matches > 1 {
            return Err("Vise artikala ima isti barkod; odaberite stabilni identitet".into());
        }
        let mut stmt = self
            .conn
            .prepare(
                "SELECT barcode, sync_id, name, category, default_unit, last_price_cents, last_store_name, image_data_url, image_hash, updated_at
                 FROM products WHERE barcode = ?1 AND archived = 0",
            )
            .map_err(|e| format!("Greska upita artikla: {e}"))?;

        let row_opt = stmt
            .query_row(params![barcode.trim()], |row| {
                let unit_str: String = row.get(4)?;
                let inline_url: Option<String> = row.get(7)?;
                let image_hash: String = row.get(8)?;
                Ok((
                    Product {
                        barcode: row.get(0)?,
                        sync_id: row.get(1)?,
                        name: row.get(2)?,
                        category: row.get(3)?,
                        default_unit: Unit::parse(&unit_str).unwrap_or(Unit::Kom),
                        last_price_cents: row.get(5)?,
                        last_store_name: row.get(6)?,
                        image_data_url: inline_url,
                        updated_at: row.get(9)?,
                    },
                    image_hash,
                ))
            })
            .optional()
            .map_err(|e| format!("Greska citanja artikla: {e}"))?;

        Ok(row_opt.map(|(mut prod, image_hash)| {
            self.hydrate_product_image(&mut prod, &image_hash);
            prod
        }))
    }

    pub fn list_products(&self, search: Option<&str>) -> Result<Vec<Product>, String> {
        let pattern = search
            .map(|s| format!("%{}%", s.trim().to_lowercase()))
            .unwrap_or_else(|| "%".to_owned());

        let mut stmt = self
            .conn
            .prepare(
                "SELECT barcode, sync_id, name, category, default_unit, last_price_cents, last_store_name, image_data_url, image_hash, updated_at
                 FROM products
                   WHERE archived = 0 AND (LOWER(name) LIKE ?1 OR LOWER(barcode) LIKE ?1 OR LOWER(category) LIKE ?1)
                 ORDER BY updated_at DESC",
            )
            .map_err(|e| format!("Greska listanja artikala: {e}"))?;

        let rows = stmt
            .query_map(params![pattern], |row| {
                let unit_str: String = row.get(4)?;
                Ok((
                    Product {
                        barcode: row.get(0)?,
                        sync_id: row.get(1)?,
                        name: row.get(2)?,
                        category: row.get(3)?,
                        default_unit: Unit::parse(&unit_str).unwrap_or(Unit::Kom),
                        last_price_cents: row.get(5)?,
                        last_store_name: row.get(6)?,
                        image_data_url: row.get(7)?,
                        updated_at: row.get(9)?,
                    },
                    row.get::<_, String>(8)?,
                ))
            })
            .map_err(|e| format!("Greska citanja kataloga: {e}"))?;

        let mut products = Vec::new();
        for row in rows {
            let (mut prod, image_hash) = row.map_err(|e| format!("Greska retka artikla: {e}"))?;
            self.hydrate_product_image(&mut prod, &image_hash);
            products.push(prod);
        }
        Ok(products)
    }

    pub fn get_product_store_prices(&self, barcode: &str) -> Result<Vec<StorePrice>, String> {
        let matches: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM products WHERE barcode=?1 AND archived=0",
                [barcode.trim()],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if matches > 1 {
            return Err("Povijest cijena zahtijeva jednoznacan identitet artikla".into());
        }
        let mut stmt = self
            .conn
            .prepare(
                "
                SELECT ph.store_name, ph.price_cents, ph.recorded_at
                FROM price_history ph
                INNER JOIN (
                    SELECT store_name, MAX(id) AS max_id
                    FROM price_history
                      WHERE barcode = ?1 AND archived = 0
                        AND product_sync_id IN (SELECT sync_id FROM products WHERE barcode=?1 AND archived=0)
                    GROUP BY store_name
                ) latest ON ph.id = latest.max_id
                ORDER BY ph.price_cents ASC
                ",
            )
            .map_err(|e| format!("Greska upita cijena po trgovinama: {e}"))?;

        let rows = stmt
            .query_map(params![barcode.trim()], |row| {
                Ok(StorePrice {
                    store_name: row.get(0)?,
                    price_cents: row.get(1)?,
                    updated_at: row.get(2)?,
                    is_cheapest: false,
                })
            })
            .map_err(|e| format!("Greska citanja cijena po trgovinama: {e}"))?;

        let mut list = Vec::new();
        for (idx, row) in rows.enumerate() {
            let mut item = row.map_err(|e| format!("Greska retka cijene: {e}"))?;
            if idx == 0 {
                item.is_cheapest = true;
            }
            list.push(item);
        }
        Ok(list)
    }

    pub fn save_receipt(
        &self,
        store_id: i64,
        store_name: &str,
        items: &[CartItem],
        receipt_total_cents: i64,
    ) -> Result<ReceiptSummary, String> {
        self.local_save_receipt(store_id, store_name, items, receipt_total_cents)
    }

    pub fn list_receipts(&self) -> Result<Vec<ReceiptSummary>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, sync_id, store_id, store_name, created_at, cart_total_cents, receipt_total_cents, difference_cents
                 FROM receipts WHERE archived = 0 ORDER BY id DESC",
            )
            .map_err(|e| format!("Greska listanja racuna: {e}"))?;

        let rows = stmt
            .query_map([], |row| {
                Ok(ReceiptSummary {
                    id: row.get(0)?,
                    sync_id: row.get(1)?,
                    store_id: row.get(2)?,
                    store_name: row.get(3)?,
                    created_at: row.get(4)?,
                    items: Vec::new(),
                    cart_total_cents: row.get(5)?,
                    receipt_total_cents: row.get(6)?,
                    difference_cents: row.get(7)?,
                })
            })
            .map_err(|e| format!("Greska citanja racuna: {e}"))?;

        let mut receipts = Vec::new();
        for row in rows {
            let mut r = row.map_err(|e| format!("Greska retka racuna: {e}"))?;
            if !crate::apply::receipt_is_complete(&self.conn, &r.sync_id)? {
                continue;
            }
            r.items = self.get_receipt_items(r.id)?;
            receipts.push(r);
        }
        Ok(receipts)
    }

    pub fn incomplete_receipt_count(&self) -> Result<usize, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT sync_id FROM receipts WHERE archived=0")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?;
        let mut count = 0;
        for row in rows {
            if !crate::apply::receipt_is_complete(&self.conn, &row.map_err(|e| e.to_string())?)? {
                count += 1;
            }
        }
        Ok(count)
    }

    fn get_receipt_items(&self, receipt_id: i64) -> Result<Vec<CartItem>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, barcode, name, quantity, unit, unit_price_cents, total_cents
                 FROM receipt_items WHERE receipt_id = ?1 AND archived = 0 ORDER BY id ASC",
            )
            .map_err(|e| format!("Greska upita stavki racuna: {e}"))?;

        let rows = stmt
            .query_map(params![receipt_id], |row| {
                let id_num: i64 = row.get(0)?;
                let unit_str: String = row.get(4)?;
                Ok(CartItem {
                    id: id_num.to_string(),
                    barcode: row.get(1)?,
                    name: row.get(2)?,
                    quantity: row.get(3)?,
                    unit: Unit::parse(&unit_str).unwrap_or(Unit::Kom),
                    unit_price_cents: row.get(5)?,
                    total_cents: row.get(6)?,
                    image_data_url: None,
                })
            })
            .map_err(|e| format!("Greska citanja stavki racuna: {e}"))?;

        let mut items = Vec::new();
        for row in rows {
            items.push(row.map_err(|e| format!("Greska retka stavke: {e}"))?);
        }
        Ok(items)
    }

    pub fn create_shopping_list(
        &self,
        name: &str,
        store_id: Option<i64>,
    ) -> Result<ShoppingList, String> {
        self.local_create_list(name, store_id, false)
    }

    /// A list every member of the network sees and edits, family members included.
    /// Only a device that can admit members can make one.
    /// Deletes a list on every device; a shared one only from a device that manages
    /// the network.
    pub fn delete_shopping_list(&self, list_id: i64) -> Result<(), String> {
        self.local_delete_list(list_id)
    }

    /// Shares a private list: a shared copy with its items, the original archived.
    pub fn share_shopping_list(&self, list_id: i64) -> Result<ShoppingList, String> {
        self.local_set_list_shared(list_id, true)
    }

    /// Stops sharing: the shared list is archived and its items go back to the
    /// private list it came from (or to a new private list).
    pub fn unshare_shopping_list(&self, list_id: i64) -> Result<ShoppingList, String> {
        self.local_set_list_shared(list_id, false)
    }

    pub fn create_shared_shopping_list(
        &self,
        name: &str,
        store_id: Option<i64>,
    ) -> Result<ShoppingList, String> {
        self.local_create_list(name, store_id, true)
    }

    pub fn add_planned_item(
        &self,
        list_id: i64,
        barcode: Option<&str>,
        name: &str,
        quantity: f64,
        unit: Unit,
        estimated_price_cents: i64,
    ) -> Result<PlannedItem, String> {
        self.local_add_item(
            list_id,
            barcode,
            name,
            quantity,
            unit,
            estimated_price_cents,
        )
    }

    pub fn toggle_planned_item(&self, item_id: i64, checked: bool) -> Result<(), String> {
        self.local_toggle_item(item_id, checked)
    }

    pub fn remove_planned_item(&self, item_sync_id: &str) -> Result<(), String> {
        self.set_record_archived(
            crate::db::local::LocalEntity::PlannedItem,
            item_sync_id,
            true,
        )
    }

    pub fn list_shopping_lists(&self) -> Result<Vec<ShoppingList>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT sl.id, sl.sync_id, sl.name, sl.store_id, s.name, sl.updated_at,
                        EXISTS(SELECT 1 FROM record_versions rv WHERE rv.entity = 'shopping_lists'
                               AND rv.entity_id = sl.sync_id AND rv.channel = 'list:' || sl.sync_id)
                 FROM shopping_lists sl
                 LEFT JOIN stores s ON sl.store_id = s.id
                  WHERE sl.archived = 0
                 ORDER BY sl.id DESC",
            )
            .map_err(|e| format!("Greska listanja popisa: {e}"))?;

        let rows = stmt
            .query_map([], |row| {
                Ok(ShoppingList {
                    id: row.get(0)?,
                    sync_id: row.get(1)?,
                    name: row.get(2)?,
                    store_id: row.get(3)?,
                    store_name: row.get(4)?,
                    items: Vec::new(),
                    total_estimated_cents: 0,
                    updated_at: row.get(5)?,
                    shared: row.get(6)?,
                })
            })
            .map_err(|e| format!("Greska citanja popisa: {e}"))?;

        let mut lists = Vec::new();
        for row in rows {
            let mut list = row.map_err(|e| format!("Greska retka popisa: {e}"))?;
            list.items = self.get_planned_items(list.id)?;
            list.total_estimated_cents = Self::checked_list_total(&self.conn, list.id)?;
            lists.push(list);
        }
        Ok(lists)
    }

    fn get_planned_items(&self, list_id: i64) -> Result<Vec<PlannedItem>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, sync_id, list_id, barcode, name, quantity, unit, estimated_price_cents, checked, updated_at
                 FROM planned_items WHERE list_id = ?1 AND archived = 0 ORDER BY checked ASC, id ASC",
            )
            .map_err(|e| format!("Greska upita stavki popisa: {e}"))?;

        let rows = stmt
            .query_map(params![list_id], |row| {
                let unit_str: String = row.get(6)?;
                let chk: i32 = row.get(8)?;
                Ok(PlannedItem {
                    id: row.get(0)?,
                    sync_id: row.get(1)?,
                    list_id: row.get(2)?,
                    barcode: row.get(3)?,
                    name: row.get(4)?,
                    quantity: row.get(5)?,
                    unit: Unit::parse(&unit_str).unwrap_or(Unit::Kom),
                    estimated_price_cents: row.get(7)?,
                    checked: chk != 0,
                    updated_at: row.get(9)?,
                })
            })
            .map_err(|e| format!("Greska citanja stavki popisa: {e}"))?;

        let mut items = Vec::new();
        for row in rows {
            items.push(row.map_err(|e| format!("Greska retka stavke popisa: {e}"))?);
        }
        Ok(items)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map_err(|e| format!("Greska spremanja postavke: {e}"))?;
        Ok(())
    }

    pub fn get_setting(&self, key: &str) -> Result<Option<String>, String> {
        self.conn
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| format!("Greska citanja postavke: {e}"))
    }

    pub fn export_sync_bundle(&self) -> Result<SyncBundle, String> {
        self.export_sync_bundle_for_role(PeerRole::OwnDevice)
    }

    pub fn export_sync_bundle_for_role(&self, role: PeerRole) -> Result<SyncBundle, String> {
        self.export_sync_bundle_filtered(role, None)
    }

    pub fn export_sync_bundle_filtered(
        &self,
        role: PeerRole,
        shared_lists_override: Option<&str>,
    ) -> Result<SyncBundle, String> {
        let source_node_id = self.node_id()?;
        let network_name = self.network_name()?;
        let frontier = get_frontier(&self.conn)?;
        let all_versions = compute_delta(&self.conn, &BTreeMap::new(), 5000)?;
        let lists = self.list_shopping_lists()?;
        let members = self.list_members()?;

        match role {
            PeerRole::OwnDevice => {
                let own_sas = members
                    .iter()
                    .find(|m| m.role == PeerRole::OwnDevice)
                    .map(|m| m.sas_code.clone());
                Ok(SyncBundle {
                    version: 1,
                    exported_at: Utc::now().to_rfc3339(),
                    source_node_id,
                    network_name,
                    role: Some(PeerRole::OwnDevice),
                    sas_code: own_sas,
                    shared_lists: Some("*".to_owned()),
                    stores: self.list_stores()?,
                    products: self.list_products(None)?,
                    receipts: self.list_receipts()?,
                    lists,
                    frontier,
                    versions: all_versions,
                })
            }
            PeerRole::FamilyMember => {
                let family_members: Vec<&NetworkMember> = members
                    .iter()
                    .filter(|m| m.role == PeerRole::FamilyMember)
                    .collect();

                let effective_filter = match shared_lists_override
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    Some(explicit) => explicit.to_owned(),
                    None if !family_members.is_empty() => {
                        let any_wildcard = family_members
                            .iter()
                            .any(|m| shared_lists_allows(&m.shared_lists, "__any__", "__any__"));
                        if any_wildcard {
                            "*".to_owned()
                        } else {
                            family_members
                                .iter()
                                .map(|m| m.shared_lists.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        }
                    }
                    None => "*".to_owned(),
                };

                let family_sas = family_members.first().map(|m| m.sas_code.clone());

                let filtered_lists: Vec<ShoppingList> = lists
                    .into_iter()
                    .filter(|l| shared_lists_allows(&effective_filter, &l.name, &l.sync_id))
                    .collect();

                let allowed_list_sync_ids: HashSet<String> =
                    filtered_lists.iter().map(|l| l.sync_id.clone()).collect();
                let allowed_item_sync_ids: HashSet<String> = filtered_lists
                    .iter()
                    .flat_map(|l| l.items.iter().map(|i| i.sync_id.clone()))
                    .collect();
                let referenced_store_ids: HashSet<i64> =
                    filtered_lists.iter().filter_map(|l| l.store_id).collect();
                let referenced_store_names: HashSet<String> = filtered_lists
                    .iter()
                    .filter_map(|l| l.store_name.as_ref().map(|n| n.to_lowercase()))
                    .collect();
                let referenced_barcodes: HashSet<String> = filtered_lists
                    .iter()
                    .flat_map(|l| {
                        l.items
                            .iter()
                            .filter_map(|i| i.barcode.as_ref().map(|b| b.trim().to_owned()))
                    })
                    .filter(|b| !b.is_empty())
                    .collect();

                let stores: Vec<Store> = self
                    .list_stores()?
                    .into_iter()
                    .filter(|s| {
                        referenced_store_ids.contains(&s.id)
                            || referenced_store_names.contains(&s.name.to_lowercase())
                    })
                    .collect();

                let products: Vec<Product> = self
                    .list_products(None)?
                    .into_iter()
                    .filter(|p| referenced_barcodes.contains(p.barcode.trim()))
                    .collect();

                let versions = all_versions
                    .into_iter()
                    .filter(|v| {
                        (v.entity == "shopping_lists"
                            && allowed_list_sync_ids.contains(&v.entity_id))
                            || (v.entity == "planned_items"
                                && allowed_item_sync_ids.contains(&v.entity_id))
                    })
                    .collect();

                let family_frontier = frontier
                    .into_iter()
                    .filter(|(k, _)| k.ends_with("|obitelj"))
                    .collect();

                Ok(SyncBundle {
                    version: 1,
                    exported_at: Utc::now().to_rfc3339(),
                    source_node_id,
                    network_name,
                    role: Some(PeerRole::FamilyMember),
                    sas_code: family_sas,
                    shared_lists: Some(effective_filter),
                    stores,
                    products,
                    receipts: Vec::new(),
                    lists: filtered_lists,
                    frontier: family_frontier,
                    versions,
                })
            }
        }
    }

    pub fn import_sync_bundle(&self, bundle: &SyncBundle) -> Result<(usize, usize, usize), String> {
        if bundle.version != 1 {
            return Err("Nepodrzana inacica legacy paketa".into());
        }
        let local_node_id = self.node_id()?;
        let (effective_role, member_shared_lists) = verify_peer_identity(
            &self.conn,
            &local_node_id,
            &bundle.source_node_id,
            bundle.sas_code.as_deref(),
            bundle.role,
        )?;

        let effective_list_filter = member_shared_lists
            .as_deref()
            .or(bundle.shared_lists.as_deref())
            .unwrap_or("*");

        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| format!("Greska pokretanja transakcije uvoza: {e}"))?;

        // This is an explicit legacy snapshot import, never authenticated peer
        // admission. Ignore supplied version/author/role proofs entirely.
        // Changed snapshots are recorded as new local changes before commit.

        let mut added_stores = 0usize;
        for store in &bundle.stores {
            let existing: Option<(i64, String)> = tx
                .query_row(
                    "SELECT id, updated_at FROM stores WHERE sync_id = ?1",
                    params![store.sync_id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(|e| format!("Greska provjere trgovine: {e}"))?;

            let store_updated = if store.updated_at.trim().is_empty() {
                Utc::now().to_rfc3339()
            } else {
                store.updated_at.clone()
            };

            match existing {
                None => {
                    let sync_id = if store.sync_id.trim().is_empty() {
                        Uuid::new_v4().to_string()
                    } else {
                        store.sync_id.trim().to_owned()
                    };
                    tx.execute(
                        "INSERT INTO stores (sync_id, name, location, color, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                        params![
                            sync_id,
                            store.name.trim(),
                            store.location.trim(),
                            store.color.trim(),
                            store_updated
                        ],
                    )
                    .map_err(|e| format!("Greska uvoza trgovine: {e}"))?;
                    added_stores += 1;
                }
                Some((id, local_updated))
                    if !store.updated_at.is_empty() && store.updated_at > local_updated =>
                {
                    tx.execute(
                        "UPDATE stores SET location = ?1, color = ?2, updated_at = ?3 WHERE id = ?4",
                        params![store.location.trim(), store.color.trim(), store.updated_at, id],
                    )
                    .map_err(|e| format!("Greska azuriranja trgovine: {e}"))?;
                    added_stores += 1;
                }
                _ => {}
            }
        }

        let mut merged_products = 0usize;
        for prod in &bundle.products {
            let local_updated: Option<String> = tx
                .query_row(
                    "SELECT updated_at FROM products WHERE sync_id = ?1",
                    params![prod.sync_id.trim()],
                    |r| r.get(0),
                )
                .optional()
                .map_err(|e| format!("Greska provjere artikla pri uvozu: {e}"))?;

            let should_apply = match local_updated {
                None => true,
                Some(ref existing_ts) => prod.updated_at > *existing_ts,
            };

            if should_apply {
                let sync_id = if prod.sync_id.trim().is_empty() {
                    Uuid::new_v4().to_string()
                } else {
                    prod.sync_id.trim().to_owned()
                };
                let updated_at = if prod.updated_at.trim().is_empty() {
                    Utc::now().to_rfc3339()
                } else {
                    prod.updated_at.clone()
                };
                tx.execute(
                    "
                    INSERT INTO products (barcode, sync_id, name, category, default_unit, last_price_cents, last_store_name, image_data_url, updated_at)
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                      ON CONFLICT(sync_id) DO UPDATE SET
                        name = excluded.name,
                        category = excluded.category,
                        default_unit = excluded.default_unit,
                        last_price_cents = excluded.last_price_cents,
                        last_store_name = excluded.last_store_name,
                        image_data_url = COALESCE(excluded.image_data_url, products.image_data_url),
                        updated_at = excluded.updated_at
                    ",
                    params![
                        prod.barcode.trim(),
                        sync_id,
                        prod.name.trim(),
                        prod.category.trim(),
                        prod.default_unit.as_str(),
                        prod.last_price_cents,
                        prod.last_store_name.trim(),
                        prod.image_data_url,
                        updated_at
                    ],
                )
                .map_err(|e| format!("Greska uvoza artikla: {e}"))?;

                if prod.last_price_cents > 0 && !prod.last_store_name.trim().is_empty() {
                    tx.execute(
                        "INSERT INTO price_history (barcode, store_name, price_cents, recorded_at) VALUES (?1, ?2, ?3, ?4)",
                        params![
                            prod.barcode.trim(),
                            prod.last_store_name.trim(),
                            prod.last_price_cents,
                            updated_at
                        ],
                    )
                    .map_err(|e| format!("Greska uvoza povijesti cijene: {e}"))?;
                }
                merged_products += 1;
            }
        }

        if effective_role == PeerRole::OwnDevice {
            for receipt in &bundle.receipts {
                let exists: bool = tx
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM receipts WHERE sync_id = ?1)",
                        params![receipt.sync_id],
                        |r| r.get(0),
                    )
                    .map_err(|e| format!("Greska provjere racuna pri uvozu: {e}"))?;
                if exists {
                    continue;
                }

                let resolved_store_id = resolve_legacy_store(
                    &tx,
                    bundle,
                    Some(receipt.store_id),
                    Some(&receipt.store_name),
                )?
                .ok_or("Nedostaje trgovina legacy racuna")?;

                let normalized_items: Vec<CartItem> = receipt
                    .items
                    .iter()
                    .map(|item| {
                        let line_total = calculate_line_total_cents(
                            item.quantity,
                            item.unit,
                            item.unit_price_cents,
                        );
                        CartItem {
                            id: item.id.clone(),
                            barcode: item.barcode.trim().to_owned(),
                            name: item.name.trim().to_owned(),
                            quantity: item.quantity,
                            unit: item.unit,
                            unit_price_cents: item.unit_price_cents,
                            total_cents: line_total,
                            image_data_url: item.image_data_url.clone(),
                        }
                    })
                    .collect();

                if receipt.receipt_total_cents < 0 {
                    return Err("Iznos uvezenog racuna je negativan".into());
                }
                let cart_total_cents = normalized_items.iter().try_fold(0i64, |total, item| {
                    let line = crate::calc::checked_line_total_cents(
                        item.quantity,
                        item.unit,
                        item.unit_price_cents,
                    )?;
                    total
                        .checked_add(line)
                        .ok_or_else(|| "Zbroj uvezenog racuna prelazi raspon".to_owned())
                })?;
                let difference_cents = cart_total_cents - receipt.receipt_total_cents;

                tx.execute(
                    "INSERT INTO receipts (sync_id, store_id, store_name, created_at, cart_total_cents, receipt_total_cents, difference_cents)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        receipt.sync_id,
                        resolved_store_id,
                        receipt.store_name.trim(),
                        receipt.created_at,
                        cart_total_cents,
                        receipt.receipt_total_cents,
                        difference_cents
                    ],
                )
                .map_err(|e| format!("Greska uvoza racuna: {e}"))?;

                let receipt_id = tx.last_insert_rowid();
                for item in &normalized_items {
                    tx.execute(
                        "INSERT INTO receipt_items (receipt_id, barcode, name, quantity, unit, unit_price_cents, total_cents)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        params![
                            receipt_id,
                            item.barcode,
                            item.name,
                            item.quantity,
                            item.unit.as_str(),
                            item.unit_price_cents,
                            item.total_cents
                        ],
                    )
                    .map_err(|e| format!("Greska uvoza stavke racuna: {e}"))?;
                }
            }
        }

        let mut added_lists = 0usize;
        for list in &bundle.lists {
            if effective_role == PeerRole::FamilyMember
                && !shared_lists_allows(effective_list_filter, &list.name, &list.sync_id)
            {
                continue;
            }

            let resolved_store_id =
                resolve_legacy_store(&tx, bundle, list.store_id, list.store_name.as_deref())?;

            let existing_list: Option<(i64, String, Option<i64>, String)> = tx
                .query_row(
                    "SELECT id, name, store_id, updated_at FROM shopping_lists WHERE sync_id = ?1",
                    params![list.sync_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .optional()
                .map_err(|e| format!("Greska provjere popisa: {e}"))?;

            let list_updated = if list.updated_at.trim().is_empty() {
                Utc::now().to_rfc3339()
            } else {
                list.updated_at.clone()
            };

            match existing_list {
                None => {
                    tx.execute(
                        "INSERT INTO shopping_lists (sync_id, name, store_id, updated_at) VALUES (?1, ?2, ?3, ?4)",
                        params![list.sync_id, list.name.trim(), resolved_store_id, list_updated],
                    )
                    .map_err(|e| format!("Greska uvoza popisa: {e}"))?;
                    let list_id = tx.last_insert_rowid();
                    for item in &list.items {
                        let item_sync_id = if item.sync_id.trim().is_empty() {
                            Uuid::new_v4().to_string()
                        } else {
                            item.sync_id.trim().to_owned()
                        };
                        let item_updated = if item.updated_at.trim().is_empty() {
                            list_updated.clone()
                        } else {
                            item.updated_at.clone()
                        };
                        tx.execute(
                            "INSERT INTO planned_items (sync_id, list_id, barcode, name, quantity, unit, estimated_price_cents, checked, updated_at)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                            params![
                                item_sync_id,
                                list_id,
                                item.barcode.as_deref(),
                                item.name.trim(),
                                item.quantity,
                                item.unit.as_str(),
                                item.estimated_price_cents,
                                i32::from(item.checked),
                                item_updated
                            ],
                        )
                        .map_err(|e| format!("Greska uvoza stavke popisa: {e}"))?;
                    }
                    added_lists += 1;
                }
                Some((list_id, local_name, local_store_id, local_updated_at)) => {
                    let mut list_changed = false;
                    if list_updated > local_updated_at
                        && (list.name.trim() != local_name || resolved_store_id != local_store_id)
                    {
                        tx.execute(
                            "UPDATE shopping_lists SET name = ?1, store_id = COALESCE(?2, store_id), updated_at = ?3 WHERE id = ?4",
                            params![list.name.trim(), resolved_store_id, list_updated, list_id],
                        )
                        .map_err(|e| format!("Greska azuriranja popisa: {e}"))?;
                        list_changed = true;
                    }

                    for item in &list.items {
                        let existing_item: Option<(i64, String)> = tx
                            .query_row(
                                "SELECT id, updated_at FROM planned_items WHERE sync_id = ?1",
                                params![item.sync_id],
                                |r| Ok((r.get(0)?, r.get(1)?)),
                            )
                            .optional()
                            .map_err(|e| format!("Greska provjere stavke popisa: {e}"))?;

                        let item_updated = if item.updated_at.trim().is_empty() {
                            list_updated.clone()
                        } else {
                            item.updated_at.clone()
                        };

                        match existing_item {
                            None => {
                                let item_sync_id = if item.sync_id.trim().is_empty() {
                                    Uuid::new_v4().to_string()
                                } else {
                                    item.sync_id.trim().to_owned()
                                };
                                tx.execute(
                                    "INSERT INTO planned_items (sync_id, list_id, barcode, name, quantity, unit, estimated_price_cents, checked, updated_at)
                                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                                    params![
                                        item_sync_id,
                                        list_id,
                                        item.barcode.as_deref(),
                                        item.name.trim(),
                                        item.quantity,
                                        item.unit.as_str(),
                                        item.estimated_price_cents,
                                        i32::from(item.checked),
                                        item_updated
                                    ],
                                )
                                .map_err(|e| format!("Greska dodavanja nove stavke u postojeci popis: {e}"))?;
                                list_changed = true;
                            }
                            Some((local_item_id, local_item_updated))
                                if item_updated > local_item_updated =>
                            {
                                tx.execute(
                                    "UPDATE planned_items
                                     SET barcode = ?1, name = ?2, quantity = ?3, unit = ?4, estimated_price_cents = ?5, checked = ?6, updated_at = ?7
                                     WHERE id = ?8",
                                    params![
                                        item.barcode.as_deref(),
                                        item.name.trim(),
                                        item.quantity,
                                        item.unit.as_str(),
                                        item.estimated_price_cents,
                                        i32::from(item.checked),
                                        item_updated,
                                        local_item_id
                                    ],
                                )
                                .map_err(|e| format!("Greska azuriranja stavke popisa pri uvozu: {e}"))?;
                                list_changed = true;
                            }
                            _ => {}
                        }
                    }

                    if list_changed {
                        let newest_ts = if list_updated > local_updated_at {
                            list_updated
                        } else {
                            Utc::now().to_rfc3339()
                        };
                        tx.execute(
                            "UPDATE shopping_lists SET updated_at = ?1 WHERE id = ?2",
                            params![newest_ts, list_id],
                        )
                        .map_err(|e| format!("Greska azuriranja vremena popisa: {e}"))?;
                        added_lists += 1;
                    }
                }
            }
        }

        self.capture_existing(&tx)?;
        tx.commit()
            .map_err(|e| format!("Greska potvrde uvoza paketa: {e}"))?;

        Ok((added_stores, merged_products, added_lists))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_default_stores_and_adds_custom_store() {
        let db = Database::open_in_memory().expect("in-memory db");
        let stores = db.list_stores().expect("list stores");
        assert_eq!(stores.len(), 5);

        let plodine = db
            .add_store("Plodine", "Rijeka", "#EF4444")
            .expect("add custom store");
        assert_eq!(plodine.name, "Plodine");
        assert_eq!(db.list_stores().expect("list").len(), 6);
    }

    #[test]
    fn saves_product_with_photo_and_tracks_store_prices() {
        let db = Database::open_in_memory().expect("db");
        db.upsert_product(&UpsertProductInput {
            barcode: "3850102123",
            name: "Barilla Spageti 500g",
            category: "Tjestenina",
            default_unit: Unit::Kom,
            price_cents: 149,
            store_name: "Super Konzum",
            image_data_url: Some("data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII="),
        })
        .expect("upsert 1");

        db.upsert_product(&UpsertProductInput {
            barcode: "3850102123",
            name: "Barilla Spageti 500g",
            category: "Tjestenina",
            default_unit: Unit::Kom,
            price_cents: 139,
            store_name: "Kaufland",
            image_data_url: None,
        })
        .expect("upsert 2");

        let prod = db.get_product("3850102123").expect("get").expect("exists");
        assert_eq!(
            prod.image_data_url.as_deref(),
            Some(
                "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII="
            )
        );

        let prices = db.get_product_store_prices("3850102123").expect("prices");
        assert_eq!(prices.len(), 2);
        assert_eq!(prices[0].store_name, "Kaufland");
        assert!(prices[0].is_cheapest);
        assert_eq!(prices[0].price_cents, 139);
    }

    #[test]
    fn saves_receipt_atomically_and_recalculates_totals_consistently() {
        let db = Database::open_in_memory().expect("db");
        let stores = db.list_stores().expect("stores");
        let store_id = stores[0].id;

        // Caller passes an inconsistent total_cents (999) for 2 * 149 = 298
        let items = vec![CartItem {
            id: "1".to_owned(),
            barcode: "3850102123".to_owned(),
            name: "Barilla Spageti".to_owned(),
            quantity: 2.0,
            unit: Unit::Kom,
            unit_price_cents: 149,
            total_cents: 999,
            image_data_url: None,
        }];

        let receipt = db
            .save_receipt(store_id, "Super Konzum", &items, 298)
            .expect("save receipt");
        assert_eq!(receipt.cart_total_cents, 298);
        assert_eq!(receipt.items[0].total_cents, 298);
        assert_eq!(receipt.difference_cents, 0);

        let stored = db.list_receipts().expect("list receipts");
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].cart_total_cents, 298);
        assert_eq!(stored[0].items[0].total_cents, 298);

        let list = db
            .create_shopping_list("Tjedna nabavka", Some(store_id))
            .expect("create list");
        let planned = db
            .add_planned_item(list.id, None, "Kruh", 1.0, Unit::Kom, 120)
            .expect("add planned");
        db.toggle_planned_item(planned.id, true).expect("toggle");

        let lists = db.list_shopping_lists().expect("lists");
        assert_eq!(lists.len(), 1);
        assert_eq!(lists[0].total_estimated_cents, 120);
        assert!(lists[0].items[0].checked);
    }

    #[test]
    fn exports_and_merges_sync_bundle_with_receipts_and_list_updates() {
        let db1 = Database::open_in_memory().expect("db1");
        let plodine = db1.add_store("Plodine", "Split", "#EF4444").expect("store");
        db1.upsert_product(&UpsertProductInput {
            barcode: "385111222333",
            name: "Kava 250g",
            category: "Napici",
            default_unit: Unit::Kom,
            price_cents: 340,
            store_name: "Plodine",
            image_data_url: None,
        })
        .expect("prod");
        db1.save_receipt(
            plodine.id,
            "Plodine",
            &[CartItem {
                id: "1".to_owned(),
                barcode: "385111222333".to_owned(),
                name: "Kava 250g".to_owned(),
                quantity: 1.0,
                unit: Unit::Kom,
                unit_price_cents: 340,
                total_cents: 340,
                image_data_url: None,
            }],
            340,
        )
        .expect("save receipt");

        let l = db1
            .create_shopping_list("Subota", Some(plodine.id))
            .expect("create list");
        let item1 = db1
            .add_planned_item(l.id, Some("385111222333"), "Kava 250g", 1.0, Unit::Kom, 340)
            .expect("item");

        let bundle = db1.export_sync_bundle().expect("export");
        assert_eq!(bundle.receipts.len(), 1);

        let db2 = Database::open_in_memory().expect("db2");
        let (s1, p1, l1) = db2.import_sync_bundle(&bundle).expect("import 1");
        assert_eq!(s1, bundle.stores.len());
        assert_eq!(p1, 1);
        assert_eq!(l1, 1);
        assert_eq!(db2.list_receipts().expect("receipts on db2").len(), 1);

        // Importing the identical bundle again should be idempotent
        let (s2, _p2, l2) = db2.import_sync_bundle(&bundle).expect("import 2");
        assert_eq!(s2, 0);
        assert_eq!(l2, 0);
        assert_eq!(db2.list_receipts().expect("receipts still 1").len(), 1);

        // Now modify the existing list on db1 (toggle item1 + add a second item) and re-export
        db1.toggle_planned_item(item1.id, true)
            .expect("toggle item1");
        db1.add_planned_item(l.id, None, "Mlijeko", 2.0, Unit::L, 110)
            .expect("add item2");

        let updated_bundle = db1.export_sync_bundle().expect("export updated");
        let (_s3, _p3, l3) = db2
            .import_sync_bundle(&updated_bundle)
            .expect("import updated");
        assert_eq!(l3, 1);

        let db2_lists = db2.list_shopping_lists().expect("db2 lists");
        assert_eq!(db2_lists.len(), 1);
        assert_eq!(db2_lists[0].items.len(), 2);
        let kava = db2_lists[0]
            .items
            .iter()
            .find(|i| i.name == "Kava 250g")
            .expect("kava item");
        assert!(kava.checked);
    }

    #[test]
    fn family_export_filters_by_shared_lists_and_excludes_receipts() {
        let db = Database::open_in_memory().expect("db");
        let stores = db.list_stores().expect("stores");
        let konzum = &stores[0];
        let lidl = &stores[2];

        let l1 = db
            .create_shopping_list("Obiteljska subota", Some(konzum.id))
            .expect("l1");
        db.add_planned_item(l1.id, None, "Kruh", 1.0, Unit::Kom, 150)
            .expect("i1");

        let l2 = db
            .create_shopping_list("Osobni popis", Some(lidl.id))
            .expect("l2");
        db.add_planned_item(l2.id, None, "Tajni poklon", 1.0, Unit::Kom, 5000)
            .expect("i2");

        db.save_receipt(konzum.id, &konzum.name, &[], 0)
            .expect("receipt");
        db.add_member(
            "Ana",
            PeerRole::FamilyMember,
            "482-915",
            "Obiteljska subota",
        )
        .expect("add family member");

        let fam_bundle = db
            .export_sync_bundle_for_role(PeerRole::FamilyMember)
            .expect("family export");

        assert!(fam_bundle.receipts.is_empty());
        assert_eq!(fam_bundle.lists.len(), 1);
        assert_eq!(fam_bundle.lists[0].name, "Obiteljska subota");
        assert_eq!(fam_bundle.stores.len(), 1);
        assert_eq!(fam_bundle.stores[0].name, konzum.name);
    }
}
