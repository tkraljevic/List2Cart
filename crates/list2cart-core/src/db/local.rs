//! Local changes and their signed snapshots share one SQLite transaction.
use super::*;
use crate::apply::{
    store_control_record, validate_payload_syntax_only, verify_domain_version_auth,
};
use crate::ledger::{Version, record_signed_version};
use crate::models::{ControlPayload, qty_to_millis};
use ed25519_dalek::{SigningKey, VerifyingKey};
use list2cart_sync::identity::{
    generate_signing_key, load_key, load_or_create_key, parse_public_key, principal_id_from_key,
    verifying_key_to_base64,
};
use list2cart_sync::trust::{MemberRole, NetworkKey, Validity};

const ID_NAMESPACE: Uuid = Uuid::from_u128(0x6c32632d_6c6f_4361_8c69_64656e746974);
const ENTITIES: [&str; 7] = [
    "stores",
    "products",
    "price_history",
    "receipts",
    "receipt_items",
    "shopping_lists",
    "planned_items",
];

#[derive(Debug, Clone, serde::Serialize)]
pub struct SyncIdentity {
    pub device_public_key: String,
    pub network_public_key: String,
    pub principal_id: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct IdentityConflict {
    pub entity: String,
    pub value: String,
    pub sync_ids: Vec<String>,
}

pub(super) struct LocalIdentity {
    key: SigningKey,
    network_public: VerifyingKey,
    network_key: Option<SigningKey>,
    principal: String,
}

fn setting(conn: &Connection, name: &str) -> Result<Option<String>, String> {
    let exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='settings')",
            [],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if !exists {
        return Ok(None);
    }
    conn.query_row("SELECT value FROM settings WHERE key=?1", [name], |r| {
        r.get(0)
    })
    .optional()
    .map_err(|e| e.to_string())
}

fn key_path(database: &Path, kind: &str) -> std::path::PathBuf {
    let mut name = database.as_os_str().to_os_string();
    name.push(format!(".{kind}.key"));
    name.into()
}

impl LocalIdentity {
    pub(super) fn ephemeral() -> Self {
        let root = generate_signing_key();
        Self {
            key: generate_signing_key(),
            network_public: root.verifying_key(),
            principal: principal_id_from_key(root.verifying_key().as_bytes()),
            network_key: Some(root),
        }
    }

    pub(super) fn load(conn: &Connection, path: &Path) -> Result<Self, String> {
        let device_path = key_path(path, "device");
        let bound = setting(conn, "device_key_fingerprint")?.is_some();
        let key = if bound {
            load_key(&device_path)
                .map_err(|e| e.to_string())?
                .ok_or("Nedostaje kljuc vezan uz bazu; potreban je oporavak identiteta")?
        } else {
            load_or_create_key(&device_path).map_err(|e| e.to_string())?
        };
        // Verify an existing binding before migrations or any data changes.
        if bound {
            crate::migrations::bind_or_verify_device_key_fingerprint(
                conn,
                Some(&verifying_key_to_base64(&key.verifying_key())),
            )?;
        }
        let root_path = key_path(path, "network");
        let (network_public, network_key) = match setting(conn, "sync_network_public")? {
            Some(public) => {
                let vk = parse_public_key(&public).map_err(|e| e.to_string())?;
                let root = load_key(&root_path).map_err(|e| e.to_string())?;
                if root.as_ref().is_some_and(|sk| sk.verifying_key() != vk) {
                    return Err("Kljuc mreze ne odgovara bazi".into());
                }
                (vk, root)
            }
            None => {
                let root = load_or_create_key(&root_path).map_err(|e| e.to_string())?;
                (root.verifying_key(), Some(root))
            }
        };
        let principal = setting(conn, "sync_principal")?
            .unwrap_or_else(|| principal_id_from_key(network_public.as_bytes()));
        if principal.is_empty() || principal.contains(['|', ':']) {
            return Err("Neispravan principal".into());
        }
        Ok(Self {
            key,
            network_public,
            network_key,
            principal,
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub enum LocalEntity {
    Store,
    Product,
    Price,
    Receipt,
    ReceiptItem,
    ShoppingList,
    PlannedItem,
}
impl LocalEntity {
    fn table(self) -> &'static str {
        match self {
            Self::Store => "stores",
            Self::Product => "products",
            Self::Price => "price_history",
            Self::Receipt => "receipts",
            Self::ReceiptItem => "receipt_items",
            Self::ShoppingList => "shopping_lists",
            Self::PlannedItem => "planned_items",
        }
    }
}

fn sql_error(e: rusqlite::Error) -> String {
    format!("Greska lokalne promjene: {e}")
}

fn stable_id(principal: &str, kind: &str, value: &str) -> String {
    // JSON tuple prevents ambiguous field boundaries in deterministic identities.
    let bytes = serde_json::to_vec(&(principal, kind, value)).expect("strings serialize");
    Uuid::new_v5(&ID_NAMESPACE, &bytes).to_string()
}

impl Database {
    /// Resolve an explicitly reviewed group without rewriting historical references.
    pub fn resolve_identity_conflict(
        &self,
        entity: &str,
        keep_id: &str,
        expected_ids: &[String],
    ) -> Result<(), String> {
        if !matches!(entity, "stores" | "products") {
            return Err("Nepodrzana vrsta sukoba".into());
        }
        let tx = self.conn.unchecked_transaction().map_err(sql_error)?;
        let expected: std::collections::BTreeSet<_> = expected_ids.iter().cloned().collect();
        let group = self
            .identity_conflicts()?
            .into_iter()
            .find(|g| g.entity == entity && g.sync_ids.iter().any(|id| id == keep_id))
            .ok_or("Sukob vise nije aktivan; osvjezite prikaz")?;
        if expected.len() != expected_ids.len()
            || expected.len() < 2
            || expected != group.sync_ids.iter().cloned().collect()
        {
            return Err("Sukob se promijenio; ponovno pregledajte zapise".into());
        }
        for id in &group.sync_ids {
            if id != keep_id {
                tx.execute(
                    &format!("UPDATE {entity} SET archived=1,updated_at=?1 WHERE sync_id=?2"),
                    params![Utc::now().to_rfc3339(), id],
                )
                .map_err(sql_error)?;
                self.record_snapshot(&tx, entity, id)?;
            }
        }
        tx.commit().map_err(sql_error)
    }

    /// Stable, arrival-order independent groups for an explicit merge chooser.
    pub fn identity_conflicts(&self) -> Result<Vec<IdentityConflict>, String> {
        let mut conflicts = Vec::new();
        for (entity, sql) in [
            (
                "stores",
                "SELECT LOWER(name),sync_id FROM stores WHERE archived=0 ORDER BY LOWER(name),sync_id",
            ),
            (
                "products",
                "SELECT barcode,sync_id FROM products WHERE archived=0 ORDER BY barcode,sync_id",
            ),
        ] {
            let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
            let mut stmt = self.conn.prepare(sql).map_err(sql_error)?;
            let rows = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .map_err(sql_error)?;
            for row in rows {
                let (value, id) = row.map_err(sql_error)?;
                groups.entry(value).or_default().push(id);
            }
            conflicts.extend(groups.into_iter().filter(|(_, ids)| ids.len() > 1).map(
                |(value, sync_ids)| IdentityConflict {
                    entity: entity.into(),
                    value,
                    sync_ids,
                },
            ));
        }
        Ok(conflicts)
    }

    pub fn sync_identity(&self) -> SyncIdentity {
        SyncIdentity {
            device_public_key: verifying_key_to_base64(&self.identity.key.verifying_key()),
            network_public_key: verifying_key_to_base64(&self.identity.network_public),
            principal_id: self.identity.principal.clone(),
        }
    }

    /// Encrypts a delta for one admitted recipient using this database's device key.
    pub fn export_protected_bundle(
        &self,
        recipient_public_key: &str,
        frontier: &BTreeMap<String, String>,
        holes: &[crate::ledger::HoleRecord],
        max_bytes: usize,
    ) -> Result<list2cart_sync::wire::bundle_v2::ProtectedBundleV2, String> {
        let recipient = parse_public_key(recipient_public_key).map_err(|e| e.to_string())?;
        crate::apply::export_protected_bundle_for_recipient(
            &self.conn,
            crate::apply::ExportRequest {
                local_node_id: &self.node_id()?,
                sender_sk: &self.identity.key,
                network_pub: &self.identity.network_public,
                recipient_pub: &recipient,
                peer_frontier: frontier,
                requested_holes: holes,
                max_bytes,
                now_unix: Utc::now().timestamp(),
            },
        )
    }

    /// Opens and admits a protected package without exposing private key material.
    pub fn import_protected_bundle(
        &self,
        bundle: &list2cart_sync::wire::bundle_v2::ProtectedBundleV2,
    ) -> Result<crate::apply::ApplyReport, String> {
        crate::apply::import_protected_bundle(
            &self.conn,
            &self.identity.key,
            &self.identity.network_public,
            &self.identity.principal,
            bundle,
            Utc::now().timestamp(),
        )
    }

    fn recovery_context(&self) -> crate::recovery::RecoveryContext<'_> {
        crate::recovery::RecoveryContext {
            conn: &self.conn,
            key: &self.identity.key,
            network: &self.identity.network_public,
            principal: &self.identity.principal,
            now: Utc::now().timestamp(),
        }
    }

    /// Signed, recipient-bound knowledge and a durable snapshot continuation.
    pub fn sync_request(&self, peer: &str) -> Result<crate::exchange::SyncRequest, String> {
        crate::exchange::request(self.recovery_context(), peer)
    }

    /// Chooses an ordinary delta or starts/resumes recovery without a UI decision.
    pub fn sync_reply(
        &self,
        request: &crate::exchange::SyncRequest,
        max_bytes: usize,
    ) -> Result<list2cart_sync::wire::bundle_v2::ProtectedBundleV2, String> {
        crate::exchange::reply(self.recovery_context(), request, max_bytes)
    }

    pub fn import_sync_page(
        &self,
        bundle: &list2cart_sync::wire::bundle_v2::ProtectedBundleV2,
    ) -> Result<crate::exchange::SyncReceiveOutcome, String> {
        crate::exchange::receive(self.recovery_context(), bundle)
    }

    pub fn start_snapshot_recovery(
        &self,
        recipient: &str,
    ) -> Result<crate::recovery::SnapshotManifest, String> {
        crate::recovery::start(self.recovery_context(), recipient)
    }

    /// Explicit durable, scoped transfer; automatic request/reply remains separate.
    pub fn start_family_snapshot_recovery(
        &self,
        recipient: &str,
    ) -> Result<crate::recovery::SnapshotManifest, String> {
        crate::recovery::start_family(self.recovery_context(), recipient)
    }

    pub fn export_snapshot_page(
        &self,
        snapshot_id: &str,
        recipient: &str,
        offset: usize,
        max_bytes: usize,
    ) -> Result<list2cart_sync::wire::bundle_v2::ProtectedBundleV2, String> {
        crate::recovery::export_page(
            self.recovery_context(),
            snapshot_id,
            recipient,
            offset,
            max_bytes,
        )
    }

    pub fn import_snapshot_page(
        &self,
        bundle: &list2cart_sync::wire::bundle_v2::ProtectedBundleV2,
    ) -> Result<crate::recovery::SnapshotProgress, String> {
        crate::recovery::import_page(self.recovery_context(), bundle)
    }

    pub fn snapshot_progress(
        &self,
        sender: &str,
        snapshot_id: &str,
    ) -> Result<Option<crate::recovery::SnapshotProgress>, String> {
        crate::recovery::progress(&self.conn, sender, snapshot_id)
    }

    /// Discards only derived transfer state; admitted domain records are retained.
    pub fn discard_snapshot_transfer(
        &self,
        snapshot_id: &str,
        peer: &str,
        outgoing: bool,
    ) -> Result<(), String> {
        crate::recovery::discard(&self.conn, snapshot_id, peer, outgoing)
    }

    pub(super) fn initialize_local_identity(&self) -> Result<(), String> {
        let tx = self.conn.unchecked_transaction().map_err(sql_error)?;
        let public = self.sync_identity();
        crate::migrations::bind_or_verify_device_key_fingerprint(
            &tx,
            Some(&public.device_public_key),
        )?;
        for (key, value) in [
            ("sync_network_public", &public.network_public_key),
            ("sync_principal", &public.principal_id),
            ("node_id", &public.device_public_key),
        ] {
            tx.execute("INSERT INTO settings(key,value) VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key,value]).map_err(sql_error)?;
        }
        let existing: Option<String> = tx.query_row("SELECT raw_json FROM sync_trust_records WHERE kind='membership' AND subject_key=?1", [&public.device_public_key], |r| r.get(0)).optional().map_err(sql_error)?;
        let now = Utc::now().timestamp();
        if let Some(json) = existing {
            let ControlPayload::Membership(m) = serde_json::from_str(&json)
                .map_err(|e| format!("Neispravno lokalno clanstvo: {e}"))?
            else {
                return Err("Neispravan tip clanstva".into());
            };
            m.verify(
                &self.identity.network_public,
                &self.identity.key.verifying_key(),
                now,
            )
            .map_err(|e| e.to_string())?;
            if m.principal_id != self.identity.principal {
                return Err("Clanstvo je vezano uz drugu osobu".into());
            }
        } else if let Some(root) = &self.identity.network_key {
            let network = NetworkKey::from_signing_key("List2Cart", root.clone());
            let m = network
                .admit_member(
                    &public.device_public_key,
                    &self.identity.key.verifying_key(),
                    MemberRole::OwnDevice,
                    &public.principal_id,
                    "root",
                    Validity {
                        issued_at: now,
                        valid_for_secs: 10 * 365 * 86400,
                    },
                )
                .map_err(|e| e.to_string())?;
            let ctrl = ControlPayload::Membership(m);
            store_control_record(&tx, &network.public, &ctrl, now)?;
            let payload = serde_json::to_string(&ctrl).map_err(|e| e.to_string())?;
            record_signed_version(
                &tx,
                &public.network_public_key,
                root,
                "membership",
                &format!("membership:{}", public.device_public_key),
                false,
                &payload,
                "control",
                "",
            )?;
        } else {
            return Err("Nedostaje potvrdeno clanstvo uredjaja".into());
        }
        tx.commit().map_err(sql_error)?;
        self.seed_default_stores()?;
        let tx = self.conn.unchecked_transaction().map_err(sql_error)?;
        if setting(&tx, "local_snapshot_bootstrap_v1")?.is_none() {
            self.capture_existing(&tx)?;
            tx.execute(
                "INSERT INTO settings(key,value) VALUES ('local_snapshot_bootstrap_v1','done')",
                [],
            )
            .map_err(sql_error)?;
        }
        // Only the original local author may attest the complete legacy item set.
        let old_receipts = {
            let mut stmt = tx.prepare("SELECT r.sync_id FROM receipts r JOIN record_versions v ON v.version_id=(SELECT MAX(version_id) FROM record_versions WHERE entity='receipts' AND entity_id=r.sync_id AND signature!='' AND legacy_imported=0) WHERE v.author_pub_key=?1 AND json_type(v.payload,'$.item_ids') IS NULL").map_err(sql_error)?;
            stmt.query_map([&public.device_public_key], |r| r.get::<_, String>(0))
                .map_err(sql_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(sql_error)?
        };
        for id in old_receipts {
            self.record_snapshot(&tx, "receipts", &id)?;
        }
        tx.commit().map_err(sql_error)
    }

    pub(super) fn local_seed_stores(&self) -> Result<(), String> {
        // A paired replica receives the catalog; pre-seeding can collide with legacy IDs.
        if self.identity.network_key.is_none() {
            return Ok(());
        }
        let tx = self.conn.unchecked_transaction().map_err(sql_error)?;
        if setting(&tx, "default_stores_seeded")?.is_some() {
            return Ok(());
        }
        let existing: i64 = tx
            .query_row("SELECT COUNT(*) FROM stores", [], |r| r.get(0))
            .map_err(sql_error)?;
        if existing > 0 {
            tx.execute("INSERT INTO settings(key,value) VALUES ('default_stores_seeded','existing catalog')",[]).map_err(sql_error)?;
            return tx.commit().map_err(sql_error);
        }
        for (name, location, color) in [
            ("Super Konzum", "Zagreb", "#DC2626"),
            ("Kaufland", "Zagreb", "#1768D1"),
            ("Lidl", "Zagreb", "#D97706"),
            ("Spar", "Zagreb", "#16A34A"),
            ("DM Drogerie Markt", "Centar", "#0284C7"),
        ] {
            let id = stable_id(&self.identity.principal, "default_store", name);
            let changed = tx.execute("INSERT INTO stores(sync_id,name,location,color,updated_at) VALUES (?1,?2,?3,?4,?5) ON CONFLICT(sync_id) DO NOTHING",
                params![id,name,location,color,Utc::now().to_rfc3339()]).map_err(sql_error)?;
            if changed > 0 {
                self.record_snapshot(&tx, "stores", &id)?;
            }
        }
        tx.execute(
            "INSERT INTO settings(key,value) VALUES ('default_stores_seeded','done')",
            [],
        )
        .map_err(sql_error)?;
        tx.commit().map_err(sql_error)
    }

    pub(super) fn local_add_store(
        &self,
        name: &str,
        location: &str,
        color: &str,
    ) -> Result<Store, String> {
        if name.trim().is_empty() {
            return Err("Naziv trgovine je prazan".into());
        }
        let id = Uuid::new_v4().to_string();
        let tx = self.conn.unchecked_transaction().map_err(sql_error)?;
        tx.execute(
            "INSERT INTO stores(sync_id,name,location,color,updated_at) VALUES (?1,?2,?3,?4,?5)",
            params![
                id,
                name.trim(),
                location.trim(),
                color.trim(),
                Utc::now().to_rfc3339()
            ],
        )
        .map_err(sql_error)?;
        self.record_snapshot(&tx, "stores", &id)?;
        tx.commit().map_err(sql_error)?;
        self.list_stores()?
            .into_iter()
            .find(|s| s.sync_id == id)
            .ok_or("Trgovina nije pronadjena nakon upisa".into())
    }

    fn write_product(&self, tx: &Connection, input: &UpsertProductInput<'_>) -> Result<(), String> {
        let matches: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM products WHERE barcode=?1 AND archived=0",
                [input.barcode.trim()],
                |r| r.get(0),
            )
            .map_err(sql_error)?;
        if matches > 1 {
            return Err("Vise artikala ima isti barkod; prvo razrijesite sukob identiteta".into());
        }
        if input.barcode.trim().is_empty()
            || input.name.trim().is_empty()
            || !(0..=crate::calc::MAX_SAFE_CENTS).contains(&input.price_cents)
        {
            return Err("Neispravan naziv, barkod ili cijena".into());
        }
        let existing: Option<(String, bool)> = tx
            .query_row(
                "SELECT sync_id,archived FROM products WHERE barcode=?1 ORDER BY archived,sync_id LIMIT 1",
                [input.barcode.trim()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(sql_error)?;
        if existing.as_ref().is_some_and(|(_, archived)| *archived) {
            return Err("Artikl je arhiviran; prvo ga izricito obnovite".into());
        }
        let id = existing.map(|(id, _)| id).unwrap_or_else(|| {
            stable_id(&self.identity.principal, "product", input.barcode.trim())
        });
        let now = Utc::now().to_rfc3339();
        tx.execute("INSERT INTO products(barcode,sync_id,name,category,default_unit,last_price_cents,last_store_name,image_data_url,updated_at)
              VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(sync_id) DO UPDATE SET name=excluded.name,category=excluded.category,
            default_unit=excluded.default_unit,last_price_cents=excluded.last_price_cents,last_store_name=excluded.last_store_name,
            image_data_url=COALESCE(excluded.image_data_url,products.image_data_url),updated_at=excluded.updated_at",
            params![input.barcode.trim(),id,input.name.trim(),input.category.trim(),input.default_unit.as_str(),input.price_cents,input.store_name.trim(),input.image_data_url,now]).map_err(sql_error)?;
        self.record_snapshot(tx, "products", &id)?;
        if input.price_cents > 0 && !input.store_name.trim().is_empty() {
            let price_id = Uuid::new_v4().to_string();
            tx.execute("INSERT INTO price_history(sync_id,barcode,store_name,price_cents,recorded_at,updated_at,product_sync_id,store_sync_id)
                VALUES (?1,?2,?3,?4,?5,?5,?6,COALESCE((SELECT sync_id FROM stores WHERE name=?3),''))",
                params![price_id,input.barcode.trim(),input.store_name.trim(),input.price_cents,now,id]).map_err(sql_error)?;
            self.record_snapshot(tx, "price_history", &price_id)?;
        }
        Ok(())
    }

    pub(super) fn local_upsert_product(
        &self,
        input: &UpsertProductInput<'_>,
    ) -> Result<Product, String> {
        let tx = self.conn.unchecked_transaction().map_err(sql_error)?;
        self.write_product(&tx, input)?;
        tx.commit().map_err(sql_error)?;
        self.get_product(input.barcode)?
            .ok_or("Artikl nije pronadjen nakon upisa".into())
    }

    pub(super) fn local_save_receipt(
        &self,
        store_id: i64,
        store_name: &str,
        items: &[CartItem],
        receipt_total: i64,
    ) -> Result<ReceiptSummary, String> {
        if !(0..=crate::calc::MAX_SAFE_CENTS).contains(&receipt_total) {
            return Err("Iznos racuna je izvan podrzanog raspona".into());
        }
        let tx = self.conn.unchecked_transaction().map_err(sql_error)?;
        let (store_sync, actual_name): (String, String) = tx
            .query_row(
                "SELECT sync_id,name FROM stores WHERE id=?1 AND archived=0",
                [store_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(sql_error)?;
        if actual_name != store_name.trim() {
            return Err("Naziv trgovine ne odgovara odabranom identitetu".into());
        }
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let mut normalized = Vec::with_capacity(items.len());
        let mut total = 0i64;
        for item in items {
            validate_quantity(item.quantity)?;
            if item.name.trim().is_empty() || item.unit_price_cents < 0 {
                return Err("Neispravna stavka racuna".into());
            }
            let line = crate::calc::checked_line_total_cents(
                item.quantity,
                item.unit,
                item.unit_price_cents,
            )?;
            total = total
                .checked_add(line)
                .ok_or("Zbroj racuna prelazi raspon")?;
            if total > crate::calc::MAX_SAFE_CENTS {
                return Err("Zbroj racuna prelazi podrzani raspon".into());
            }
            normalized.push(CartItem {
                total_cents: line,
                quantity: crate::models::millis_to_qty(qty_to_millis(item.quantity)?),
                ..item.clone()
            });
        }
        tx.execute("INSERT INTO receipts(sync_id,store_id,store_sync_id,store_name,created_at,cart_total_cents,receipt_total_cents,difference_cents)
            VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",params![id,store_id,store_sync,actual_name,now,total,receipt_total,total-receipt_total]).map_err(sql_error)?;
        let row_id = tx.last_insert_rowid();
        for item in &mut normalized {
            let item_id = Uuid::new_v4().to_string();
            tx.execute("INSERT INTO receipt_items(sync_id,receipt_id,receipt_sync_id,barcode,name,quantity,unit,unit_price_cents,total_cents,updated_at)
                VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![item_id,row_id,id,item.barcode.trim(),item.name.trim(),item.quantity,item.unit.as_str(),item.unit_price_cents,item.total_cents,now]).map_err(sql_error)?;
            item.id = tx.last_insert_rowid().to_string();
            self.record_snapshot(&tx, "receipt_items", &item_id)?;
            if !item.barcode.trim().is_empty() {
                self.write_product(
                    &tx,
                    &UpsertProductInput {
                        barcode: &item.barcode,
                        name: &item.name,
                        category: "Opce",
                        default_unit: item.unit,
                        price_cents: item.unit_price_cents,
                        store_name: &actual_name,
                        image_data_url: item.image_data_url.as_deref(),
                    },
                )?;
            }
        }
        self.record_snapshot(&tx, "receipts", &id)?;
        tx.commit().map_err(sql_error)?;
        Ok(ReceiptSummary {
            id: row_id,
            sync_id: id,
            store_id,
            store_name: actual_name,
            created_at: now,
            items: normalized,
            cart_total_cents: total,
            receipt_total_cents: receipt_total,
            difference_cents: total - receipt_total,
        })
    }

    pub(super) fn local_create_list(
        &self,
        name: &str,
        store_id: Option<i64>,
    ) -> Result<ShoppingList, String> {
        if name.trim().is_empty() {
            return Err("Naziv popisa je prazan".into());
        }
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let tx = self.conn.unchecked_transaction().map_err(sql_error)?;
        tx.execute("INSERT INTO shopping_lists(sync_id,name,store_id,store_sync_id,updated_at) VALUES (?1,?2,?3,(SELECT sync_id FROM stores WHERE id=?3 AND archived=0),?4)",params![id,name.trim(),store_id,now]).map_err(sql_error)?;
        if let Some(sid) = store_id {
            ensure_active(&tx, "stores", sid)?;
        }
        self.record_snapshot(&tx, "shopping_lists", &id)?;
        tx.commit().map_err(sql_error)?;
        self.list_shopping_lists()?
            .into_iter()
            .find(|l| l.sync_id == id)
            .ok_or("Popis nije pronadjen nakon upisa".into())
    }

    #[allow(clippy::too_many_arguments)] // Existing domain command: caller supplies a complete planned item.
    pub(super) fn local_add_item(
        &self,
        list_id: i64,
        barcode: Option<&str>,
        name: &str,
        quantity: f64,
        unit: Unit,
        price: i64,
    ) -> Result<PlannedItem, String> {
        validate_quantity(quantity)?;
        crate::calc::checked_line_total_cents(quantity, unit, price)?;
        if name.trim().is_empty() || price < 0 {
            return Err("Neispravna stavka popisa".into());
        }
        let quantity = crate::models::millis_to_qty(qty_to_millis(quantity)?);
        let tx = self.conn.unchecked_transaction().map_err(sql_error)?;
        ensure_active(&tx, "shopping_lists", list_id)?;
        let parent: String = tx
            .query_row(
                "SELECT sync_id FROM shopping_lists WHERE id=?1",
                [list_id],
                |r| r.get(0),
            )
            .map_err(sql_error)?;
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let barcode = barcode.map(str::trim).filter(|b| !b.is_empty());
        tx.execute("INSERT INTO planned_items(sync_id,list_id,list_sync_id,barcode,name,quantity,unit,estimated_price_cents,checked,updated_at)
            VALUES (?1,?2,?3,?4,?5,?6,?7,?8,0,?9)",params![id,list_id,parent,barcode,name.trim(),quantity,unit.as_str(),price,now]).map_err(sql_error)?;
        let row_id = tx.last_insert_rowid();
        self.touch_list(&tx, list_id, &now)?;
        self.record_snapshot(&tx, "planned_items", &id)?;
        tx.commit().map_err(sql_error)?;
        Ok(PlannedItem {
            id: row_id,
            sync_id: id,
            list_id,
            barcode: barcode.map(ToOwned::to_owned),
            name: name.trim().into(),
            quantity,
            unit,
            estimated_price_cents: price,
            checked: false,
            updated_at: now,
        })
    }

    pub(super) fn local_toggle_item(&self, item_id: i64, checked: bool) -> Result<(), String> {
        let tx = self.conn.unchecked_transaction().map_err(sql_error)?;
        ensure_active(&tx, "planned_items", item_id)?;
        let (id, list_id): (String, i64) = tx
            .query_row(
                "SELECT sync_id,list_id FROM planned_items WHERE id=?1",
                [item_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(sql_error)?;
        ensure_active(&tx, "shopping_lists", list_id)?;
        let now = Utc::now().to_rfc3339();
        tx.execute(
            "UPDATE planned_items SET checked=?1,updated_at=?2 WHERE id=?3",
            params![checked, now, item_id],
        )
        .map_err(sql_error)?;
        self.touch_list(&tx, list_id, &now)?;
        self.record_snapshot(&tx, "planned_items", &id)?;
        tx.commit().map_err(sql_error)
    }

    fn touch_list(&self, tx: &Connection, id: i64, now: &str) -> Result<(), String> {
        Self::checked_list_total(tx, id)?;
        tx.execute(
            "UPDATE shopping_lists SET updated_at=?1 WHERE id=?2",
            params![now, id],
        )
        .map_err(sql_error)?;
        let sync: String = tx
            .query_row(
                "SELECT sync_id FROM shopping_lists WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .map_err(sql_error)?;
        self.record_snapshot(tx, "shopping_lists", &sync)?;
        Ok(())
    }

    pub(super) fn checked_list_total(conn: &Connection, id: i64) -> Result<i64, String> {
        let mut stmt=conn.prepare("SELECT quantity,unit,estimated_price_cents FROM planned_items WHERE list_id=?1 AND archived=0").map_err(sql_error)?;
        let rows = stmt
            .query_map([id], |r| {
                Ok((
                    r.get::<_, f64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })
            .map_err(sql_error)?;
        let mut total = 0i64;
        for row in rows {
            let (q, u, p) = row.map_err(sql_error)?;
            let line = crate::calc::checked_line_total_cents(q, Unit::parse(&u)?, p)?;
            total = total
                .checked_add(line)
                .ok_or("Zbroj popisa prelazi dopusteni raspon")?;
            if total > crate::calc::MAX_SAFE_CENTS {
                return Err("Zbroj popisa prelazi podrzani raspon".into());
            }
        }
        Ok(total)
    }

    /// Explicit archive/restore: no physical DELETE, so offline peers retain tombstones.
    pub fn set_record_archived(
        &self,
        entity: LocalEntity,
        sync_id: &str,
        archived: bool,
    ) -> Result<(), String> {
        let table = entity.table();
        let tx = self.conn.unchecked_transaction().map_err(sql_error)?;
        let assignment = if matches!(entity, LocalEntity::Receipt) {
            "archived=?1"
        } else {
            "archived=?1,updated_at=?3"
        };
        let now = Utc::now().to_rfc3339();
        let sql = format!("UPDATE {table} SET {assignment} WHERE sync_id=?2");
        let n = if matches!(entity, LocalEntity::Receipt) {
            tx.execute(&sql, params![archived, sync_id])
        } else {
            tx.execute(&sql, params![archived, sync_id, now])
        }
        .map_err(sql_error)?;
        if n != 1 {
            return Err("Zapis za arhiviranje nije pronadjen".into());
        }
        self.record_snapshot(&tx, table, sync_id)?;
        if matches!(entity, LocalEntity::PlannedItem) {
            let parent = tx
                .query_row(
                    "SELECT list_id FROM planned_items WHERE sync_id=?1",
                    [sync_id],
                    |r| r.get(0),
                )
                .map_err(sql_error)?;
            self.touch_list(&tx, parent, &now)?;
        }
        if matches!(entity, LocalEntity::ReceiptItem) {
            let rid: i64 = tx
                .query_row(
                    "SELECT receipt_id FROM receipt_items WHERE sync_id=?1",
                    [sync_id],
                    |r| r.get(0),
                )
                .map_err(sql_error)?;
            let sum:i64=tx.query_row("SELECT COALESCE(SUM(total_cents),0) FROM receipt_items WHERE receipt_id=?1 AND archived=0",[rid],|r|r.get(0)).map_err(sql_error)?;
            tx.execute("UPDATE receipts SET cart_total_cents=?1,difference_cents=?1-receipt_total_cents WHERE id=?2",params![sum,rid]).map_err(sql_error)?;
            let parent: String = tx
                .query_row("SELECT sync_id FROM receipts WHERE id=?1", [rid], |r| {
                    r.get(0)
                })
                .map_err(sql_error)?;
            self.record_snapshot(&tx, "receipts", &parent)?;
        }
        tx.commit().map_err(sql_error)
    }

    fn record_snapshot(
        &self,
        conn: &Connection,
        entity: &str,
        id: &str,
    ) -> Result<Version, String> {
        let (payload, archived) = snapshot(conn, entity, id)?;
        let channel = if entity == "planned_items" {
            let parent:String=conn.query_row("SELECT sl.sync_id FROM planned_items pi JOIN shopping_lists sl ON sl.id=pi.list_id WHERE pi.sync_id=?1",[id],|r|r.get(0)).map_err(sql_error)?;
            self.channel_for(conn, "shopping_lists", &parent)?
        } else {
            self.channel_for(conn, entity, id)?
        };
        let grant = if let Some(list_id) = channel.strip_prefix("list:") {
            conn.query_row("SELECT record_id FROM sync_trust_records WHERE kind='list_grant' AND subject_key=?1 AND target_id=?2 AND permission='write' AND revoked=0
                AND issued_at<=?3 AND (expires_at=0 OR expires_at>=?3) ORDER BY issued_at DESC LIMIT 1",
                params![self.sync_identity().device_public_key,list_id,Utc::now().timestamp()],|r|r.get::<_,String>(0)).optional().map_err(sql_error)?.ok_or("Uredjaj nema write ovlast za popis")?
        } else {
            String::new()
        };
        let version = record_signed_version(
            conn,
            &self.sync_identity().device_public_key,
            &self.identity.key,
            entity,
            id,
            archived,
            &payload,
            &channel,
            &grant,
        )?;
        verify_domain_version_auth(
            conn,
            &self.identity.network_public,
            &version,
            Utc::now().timestamp(),
        )?;
        validate_payload_syntax_only(&version)?;
        Ok(version)
    }

    fn channel_for(&self, conn: &Connection, entity: &str, id: &str) -> Result<String, String> {
        let channel:Option<String>=conn.query_row("SELECT channel FROM record_versions WHERE entity=?1 AND entity_id=?2 AND signature!='' ORDER BY version_id DESC LIMIT 1",params![entity,id],|r|r.get(0)).optional().map_err(sql_error)?;
        Ok(channel.unwrap_or_else(|| format!("private:{}", self.identity.principal)))
    }

    /// Adopts legacy local snapshots as new local versions; never re-signs old author claims.
    pub(super) fn capture_existing(&self, conn: &Connection) -> Result<(), String> {
        for table in ["receipt_items", "price_history"] {
            let ids = string_rows(
                conn,
                &format!(
                    "SELECT CAST(id AS TEXT) FROM {table} WHERE sync_id IS NULL OR sync_id=''"
                ),
            )?;
            for id in ids {
                let stable = stable_id(&self.identity.principal, table, &id);
                conn.execute(
                    &format!("UPDATE {table} SET sync_id=?1 WHERE id=?2"),
                    params![stable, id],
                )
                .map_err(sql_error)?;
            }
        }
        for table in ENTITIES {
            for id in string_rows(conn, &format!("SELECT sync_id FROM {table}"))? {
                let (payload, archived) = snapshot(conn, table, &id)?;
                let latest:Option<(String,bool)>=conn.query_row("SELECT payload,archived FROM record_versions WHERE entity=?1 AND entity_id=?2 AND signature!='' ORDER BY version_id DESC LIMIT 1",params![table,id],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(sql_error)?;
                let unchanged = latest.as_ref().is_some_and(|(old, flag)| {
                    *flag == archived
                        && serde_json::from_str::<serde_json::Value>(old).ok()
                            == serde_json::from_str::<serde_json::Value>(&payload).ok()
                });
                if !unchanged {
                    self.record_snapshot(conn, table, &id)?;
                }
            }
        }
        Ok(())
    }
}

fn validate_quantity(quantity: f64) -> Result<(), String> {
    let millis = qty_to_millis(quantity)?;
    if (crate::models::millis_to_qty(millis) - quantity).abs() > 0.0000001 {
        return Err("Kolicina smije imati najvise tri decimale".into());
    }
    Ok(())
}

fn ensure_active(conn: &Connection, table: &str, id: i64) -> Result<(), String> {
    let exists: bool = conn
        .query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE id=?1 AND archived=0)"),
            [id],
            |r| r.get(0),
        )
        .map_err(sql_error)?;
    if !exists {
        return Err("Zapis ne postoji ili je arhiviran".into());
    }
    Ok(())
}

fn string_rows(conn: &Connection, sql: &str) -> Result<Vec<String>, String> {
    let mut stmt = conn.prepare(sql).map_err(sql_error)?;
    stmt.query_map([], |r| r.get(0))
        .map_err(sql_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(sql_error)
}

fn snapshot(conn: &Connection, entity: &str, id: &str) -> Result<(String, bool), String> {
    // SQLite JSON emits bools explicitly; integer millis and cents have a stable wire representation.
    let sql = match entity {
        "stores" => {
            "SELECT json_object('sync_id',sync_id,'name',name,'location',location,'color',color,'updated_at',updated_at,'archived',json(CASE archived WHEN 0 THEN 'false' ELSE 'true' END)),archived FROM stores WHERE sync_id=?1"
        }
        "products" => {
            "SELECT json_object('sync_id',sync_id,'barcode',barcode,'name',name,'category',category,'default_unit',default_unit,'last_price_cents',last_price_cents,'last_store_name',last_store_name,'image_data_url',image_data_url,'updated_at',updated_at,'archived',json(CASE archived WHEN 0 THEN 'false' ELSE 'true' END)),archived FROM products WHERE sync_id=?1"
        }
        "price_history" => {
            "SELECT json_object('sync_id',ph.sync_id,'barcode',ph.barcode,'product_sync_id',p.sync_id,'store_sync_id',COALESCE(s.sync_id,''),'store_name',ph.store_name,'price_cents',ph.price_cents,'recorded_at',ph.recorded_at,'updated_at',CASE ph.updated_at WHEN '' THEN ph.recorded_at ELSE ph.updated_at END,'archived',json(CASE ph.archived WHEN 0 THEN 'false' ELSE 'true' END)),ph.archived FROM price_history ph JOIN products p ON p.barcode=ph.barcode LEFT JOIN stores s ON s.name=ph.store_name WHERE ph.sync_id=?1"
        }
        "receipts" => {
            "SELECT json_object('sync_id',r.sync_id,'store_sync_id',s.sync_id,'store_name',r.store_name,'created_at',r.created_at,'cart_total_cents',r.cart_total_cents,'receipt_total_cents',r.receipt_total_cents,'difference_cents',r.difference_cents,'archived',json(CASE r.archived WHEN 0 THEN 'false' ELSE 'true' END)),r.archived FROM receipts r JOIN stores s ON s.id=r.store_id WHERE r.sync_id=?1"
        }
        "receipt_items" => {
            "SELECT json_object('sync_id',ri.sync_id,'receipt_sync_id',r.sync_id,'barcode',ri.barcode,'name',ri.name,'qty_millis',CAST(ROUND(ri.quantity*1000) AS INTEGER),'unit',ri.unit,'unit_price_cents',ri.unit_price_cents,'total_cents',ri.total_cents,'updated_at',CASE ri.updated_at WHEN '' THEN r.created_at ELSE ri.updated_at END,'archived',json(CASE ri.archived WHEN 0 THEN 'false' ELSE 'true' END)),ri.archived FROM receipt_items ri JOIN receipts r ON r.id=ri.receipt_id WHERE ri.sync_id=?1"
        }
        "shopping_lists" => {
            "SELECT json_object('sync_id',sl.sync_id,'name',sl.name,'store_sync_id',s.sync_id,'store_name',s.name,'updated_at',sl.updated_at,'archived',json(CASE sl.archived WHEN 0 THEN 'false' ELSE 'true' END)),sl.archived FROM shopping_lists sl LEFT JOIN stores s ON s.id=sl.store_id WHERE sl.sync_id=?1"
        }
        "planned_items" => {
            "SELECT json_object('sync_id',pi.sync_id,'list_sync_id',sl.sync_id,'barcode',pi.barcode,'name',pi.name,'qty_millis',CAST(ROUND(pi.quantity*1000) AS INTEGER),'unit',pi.unit,'estimated_price_cents',pi.estimated_price_cents,'checked',json(CASE pi.checked WHEN 0 THEN 'false' ELSE 'true' END),'updated_at',pi.updated_at,'archived',json(CASE pi.archived WHEN 0 THEN 'false' ELSE 'true' END)),pi.archived FROM planned_items pi JOIN shopping_lists sl ON sl.id=pi.list_id WHERE pi.sync_id=?1"
        }
        _ => return Err("Nepoznat entitet lokalnog zapisa".into()),
    };
    let (payload, archived): (String, bool) = conn
        .query_row(sql, [id], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(sql_error)?;
    if entity == "receipts" {
        let mut stmt = conn.prepare("SELECT sync_id FROM receipt_items WHERE receipt_id=(SELECT id FROM receipts WHERE sync_id=?1) AND archived=0 ORDER BY sync_id").map_err(sql_error)?;
        let ids = stmt
            .query_map([id], |r| r.get::<_, String>(0))
            .map_err(sql_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql_error)?;
        let mut payload: serde_json::Value =
            serde_json::from_str(&payload).map_err(|e| e.to_string())?;
        payload["item_ids"] = serde_json::to_value(ids).map_err(|e| e.to_string())?;
        return Ok((payload.to_string(), archived));
    }
    Ok((payload, archived))
}

#[cfg(test)]
mod tests;
