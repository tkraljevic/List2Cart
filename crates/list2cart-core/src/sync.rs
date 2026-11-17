use crate::models::{PocketBaseConfig, Product, Store, SyncReport};
use chrono::Utc;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct PbStoreRecord {
    sync_id: String,
    name: String,
    location: String,
    color: String,
    updated_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct PbProductRecord {
    sync_id: String,
    barcode: String,
    name: String,
    category: String,
    default_unit: String,
    last_price_cents: i64,
    last_store_name: String,
    image_data_url: Option<String>,
    updated_at: String,
}

#[derive(Debug, Deserialize)]
struct PbListResponse<T> {
    items: Vec<T>,
}

pub struct PulledStore {
    pub name: String,
    pub location: String,
    pub color: String,
}

/// Checks connectivity to a self-hosted PocketBase instance (e.g. on Unraid).
pub async fn check_pocketbase_health(base_url: &str) -> Result<bool, String> {
    let url = format!("{}/api/health", base_url.trim_end_matches('/'));
    let client = reqwest::Client::new();
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("PocketBase nije dostupan na {url}: {e}"))?;
    Ok(resp.status().is_success())
}

/// Performs two-way HTTP sync against PocketBase without holding any database locks across awaits.
pub async fn sync_with_pocketbase_http(
    stores: Vec<Store>,
    products: Vec<Product>,
    config: &PocketBaseConfig,
) -> Result<(SyncReport, Vec<PulledStore>), String> {
    let base = config.url.trim_end_matches('/');
    let client = reqwest::Client::new();

    let mut pushed_stores = 0usize;
    for store in &stores {
        let payload = PbStoreRecord {
            sync_id: store.sync_id.clone(),
            name: store.name.clone(),
            location: store.location.clone(),
            color: store.color.clone(),
            updated_at: store.updated_at.clone(),
        };
        let mut req = client
            .post(format!("{base}/api/collections/stores/records"))
            .json(&payload);
        if let Some(token) = &config.auth_token {
            req = req.bearer_auth(token);
        }
        if let Ok(r) = req.send().await
            && r.status().is_success()
        {
            pushed_stores += 1;
        }
    }

    let mut pushed_products = 0usize;
    for prod in &products {
        let payload = PbProductRecord {
            sync_id: prod.sync_id.clone(),
            barcode: prod.barcode.clone(),
            name: prod.name.clone(),
            category: prod.category.clone(),
            default_unit: prod.default_unit.as_str().to_owned(),
            last_price_cents: prod.last_price_cents,
            last_store_name: prod.last_store_name.clone(),
            image_data_url: prod.image_data_url.clone(),
            updated_at: prod.updated_at.clone(),
        };
        let mut req = client
            .post(format!("{base}/api/collections/products/records"))
            .json(&payload);
        if let Some(token) = &config.auth_token {
            req = req.bearer_auth(token);
        }
        if let Ok(r) = req.send().await
            && r.status().is_success()
        {
            pushed_products += 1;
        }
    }

    let mut pulled_stores = Vec::new();
    let mut store_req = client.get(format!("{base}/api/collections/stores/records?perPage=200"));
    if let Some(token) = &config.auth_token {
        store_req = store_req.bearer_auth(token);
    }
    if let Ok(resp) = store_req.send().await
        && resp.status().is_success()
        && let Ok(parsed) = resp.json::<PbListResponse<PbStoreRecord>>().await
    {
        for s in parsed.items {
            pulled_stores.push(PulledStore {
                name: s.name,
                location: s.location,
                color: s.color,
            });
        }
    }

    let report = SyncReport {
        pushed_stores,
        pushed_products,
        pushed_receipts: 0,
        pulled_stores: pulled_stores.len(),
        pulled_products: 0,
        synced_at: Utc::now().to_rfc3339(),
    };

    Ok((report, pulled_stores))
}
