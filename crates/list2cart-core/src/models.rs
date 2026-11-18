use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Unit {
    Kom,
    Kg,
    G,
    L,
    Ml,
}

impl Unit {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Kom => "kom",
            Self::Kg => "kg",
            Self::G => "g",
            Self::L => "l",
            Self::Ml => "ml",
        }
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_lowercase().as_str() {
            "kom" => Ok(Self::Kom),
            "kg" => Ok(Self::Kg),
            "g" => Ok(Self::G),
            "l" => Ok(Self::L),
            "ml" => Ok(Self::Ml),
            other => Err(format!("Nepoznata mjerna jedinica: {other}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Store {
    pub id: i64,
    pub sync_id: String,
    pub name: String,
    pub location: String,
    pub color: String,
    pub item_count: i64,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Product {
    pub barcode: String,
    pub sync_id: String,
    pub name: String,
    pub category: String,
    pub default_unit: Unit,
    pub last_price_cents: i64,
    pub last_store_name: String,
    pub image_data_url: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorePrice {
    pub store_name: String,
    pub price_cents: i64,
    pub updated_at: String,
    pub is_cheapest: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CartItem {
    pub id: String,
    pub barcode: String,
    pub name: String,
    pub quantity: f64,
    pub unit: Unit,
    pub unit_price_cents: i64,
    pub total_cents: i64,
    pub image_data_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptVerification {
    pub cart_total_cents: i64,
    pub receipt_total_cents: i64,
    pub difference_cents: i64,
    pub is_matched: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptSummary {
    pub id: i64,
    pub sync_id: String,
    pub store_id: i64,
    pub store_name: String,
    pub created_at: String,
    pub items: Vec<CartItem>,
    pub cart_total_cents: i64,
    pub receipt_total_cents: i64,
    pub difference_cents: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedItem {
    pub id: i64,
    pub sync_id: String,
    pub list_id: i64,
    pub barcode: Option<String>,
    pub name: String,
    pub quantity: f64,
    pub unit: Unit,
    pub estimated_price_cents: i64,
    pub checked: bool,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShoppingList {
    pub id: i64,
    pub sync_id: String,
    pub name: String,
    pub store_id: Option<i64>,
    pub store_name: Option<String>,
    pub items: Vec<PlannedItem>,
    pub total_estimated_cents: i64,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnlineProductInfo {
    pub barcode: String,
    pub name: String,
    pub brand: Option<String>,
    pub image_url: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PocketBaseConfig {
    pub url: String,
    pub auth_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncReport {
    pub pushed_stores: usize,
    pub pushed_products: usize,
    pub pushed_receipts: usize,
    pub pulled_stores: usize,
    pub pulled_products: usize,
    pub synced_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncBundle {
    pub version: u32,
    pub exported_at: String,
    #[serde(default)]
    pub source_node_id: String,
    #[serde(default)]
    pub network_name: String,
    #[serde(default)]
    pub role: Option<crate::ledger::PeerRole>,
    #[serde(default)]
    pub sas_code: Option<String>,
    #[serde(default)]
    pub shared_lists: Option<String>,
    pub stores: Vec<Store>,
    pub products: Vec<Product>,
    #[serde(default)]
    pub receipts: Vec<ReceiptSummary>,
    pub lists: Vec<ShoppingList>,
    #[serde(default)]
    pub frontier: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub versions: Vec<crate::ledger::Version>,
}

pub fn qty_to_millis(qty: f64) -> Result<i64, String> {
    if !qty.is_finite() || qty <= 0.0 {
        return Err("Kolicina mora biti konacan pozitivan broj".to_owned());
    }
    let millis = (qty * 1000.0).round();
    if millis <= 0.0 || millis > 1_000_000_000_000.0 {
        return Err("Kolicina je izvan dopustenog raspona".to_owned());
    }
    Ok(millis as i64)
}

pub fn millis_to_qty(millis: i64) -> f64 {
    (millis as f64) / 1000.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreSyncPayload {
    pub sync_id: String,
    pub name: String,
    pub location: String,
    pub color: String,
    pub updated_at: String,
    #[serde(default)]
    pub archived: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductSyncPayload {
    pub sync_id: String,
    pub barcode: String,
    pub name: String,
    pub category: String,
    pub default_unit: Unit,
    pub last_price_cents: i64,
    pub last_store_name: String,
    pub image_data_url: Option<String>,
    pub updated_at: String,
    #[serde(default)]
    pub archived: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceHistorySyncPayload {
    pub sync_id: String,
    pub barcode: String,
    pub product_sync_id: String,
    pub store_sync_id: String,
    pub store_name: String,
    pub price_cents: i64,
    pub recorded_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub archived: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptSyncPayload {
    #[serde(default)]
    pub item_ids: Option<Vec<String>>,
    pub sync_id: String,
    pub store_sync_id: String,
    pub store_name: String,
    pub created_at: String,
    pub cart_total_cents: i64,
    pub receipt_total_cents: i64,
    pub difference_cents: i64,
    #[serde(default)]
    pub archived: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptItemSyncPayload {
    pub sync_id: String,
    pub receipt_sync_id: String,
    pub barcode: String,
    pub name: String,
    pub qty_millis: i64,
    pub unit: Unit,
    pub unit_price_cents: i64,
    pub total_cents: i64,
    pub updated_at: String,
    #[serde(default)]
    pub archived: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShoppingListSyncPayload {
    pub sync_id: String,
    pub name: String,
    pub store_sync_id: Option<String>,
    pub store_name: Option<String>,
    pub updated_at: String,
    #[serde(default)]
    pub archived: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedItemSyncPayload {
    pub sync_id: String,
    pub list_sync_id: String,
    pub barcode: Option<String>,
    pub name: String,
    pub qty_millis: i64,
    pub unit: Unit,
    pub estimated_price_cents: i64,
    pub checked: bool,
    pub updated_at: String,
    #[serde(default)]
    pub archived: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "control_kind", content = "data", rename_all = "snake_case")]
pub enum ControlPayload {
    Ovlast(list2cart_sync::trust::Ovlast),
    Membership(list2cart_sync::trust::Membership),
    ListGrant(list2cart_sync::trust::ListGrant),
    Revocation(list2cart_sync::trust::Revocation),
}
