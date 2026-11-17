use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Unit {
    Kom,
    Kg,
    G,
}

impl Unit {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Kom => "kom",
            Self::Kg => "kg",
            Self::G => "g",
        }
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_lowercase().as_str() {
            "kom" => Ok(Self::Kom),
            "kg" => Ok(Self::Kg),
            "g" => Ok(Self::G),
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
    pub stores: Vec<Store>,
    pub products: Vec<Product>,
    pub lists: Vec<ShoppingList>,
    #[serde(default)]
    pub frontier: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub versions: Vec<crate::ledger::Version>,
}
