//! The daily archive of cijene.dev: every chain's branches, products and prices in
//! one ZIP, in one shape. Used for chains without an adapter of their own (Tommy,
//! dm, KTC, NTL, Ribola, Metro…). Only the chain's files are fetched, by range.
//!
//! Each chain has `stores.csv`, `products.csv` and `prices.csv`; the last holds the
//! prices of all the chain's branches, so it is read as a stream and only the rows
//! of the one branch are kept.
use crate::retail::{PriceSource, RetailProduct, cents, name_size_basis, unit_basis, unit_cents};
use crate::retail_zip::{self as zip, Archive, ZipEntry};
use chrono::Utc;
use reqwest::{Client, Url};
use serde::Deserialize;
use std::collections::HashMap;
use std::io::Read;

pub(crate) const LIST_URL: &str = "https://api.cijene.dev/v0/list";
const HOST: &str = "api.cijene.dev";
/// Largest compressed `prices.csv` read; the biggest chains here are about 12 MB.
const MAX_PRICES: u64 = 25_000_000;

pub(crate) struct ArchiveIndex {
    pub url: String,
    pub date: String,
    size: u64,
    entries: HashMap<String, ZipEntry>,
}

impl ArchiveIndex {
    /// Chains that have a list of branches in this day's archive.
    pub fn chains(&self) -> Vec<String> {
        let mut chains: Vec<String> = self
            .entries
            .keys()
            .filter_map(|name| name.strip_suffix("/stores.csv"))
            .filter(|chain| !chain.is_empty() && !chain.contains('/'))
            .map(str::to_owned)
            .collect();
        chains.sort();
        chains
    }

    fn entry(&self, path: &str) -> Result<&ZipEntry, String> {
        self.entries
            .get(path)
            .ok_or_else(|| format!("U arhivi cijene.dev nema datoteke {path}"))
    }

    async fn locate(&self, client: &Client, path: &str) -> Result<(Archive, &ZipEntry), String> {
        let entry = self.entry(path)?;
        let whole = Archive {
            base: 0,
            size: self.size,
        };
        Ok((zip::locate(client, &self.url, whole, entry).await?, entry))
    }
}

#[derive(Deserialize)]
struct ArchiveList {
    archives: Vec<ArchiveItem>,
}
#[derive(Deserialize)]
struct ArchiveItem {
    date: String,
    url: String,
}

/// The newest archive in cijene.dev's list, only from its own host over HTTPS.
pub(crate) fn latest_archive(json: &[u8]) -> Result<(String, String), String> {
    let list: ArchiveList = serde_json::from_slice(json)
        .map_err(|e| format!("Promijenjen format popisa arhiva: {e}"))?;
    let today = Utc::now().date_naive().to_string();
    list.archives
        .into_iter()
        .filter(|a| a.date.len() == 10 && a.date <= today)
        .filter(|a| {
            Url::parse(&a.url).is_ok_and(|u| u.scheme() == "https" && u.host_str() == Some(HOST))
        })
        .max_by(|a, b| a.date.cmp(&b.date))
        .map(|a| (a.url, a.date))
        .ok_or_else(|| "cijene.dev trenutacno nema dnevne arhive".into())
}

pub(crate) async fn open_index(
    client: &Client,
    url: &str,
    date: &str,
) -> Result<ArchiveIndex, String> {
    let archive = zip::open(client, url).await?;
    let entries = zip::list(client, url, archive)
        .await?
        .into_iter()
        .map(|e| (e.name.clone(), e))
        .collect();
    Ok(ArchiveIndex {
        url: url.to_owned(),
        date: date.to_owned(),
        size: archive.size,
        entries,
    })
}

async fn read_small(client: &Client, index: &ArchiveIndex, path: &str) -> Result<Vec<u8>, String> {
    let (data, entry) = index.locate(client, path).await?;
    zip::read(client, &index.url, data, entry).await
}

/// The branch a source URL points at: `<archive>#<chain>/<store_id>`.
pub(crate) fn source_url(archive_url: &str, chain: &str, store_id: &str) -> Result<String, String> {
    let mut url = Url::parse(archive_url).map_err(|e| e.to_string())?;
    url.set_fragment(Some(&format!("{chain}/{store_id}")));
    Ok(url.to_string())
}

pub(crate) async fn stores(
    client: &Client,
    index: &ArchiveIndex,
    chain: &str,
) -> Result<Vec<(PriceSource, String)>, String> {
    let bytes = read_small(client, index, &format!("{chain}/stores.csv")).await?;
    parse_stores(&bytes, chain, &index.url, &index.date)
}

/// Branches with the store ID each one's prices are filed under.
pub(crate) fn parse_stores(
    bytes: &[u8],
    chain: &str,
    archive_url: &str,
    date: &str,
) -> Result<Vec<(PriceSource, String)>, String> {
    let mut reader = csv::Reader::from_reader(bytes);
    let headers = reader.headers().map_err(|e| e.to_string())?.clone();
    let col = |name: &str| headers.iter().position(|h| h == name);
    let id = col("store_id").ok_or("Promijenjen format popisa poslovnica")?;
    let (kind, address, city, zipcode) = (col("type"), col("address"), col("city"), col("zipcode"));
    let mut stores = Vec::new();
    for row in reader.records() {
        let row = row.map_err(|e| format!("Neispravan popis poslovnica: {e}"))?;
        let get = |i: Option<usize>| i.and_then(|i| row.get(i)).unwrap_or("").trim();
        let store_id = get(Some(id));
        if store_id.is_empty() {
            continue;
        }
        let postal = Some(get(zipcode))
            .filter(|z| z.len() == 5 && z.bytes().all(|b| b.is_ascii_digit()))
            .map(str::to_owned);
        // dm publishes one price list for every shop.
        let label = if store_id == "all" {
            "Sve poslovnice (iste cijene)".to_owned()
        } else {
            [get(kind), get(address), get(zipcode), get(city)]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        };
        stores.push((
            PriceSource {
                chain: chain.into(),
                label: if label.is_empty() {
                    store_id.to_owned()
                } else {
                    label
                },
                url: source_url(archive_url, chain, store_id)?,
                price_date: date.into(),
                branch_key: format!("{chain}:{store_id}"),
                postal_code: postal,
            },
            store_id.to_owned(),
        ));
    }
    if stores.is_empty() {
        return Err("Lanac u arhivi nema poslovnica".into());
    }
    Ok(stores)
}

pub(crate) async fn store_products(
    client: &Client,
    index: &ArchiveIndex,
    chain: &str,
    store_id: &str,
) -> Result<Vec<RetailProduct>, String> {
    let products = read_small(client, index, &format!("{chain}/products.csv")).await?;
    let (data, entry) = index.locate(client, &format!("{chain}/prices.csv")).await?;
    let raw = zip::read_raw(client, &index.url, data, MAX_PRICES).await?;
    parse_store_prices(&products, zip::stream(entry, &raw)?, store_id)
}

struct StorePrice {
    price: i64,
    unit_price: Option<i64>,
    special: Option<i64>,
}

/// Products with their price at one branch. Rows of other branches are skipped as
/// they stream past, and products the branch reports as unavailable are left out.
pub(crate) fn parse_store_prices(
    products_csv: &[u8],
    prices: impl Read,
    store_id: &str,
) -> Result<Vec<RetailProduct>, String> {
    let mut reader = csv::Reader::from_reader(prices);
    let headers = reader.headers().map_err(|e| e.to_string())?.clone();
    let col = |name: &str| headers.iter().position(|h| h == name);
    let (store, product, price) = (
        col("store_id").ok_or("Promijenjen format cijena")?,
        col("product_id").ok_or("Promijenjen format cijena")?,
        col("price").ok_or("Promijenjen format cijena")?,
    );
    let (unit_price, special, available) =
        (col("unit_price"), col("special_price"), col("available"));
    let mut found: HashMap<String, StorePrice> = HashMap::new();
    let mut record = csv::StringRecord::new();
    while reader
        .read_record(&mut record)
        .map_err(|e| format!("Neispravan cjenik u arhivi: {e}"))?
    {
        if record.get(store) != Some(store_id) {
            continue;
        }
        let get = |i: Option<usize>| i.and_then(|i| record.get(i)).unwrap_or("").trim();
        if get(available) == "0" {
            continue;
        }
        let Some(regular) = cents(get(Some(price))) else {
            continue;
        };
        found.insert(
            get(Some(product)).to_owned(),
            StorePrice {
                price: regular,
                unit_price: unit_cents(get(unit_price)),
                special: cents(get(special)).filter(|&s| s > 0 && s < regular),
            },
        );
        if found.len() > 100_000 {
            return Err("Cjenik ima previse proizvoda".into());
        }
    }
    let mut reader = csv::Reader::from_reader(products_csv);
    let headers = reader.headers().map_err(|e| e.to_string())?.clone();
    let col = |name: &str| headers.iter().position(|h| h == name);
    let id = col("product_id").ok_or("Promijenjen format proizvoda")?;
    let (barcode, name, brand, unit, quantity) = (
        col("barcode"),
        col("name"),
        col("brand"),
        col("unit"),
        col("quantity"),
    );
    let mut products = Vec::new();
    for row in reader.records() {
        let row = row.map_err(|e| format!("Neispravan popis proizvoda: {e}"))?;
        let get = |i: Option<usize>| i.and_then(|i| row.get(i)).unwrap_or("").trim();
        let Some(p) = found.get(get(Some(id))) else {
            continue;
        };
        let title = get(name);
        if title.is_empty() {
            continue;
        }
        let package = format!("{} {}", get(quantity), get(unit)).trim().to_owned();
        products.push(RetailProduct {
            name: title.to_owned(),
            // A code like "tommy:12345" stands in when there is no EAN.
            barcode: Some(get(barcode))
                .filter(|b| !b.is_empty() && b.bytes().all(|c| c.is_ascii_digit()))
                .map(str::to_owned),
            brand: get(brand).to_owned(),
            special_price_cents: p.special,
            unit_basis: p
                .unit_price
                .and_then(|_| name_size_basis(title).or_else(|| unit_basis(&package)))
                .map(str::to_owned),
            unit_price_cents: p.unit_price,
            price_cents: p.price,
            package,
        });
    }
    if products.is_empty() {
        return Err("Poslovnica u arhivi nema cijena".into());
    }
    Ok(products)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_archive_comes_only_from_cijene_dev() {
        let json = br#"{"archives":[
            {"date":"2026-11-18","url":"https://api.cijene.dev/v0/archive/2026-11-18.zip","size":1},
            {"date":"2026-11-19","url":"https://api.cijene.dev/v0/archive/2026-11-19.zip","size":1},
            {"date":"2026-11-19","url":"https://evil.test/2026-11-19.zip","size":1},
            {"date":"2999-01-01","url":"https://api.cijene.dev/v0/archive/2999-01-01.zip","size":1}]}"#;
        assert_eq!(
            latest_archive(json).unwrap(),
            (
                "https://api.cijene.dev/v0/archive/2026-11-19.zip".to_owned(),
                "2026-11-19".to_owned()
            )
        );
        assert!(latest_archive(br#"{"archives":[]}"#).is_err());
    }

    #[test]
    fn branches_and_one_branchs_prices_are_read_from_the_shared_format() {
        // Žabac and dm as published by cijene.dev on 19.11.2026.
        let stores = "store_id,type,address,city,zipcode\nPJ-7,Supermarket,Dubrava 256L,Zagreb,10000\nPJ-VG,Supermarket,Trg Grada Vukovara 8,Velika Gorica,10410\n";
        let parsed = parse_stores(
            stores.as_bytes(),
            "zabac",
            "https://api.cijene.dev/v0/archive/2026-11-19.zip",
            "2026-11-19",
        )
        .unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].0.label, "Supermarket Dubrava 256L 10000 Zagreb");
        assert_eq!(parsed[0].0.branch_key, "zabac:PJ-7");
        assert_eq!(parsed[0].0.postal_code.as_deref(), Some("10000"));
        assert_eq!(
            parsed[0].0.url,
            "https://api.cijene.dev/v0/archive/2026-11-19.zip#zabac/PJ-7"
        );
        assert_eq!(parsed[0].1, "PJ-7");
        let dm = parse_stores(
            b"store_id,type,address,city,zipcode\nall,store,,,\n",
            "dm",
            "https://api.cijene.dev/v0/archive/2026-11-19.zip",
            "2026-11-19",
        )
        .unwrap();
        assert_eq!(dm[0].0.label, "Sve poslovnice (iste cijene)");

        let products = "product_id,barcode,name,brand,category,unit,quantity\n000008,59969273,\"FANTA ORANGE 0,5L\",,Pića,,\n000022,zabac:000022,SPRITE 0.5L,,Pića,,\n000030,123,ČOKOLADA,Milka,,g,100\n";
        let prices = "store_id,product_id,price,unit_price,best_price_30,anchor_price,special_price,available,special_sale_type,anchor_price_date\n\
            PJ-7,000008,0.99,1.98,0.99,0.99,,,,2025-05-02\n\
            PJ-VG,000022,0.89,,,,,,,\n\
            PJ-7,000022,0.99,1.98,,,0.79,1,akcija,\n\
            PJ-7,000030,1.49,14.90,,,,0,,\n";
        let found = parse_store_prices(products.as_bytes(), prices.as_bytes(), "PJ-7").unwrap();
        assert_eq!(
            found.len(),
            2,
            "unavailable chocolate and the other branch are left out"
        );
        assert_eq!(found[0].name, "FANTA ORANGE 0,5L");
        assert_eq!(found[0].barcode.as_deref(), Some("59969273"));
        assert_eq!(
            (found[0].unit_price_cents, found[0].unit_basis.as_deref()),
            (Some(198), Some("L"))
        );
        assert_eq!(found[1].barcode, None);
        assert_eq!(
            (found[1].price_cents, found[1].special_price_cents),
            (99, Some(79))
        );
    }
}
