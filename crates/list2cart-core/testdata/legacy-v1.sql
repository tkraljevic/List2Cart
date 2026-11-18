-- Synthetic pre-sync database. No external IDs for receipt items or price history.
PRAGMA foreign_keys = ON;
CREATE TABLE stores (
    id INTEGER PRIMARY KEY AUTOINCREMENT, sync_id TEXT NOT NULL UNIQUE,
    name TEXT NOT NULL UNIQUE, location TEXT NOT NULL DEFAULT '',
    color TEXT NOT NULL DEFAULT '#1768D1', updated_at TEXT NOT NULL
);
CREATE TABLE products (
    barcode TEXT PRIMARY KEY, sync_id TEXT NOT NULL UNIQUE, name TEXT NOT NULL,
    category TEXT NOT NULL DEFAULT 'Opce', default_unit TEXT NOT NULL DEFAULT 'kom',
    last_price_cents INTEGER NOT NULL DEFAULT 0, last_store_name TEXT NOT NULL DEFAULT '',
    image_data_url TEXT, updated_at TEXT NOT NULL
);
CREATE TABLE price_history (
    id INTEGER PRIMARY KEY AUTOINCREMENT, barcode TEXT NOT NULL, store_name TEXT NOT NULL,
    price_cents INTEGER NOT NULL, recorded_at TEXT NOT NULL,
    FOREIGN KEY (barcode) REFERENCES products(barcode) ON DELETE CASCADE
);
CREATE TABLE receipts (
    id INTEGER PRIMARY KEY AUTOINCREMENT, sync_id TEXT NOT NULL UNIQUE,
    store_id INTEGER NOT NULL, store_name TEXT NOT NULL, created_at TEXT NOT NULL,
    cart_total_cents INTEGER NOT NULL, receipt_total_cents INTEGER NOT NULL,
    difference_cents INTEGER NOT NULL, FOREIGN KEY (store_id) REFERENCES stores(id)
);
CREATE TABLE receipt_items (
    id INTEGER PRIMARY KEY AUTOINCREMENT, receipt_id INTEGER NOT NULL,
    barcode TEXT NOT NULL, name TEXT NOT NULL, quantity REAL NOT NULL, unit TEXT NOT NULL,
    unit_price_cents INTEGER NOT NULL, total_cents INTEGER NOT NULL,
    FOREIGN KEY (receipt_id) REFERENCES receipts(id) ON DELETE CASCADE
);
CREATE TABLE shopping_lists (
    id INTEGER PRIMARY KEY AUTOINCREMENT, sync_id TEXT NOT NULL UNIQUE, name TEXT NOT NULL,
    store_id INTEGER, updated_at TEXT NOT NULL,
    FOREIGN KEY (store_id) REFERENCES stores(id) ON DELETE SET NULL
);
CREATE TABLE planned_items (
    id INTEGER PRIMARY KEY AUTOINCREMENT, sync_id TEXT NOT NULL UNIQUE,
    list_id INTEGER NOT NULL, barcode TEXT, name TEXT NOT NULL, quantity REAL NOT NULL,
    unit TEXT NOT NULL, estimated_price_cents INTEGER NOT NULL DEFAULT 0,
    checked INTEGER NOT NULL DEFAULT 0, updated_at TEXT NOT NULL,
    FOREIGN KEY (list_id) REFERENCES shopping_lists(id) ON DELETE CASCADE
);
CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE record_versions (
    version_id TEXT PRIMARY KEY, entity TEXT NOT NULL, entity_id TEXT NOT NULL,
    node_id TEXT NOT NULL, supersedes TEXT NOT NULL DEFAULT '',
    archived INTEGER NOT NULL DEFAULT 0, payload TEXT NOT NULL, created_at TEXT NOT NULL,
    schema_version INTEGER NOT NULL DEFAULT 1, channel TEXT NOT NULL DEFAULT ''
);
INSERT INTO stores VALUES (7, 'legacy-store', 'Stara trgovina', 'Kvart', '#123456', '2025-01-01T10:00:00Z');
INSERT INTO products VALUES ('38501', 'legacy-product', 'Kruh', 'Pekara', 'kom', 150, 'Stara trgovina', NULL, '2025-01-01T10:00:00Z');
INSERT INTO price_history VALUES (12, '38501', 'Stara trgovina', 150, '2025-01-01T10:00:00Z');
INSERT INTO receipts VALUES (21, 'legacy-receipt', 7, 'Stara trgovina', '2025-01-01T10:00:00Z', 225, 230, -5);
INSERT INTO receipt_items VALUES (22, 21, '38501', 'Kruh', 1.5, 'kom', 150, 225);
INSERT INTO shopping_lists VALUES (31, 'legacy-list', 'Stari popis', 7, '2025-01-01T10:00:00Z');
INSERT INTO planned_items VALUES (32, 'legacy-planned', 31, '38501', 'Kruh', 1.5, 'kom', 150, 1, '2025-01-01T10:00:00Z');
INSERT INTO settings VALUES ('node_id', 'old-local-node');
INSERT INTO record_versions VALUES ('019424f8-0d00-7000-8000-000000000001', 'shopping_lists', 'legacy-list', 'old-claimed-author', '', 0, '{"name":"historical snapshot"}', '2025-01-01T10:00:00Z', 1, 'private:legacy');
