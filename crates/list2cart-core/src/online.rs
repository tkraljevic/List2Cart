use crate::models::OnlineProductInfo;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct OpenFoodFactsResponse {
    status: i32,
    product: Option<OffProduct>,
}

#[derive(Debug, Deserialize)]
struct OffProduct {
    product_name: Option<String>,
    product_name_hr: Option<String>,
    brands: Option<String>,
    image_front_small_url: Option<String>,
    image_url: Option<String>,
}

/// Fetches product metadata (name, brand, thumbnail image) from the Open Food Facts v2 API.
pub async fn fetch_openfoodfacts_product(
    barcode: &str,
) -> Result<Option<OnlineProductInfo>, String> {
    let clean = barcode.trim();
    if clean.is_empty() {
        return Ok(None);
    }

    let url = format!(
        "https://world.openfoodfacts.net/api/v2/product/{clean}?fields=product_name,product_name_hr,brands,image_front_small_url,image_url"
    );

    let client = reqwest::Client::builder()
        .user_agent("List2Cart/0.1.0 (https://github.com/tkraljevic/List2Cart)")
        .build()
        .map_err(|e| format!("Greska HTTP klijenta: {e}"))?;

    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Greska spajanja na Open Food Facts: {e}"))?;

    if !resp.status().is_success() {
        return Ok(None);
    }

    let parsed: OpenFoodFactsResponse = resp
        .json()
        .await
        .map_err(|e| format!("Greska parsiranja Open Food Facts odgovora: {e}"))?;

    if parsed.status != 1 {
        return Ok(None);
    }

    let Some(prod) = parsed.product else {
        return Ok(None);
    };

    let name = prod
        .product_name_hr
        .filter(|s| !s.trim().is_empty())
        .or(prod.product_name)
        .unwrap_or_else(|| format!("Artikl {clean}"));

    let image_url = prod.image_front_small_url.or(prod.image_url);

    Ok(Some(OnlineProductInfo {
        barcode: clean.to_owned(),
        name,
        brand: prod.brands.filter(|s| !s.trim().is_empty()),
        image_url,
        source: "Open Food Facts".to_owned(),
    }))
}
