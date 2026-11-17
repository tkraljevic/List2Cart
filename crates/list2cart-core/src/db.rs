use crate::calc::{calculate_cart_total_cents, calculate_line_total_cents};
use crate::ledger::{
    CompactionPolicy, LedgerStats, NetworkMember, PeerRole, add_network_member, auto_compact,
    compute_delta, get_frontier, get_stats, ingest_remote_versions, init_ledger_schema,
    list_network_members, record_version, remove_network_member, vacuum_db,
};
use crate::models::{
    CartItem, PlannedItem, Product, ReceiptSummary, ShoppingList, Store, StorePrice, SyncBundle,
    Unit,
};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, params};
use std::collections::BTreeMap;
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
}

impl Database {
    pub fn open(path: &Path) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| format!("Greska baze podataka: {e}"))?;
        let db = Self { conn };
        db.init_schema()?;
        Ok(db)
    }

    pub fn open_in_memory() -> Result<Self, String> {
        let conn =
            Connection::open_in_memory().map_err(|e| format!("Greska memorije baze: {e}"))?;
        let db = Self { conn };
        db.init_schema()?;
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
        let _ = self.node_id()?;
        self.seed_default_stores()?;
        let _ = auto_compact(&self.conn, CompactionPolicy::default());
        Ok(())
    }

    pub fn node_id(&self) -> Result<String, String> {
        if let Some(id) = self.get_setting("node_id")? {
            return Ok(id);
        }
        let id = format!("l2c-{}", &Uuid::new_v4().to_string()[..8]);
        self.set_setting("node_id", &id)?;
        Ok(id)
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
        if deleted == 0 {
            vacuum_db(&self.conn)?;
        }
        Ok(deleted)
    }

    fn seed_default_stores(&self) -> Result<(), String> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM stores", [], |r| r.get(0))
            .map_err(|e| format!("Greska brojanja trgovina: {e}"))?;

        if count == 0 {
            let defaults = [
                ("Super Konzum", "Zagreb", "#DC2626"),
                ("Kaufland", "Zagreb", "#1768D1"),
                ("Lidl", "Zagreb", "#D97706"),
                ("Spar", "Zagreb", "#16A34A"),
                ("DM Drogerie Markt", "Centar", "#0284C7"),
            ];
            for (name, location, color) in defaults {
                self.add_store(name, location, color)?;
            }
        }
        Ok(())
    }

    pub fn add_store(&self, name: &str, location: &str, color: &str) -> Result<Store, String> {
        let sync_id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        self.conn
            .execute(
                "INSERT INTO stores (sync_id, name, location, color, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![sync_id, name.trim(), location.trim(), color.trim(), now],
            )
            .map_err(|e| format!("Greska dodavanja trgovine: {e}"))?;
        let id = self.conn.last_insert_rowid();
        let store = Store {
            id,
            sync_id: sync_id.clone(),
            name: name.trim().to_owned(),
            location: location.trim().to_owned(),
            color: color.trim().to_owned(),
            item_count: 0,
            updated_at: now,
        };
        if let (Ok(node), Ok(payload)) = (self.node_id(), serde_json::to_string(&store)) {
            let _ = record_version(&self.conn, &node, "stores", &sync_id, false, &payload, "");
        }
        Ok(store)
    }

    pub fn list_stores(&self) -> Result<Vec<Store>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "
                SELECT s.id, s.sync_id, s.name, s.location, s.color, s.updated_at,
                       (SELECT COUNT(DISTINCT ph.barcode) FROM price_history ph WHERE ph.store_name = s.name) as item_count
                FROM stores s
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
        let now = Utc::now().to_rfc3339();
        let existing_sync_id: Option<String> = self
            .conn
            .query_row(
                "SELECT sync_id FROM products WHERE barcode = ?1",
                params![input.barcode],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| format!("Greska provjere artikla: {e}"))?;

        let sync_id = existing_sync_id.unwrap_or_else(|| Uuid::new_v4().to_string());

        self.conn
            .execute(
                "
                INSERT INTO products (barcode, sync_id, name, category, default_unit, last_price_cents, last_store_name, image_data_url, updated_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                ON CONFLICT(barcode) DO UPDATE SET
                    name = excluded.name,
                    category = excluded.category,
                    default_unit = excluded.default_unit,
                    last_price_cents = excluded.last_price_cents,
                    last_store_name = excluded.last_store_name,
                    image_data_url = COALESCE(excluded.image_data_url, products.image_data_url),
                    updated_at = excluded.updated_at
                ",
                params![
                    input.barcode.trim(),
                    sync_id,
                    input.name.trim(),
                    input.category.trim(),
                    input.default_unit.as_str(),
                    input.price_cents,
                    input.store_name.trim(),
                    input.image_data_url,
                    now
                ],
            )
            .map_err(|e| format!("Greska spremanja artikla: {e}"))?;

        if input.price_cents > 0 && !input.store_name.trim().is_empty() {
            self.conn
                .execute(
                    "INSERT INTO price_history (barcode, store_name, price_cents, recorded_at) VALUES (?1, ?2, ?3, ?4)",
                    params![input.barcode.trim(), input.store_name.trim(), input.price_cents, now],
                )
                .map_err(|e| format!("Greska spremanja povijesti cijena: {e}"))?;
        }

        let prod = self
            .get_product(input.barcode)?
            .ok_or_else(|| "Artikl nije pronadjen nakon spremanja".to_owned())?;
        if let (Ok(node), Ok(payload)) = (self.node_id(), serde_json::to_string(&prod)) {
            let _ = record_version(
                &self.conn,
                &node,
                "products",
                &prod.barcode,
                false,
                &payload,
                "",
            );
        }
        Ok(prod)
    }

    pub fn get_product(&self, barcode: &str) -> Result<Option<Product>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT barcode, sync_id, name, category, default_unit, last_price_cents, last_store_name, image_data_url, updated_at
                 FROM products WHERE barcode = ?1",
            )
            .map_err(|e| format!("Greska upita artikla: {e}"))?;

        stmt.query_row(params![barcode.trim()], |row| {
            let unit_str: String = row.get(4)?;
            Ok(Product {
                barcode: row.get(0)?,
                sync_id: row.get(1)?,
                name: row.get(2)?,
                category: row.get(3)?,
                default_unit: Unit::parse(&unit_str).unwrap_or(Unit::Kom),
                last_price_cents: row.get(5)?,
                last_store_name: row.get(6)?,
                image_data_url: row.get(7)?,
                updated_at: row.get(8)?,
            })
        })
        .optional()
        .map_err(|e| format!("Greska citanja artikla: {e}"))
    }

    pub fn list_products(&self, search: Option<&str>) -> Result<Vec<Product>, String> {
        let pattern = search
            .map(|s| format!("%{}%", s.trim().to_lowercase()))
            .unwrap_or_else(|| "%".to_owned());

        let mut stmt = self
            .conn
            .prepare(
                "SELECT barcode, sync_id, name, category, default_unit, last_price_cents, last_store_name, image_data_url, updated_at
                 FROM products
                 WHERE LOWER(name) LIKE ?1 OR LOWER(barcode) LIKE ?1 OR LOWER(category) LIKE ?1
                 ORDER BY updated_at DESC",
            )
            .map_err(|e| format!("Greska listanja artikala: {e}"))?;

        let rows = stmt
            .query_map(params![pattern], |row| {
                let unit_str: String = row.get(4)?;
                Ok(Product {
                    barcode: row.get(0)?,
                    sync_id: row.get(1)?,
                    name: row.get(2)?,
                    category: row.get(3)?,
                    default_unit: Unit::parse(&unit_str).unwrap_or(Unit::Kom),
                    last_price_cents: row.get(5)?,
                    last_store_name: row.get(6)?,
                    image_data_url: row.get(7)?,
                    updated_at: row.get(8)?,
                })
            })
            .map_err(|e| format!("Greska citanja kataloga: {e}"))?;

        let mut products = Vec::new();
        for row in rows {
            products.push(row.map_err(|e| format!("Greska retka artikla: {e}"))?);
        }
        Ok(products)
    }

    pub fn get_product_store_prices(&self, barcode: &str) -> Result<Vec<StorePrice>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "
                SELECT ph.store_name, ph.price_cents, ph.recorded_at
                FROM price_history ph
                INNER JOIN (
                    SELECT store_name, MAX(id) AS max_id
                    FROM price_history
                    WHERE barcode = ?1
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
        let sync_id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let cart_total_cents = calculate_cart_total_cents(items);
        let difference_cents = cart_total_cents - receipt_total_cents;

        self.conn
            .execute(
                "INSERT INTO receipts (sync_id, store_id, store_name, created_at, cart_total_cents, receipt_total_cents, difference_cents)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    sync_id,
                    store_id,
                    store_name,
                    now,
                    cart_total_cents,
                    receipt_total_cents,
                    difference_cents
                ],
            )
            .map_err(|e| format!("Greska spremanja racuna: {e}"))?;

        let receipt_id = self.conn.last_insert_rowid();

        for item in items {
            let line_total =
                calculate_line_total_cents(item.quantity, item.unit, item.unit_price_cents);
            self.conn
                .execute(
                    "INSERT INTO receipt_items (receipt_id, barcode, name, quantity, unit, unit_price_cents, total_cents)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        receipt_id,
                        item.barcode,
                        item.name,
                        item.quantity,
                        item.unit.as_str(),
                        item.unit_price_cents,
                        line_total
                    ],
                )
                .map_err(|e| format!("Greska spremanja stavke racuna: {e}"))?;

            if !item.barcode.trim().is_empty() {
                let _ = self.upsert_product(&UpsertProductInput {
                    barcode: &item.barcode,
                    name: &item.name,
                    category: "Opce",
                    default_unit: item.unit,
                    price_cents: item.unit_price_cents,
                    store_name,
                    image_data_url: item.image_data_url.as_deref(),
                });
            }
        }

        Ok(ReceiptSummary {
            id: receipt_id,
            sync_id,
            store_id,
            store_name: store_name.to_owned(),
            created_at: now,
            items: items.to_vec(),
            cart_total_cents,
            receipt_total_cents,
            difference_cents,
        })
    }

    pub fn list_receipts(&self) -> Result<Vec<ReceiptSummary>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, sync_id, store_id, store_name, created_at, cart_total_cents, receipt_total_cents, difference_cents
                 FROM receipts ORDER BY id DESC",
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
            r.items = self.get_receipt_items(r.id)?;
            receipts.push(r);
        }
        Ok(receipts)
    }

    fn get_receipt_items(&self, receipt_id: i64) -> Result<Vec<CartItem>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, barcode, name, quantity, unit, unit_price_cents, total_cents
                 FROM receipt_items WHERE receipt_id = ?1 ORDER BY id ASC",
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
        let sync_id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        self.conn
            .execute(
                "INSERT INTO shopping_lists (sync_id, name, store_id, updated_at) VALUES (?1, ?2, ?3, ?4)",
                params![sync_id, name.trim(), store_id, now],
            )
            .map_err(|e| format!("Greska kreiranja popisa: {e}"))?;
        let id = self.conn.last_insert_rowid();
        Ok(ShoppingList {
            id,
            sync_id,
            name: name.trim().to_owned(),
            store_id,
            store_name: None,
            items: Vec::new(),
            total_estimated_cents: 0,
            updated_at: now,
        })
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
        let sync_id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        self.conn
            .execute(
                "INSERT INTO planned_items (sync_id, list_id, barcode, name, quantity, unit, estimated_price_cents, checked, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8)",
                params![
                    sync_id,
                    list_id,
                    barcode,
                    name.trim(),
                    quantity,
                    unit.as_str(),
                    estimated_price_cents,
                    now
                ],
            )
            .map_err(|e| format!("Greska dodavanja na popis: {e}"))?;
        let id = self.conn.last_insert_rowid();
        Ok(PlannedItem {
            id,
            sync_id,
            list_id,
            barcode: barcode.map(ToOwned::to_owned),
            name: name.trim().to_owned(),
            quantity,
            unit,
            estimated_price_cents,
            checked: false,
            updated_at: now,
        })
    }

    pub fn toggle_planned_item(&self, item_id: i64, checked: bool) -> Result<(), String> {
        let now = Utc::now().to_rfc3339();
        self.conn
            .execute(
                "UPDATE planned_items SET checked = ?1, updated_at = ?2 WHERE id = ?3",
                params![i32::from(checked), now, item_id],
            )
            .map_err(|e| format!("Greska azuriranja stavke popisa: {e}"))?;
        Ok(())
    }

    pub fn list_shopping_lists(&self) -> Result<Vec<ShoppingList>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT sl.id, sl.sync_id, sl.name, sl.store_id, s.name, sl.updated_at
                 FROM shopping_lists sl
                 LEFT JOIN stores s ON sl.store_id = s.id
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
                })
            })
            .map_err(|e| format!("Greska citanja popisa: {e}"))?;

        let mut lists = Vec::new();
        for row in rows {
            let mut list = row.map_err(|e| format!("Greska retka popisa: {e}"))?;
            list.items = self.get_planned_items(list.id)?;
            list.total_estimated_cents = list
                .items
                .iter()
                .map(|i| calculate_line_total_cents(i.quantity, i.unit, i.estimated_price_cents))
                .sum();
            lists.push(list);
        }
        Ok(lists)
    }

    fn get_planned_items(&self, list_id: i64) -> Result<Vec<PlannedItem>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, sync_id, list_id, barcode, name, quantity, unit, estimated_price_cents, checked, updated_at
                 FROM planned_items WHERE list_id = ?1 ORDER BY checked ASC, id ASC",
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
        let frontier = get_frontier(&self.conn)?;
        let all_versions = compute_delta(&self.conn, &BTreeMap::new(), 5000)?;
        let lists = self.list_shopping_lists()?;

        match role {
            PeerRole::OwnDevice => Ok(SyncBundle {
                version: 1,
                exported_at: Utc::now().to_rfc3339(),
                stores: self.list_stores()?,
                products: self.list_products(None)?,
                lists,
                frontier,
                versions: all_versions,
            }),
            PeerRole::FamilyMember => {
                // Članovima obitelji šaljemo SAMO liste za kupnju i artikle/trgovine koji se nalaze na tim listama
                let versions = all_versions
                    .into_iter()
                    .filter(|v| v.channel == "obitelj" || v.entity == "shopping_lists")
                    .collect();
                Ok(SyncBundle {
                    version: 1,
                    exported_at: Utc::now().to_rfc3339(),
                    stores: self.list_stores()?,
                    products: Vec::new(),
                    lists,
                    frontier,
                    versions,
                })
            }
        }
    }

    pub fn import_sync_bundle(&self, bundle: &SyncBundle) -> Result<(usize, usize, usize), String> {
        if !bundle.versions.is_empty() {
            let _ = ingest_remote_versions(&self.conn, &bundle.versions);
        }

        let mut added_stores = 0usize;
        for store in &bundle.stores {
            let exists: bool = self
                .conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM stores WHERE sync_id = ?1 OR name = ?2)",
                    params![store.sync_id, store.name],
                    |r| r.get(0),
                )
                .map_err(|e| format!("Greska provjere trgovine: {e}"))?;
            if !exists
                && self
                    .add_store(&store.name, &store.location, &store.color)
                    .is_ok()
            {
                added_stores += 1;
            }
        }

        let mut merged_products = 0usize;
        for prod in &bundle.products {
            let local = self.get_product(&prod.barcode)?;
            let should_apply = match local {
                None => true,
                Some(ref existing) => prod.updated_at > existing.updated_at,
            };
            if should_apply {
                self.upsert_product(&UpsertProductInput {
                    barcode: &prod.barcode,
                    name: &prod.name,
                    category: &prod.category,
                    default_unit: prod.default_unit,
                    price_cents: prod.last_price_cents,
                    store_name: &prod.last_store_name,
                    image_data_url: prod.image_data_url.as_deref(),
                })?;
                merged_products += 1;
            }
        }

        let mut added_lists = 0usize;
        for list in &bundle.lists {
            let exists: bool = self
                .conn
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM shopping_lists WHERE sync_id = ?1)",
                    params![list.sync_id],
                    |r| r.get(0),
                )
                .map_err(|e| format!("Greska provjere popisa: {e}"))?;
            if !exists {
                let now = Utc::now().to_rfc3339();
                self.conn
                    .execute(
                        "INSERT INTO shopping_lists (sync_id, name, store_id, updated_at) VALUES (?1, ?2, NULL, ?3)",
                        params![list.sync_id, list.name, now],
                    )
                    .map_err(|e| format!("Greska uvoza popisa: {e}"))?;
                let list_id = self.conn.last_insert_rowid();
                for item in &list.items {
                    let _ = self.add_planned_item(
                        list_id,
                        item.barcode.as_deref(),
                        &item.name,
                        item.quantity,
                        item.unit,
                        item.estimated_price_cents,
                    );
                }
                added_lists += 1;
            }
        }

        let _ = auto_compact(&self.conn, CompactionPolicy::default());
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
            image_data_url: Some("data:image/webp;base64,AAAA"),
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
            Some("data:image/webp;base64,AAAA")
        );

        let prices = db.get_product_store_prices("3850102123").expect("prices");
        assert_eq!(prices.len(), 2);
        assert_eq!(prices[0].store_name, "Kaufland");
        assert!(prices[0].is_cheapest);
        assert_eq!(prices[0].price_cents, 139);
    }

    #[test]
    fn saves_receipt_and_shopping_list() {
        let db = Database::open_in_memory().expect("db");
        let stores = db.list_stores().expect("stores");
        let store_id = stores[0].id;

        let items = vec![CartItem {
            id: "1".to_owned(),
            barcode: "3850102123".to_owned(),
            name: "Barilla Spageti".to_owned(),
            quantity: 2.0,
            unit: Unit::Kom,
            unit_price_cents: 149,
            total_cents: 298,
            image_data_url: None,
        }];

        let receipt = db
            .save_receipt(store_id, "Super Konzum", &items, 298)
            .expect("save receipt");
        assert_eq!(receipt.difference_cents, 0);

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
    fn exports_and_merges_sync_bundle_without_duplicates() {
        let db1 = Database::open_in_memory().expect("db1");
        db1.add_store("Plodine", "Split", "#EF4444").expect("store");
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
        let l = db1
            .create_shopping_list("Subota", None)
            .expect("create list");
        db1.add_planned_item(l.id, Some("385111222333"), "Kava 250g", 1.0, Unit::Kom, 340)
            .expect("item");

        let bundle = db1.export_sync_bundle().expect("export");

        let db2 = Database::open_in_memory().expect("db2");
        let (s1, p1, l1) = db2.import_sync_bundle(&bundle).expect("import 1");
        assert_eq!(s1, 1);
        assert_eq!(p1, 1);
        assert_eq!(l1, 1);

        // Importing again should be idempotent (no duplicate stores or lists)
        let (s2, _p2, l2) = db2.import_sync_bundle(&bundle).expect("import 2");
        assert_eq!(s2, 0);
        assert_eq!(l2, 0);
    }
}
