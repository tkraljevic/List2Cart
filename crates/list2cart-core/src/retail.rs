//! Public retailer price lists, fetched directly and searched on the device.
use chrono::{Datelike, NaiveDate, Utc};
use reqwest::{Client, Url};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceSource {
    pub chain: String,
    pub label: String,
    pub url: String,
    pub price_date: String,
    /// The branch without the date of its price list; stays the same from day to day.
    #[serde(default)]
    pub branch_key: String,
    /// Croatian postal code found in the label, when the retailer puts one there.
    #[serde(default)]
    pub postal_code: Option<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct RetailProduct {
    pub name: String,
    pub barcode: Option<String>,
    pub brand: String,
    pub package: String,
    pub price_cents: i64,
    /// Price during a promotion, only when it is lower than `price_cents`.
    pub special_price_cents: Option<i64>,
    /// Price per kilogram, litre or piece, as published by the retailer.
    pub unit_price_cents: Option<i64>,
    /// "kg", "L" or "kom"; what `unit_price_cents` is measured in.
    pub unit_basis: Option<String>,
}
#[derive(Serialize)]
pub struct RetailResults {
    pub source: PriceSource,
    pub fetched_at: String,
    pub products: Vec<RetailProduct>,
    pub total_matches: usize,
}
/// A branch the user picked once; resolved to that day's price list on every search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HomeBranch {
    pub chain: String,
    pub branch_key: String,
    pub label: String,
}
#[derive(Serialize)]
pub struct BranchSearch {
    pub chain: String,
    pub branch_label: String,
    pub results: Option<RetailResults>,
    pub error: Option<String>,
}
#[derive(Serialize)]
pub struct ChainSources {
    pub chain: String,
    pub sources: Vec<PriceSource>,
    pub error: Option<String>,
}
/// Chains read straight from their own websites (`RetailSearch::list_sources`).
/// Every other chain comes from the cijene.dev daily archive: those are fresher
/// here, and Konzum's and Studenac's prices there are 40 MB a day each.
pub const CHAINS: [&str; 7] = [
    "konzum", "lidl", "kaufland", "spar", "plodine", "eurospin", "studenac",
];
const INDEX_TTL: Duration = Duration::from_secs(2 * 3600);
const PRICE_LIST_TTL: Duration = Duration::from_secs(1800);
/// One list per saved branch must fit, so a search of all saved branches reuses them.
const PRICE_LIST_CACHE_MAX: usize = 16;
struct Cached {
    at: Instant,
    fetched_at: String,
    products: Vec<RetailProduct>,
}
/// A branch's file inside a chain's daily ZIP (Plodine, Eurospin, Studenac).
#[derive(Debug, Clone)]
struct ZipSource {
    chain: String,
    zip_url: String,
    /// The archive that holds the entry: the whole ZIP or a ZIP stored inside it.
    container: crate::retail_zip::Archive,
    entry: crate::retail_zip::ZipEntry,
}
#[derive(Default)]
pub struct RetailSearch {
    sources: Mutex<HashMap<String, PriceSource>>,
    indexes: Mutex<HashMap<String, (Instant, Vec<PriceSource>)>>,
    cache: Mutex<HashMap<String, Cached>>,
    /// Source URL (ZIP URL plus `#entry`) to where that file lies in the ZIP.
    zip_sources: Mutex<HashMap<String, ZipSource>>,
    /// The cijene.dev archive's directory, so each chain needs only its own files.
    archive: Mutex<Option<(Instant, Arc<crate::retail_archive::ArchiveIndex>)>>,
    /// Source URL (archive URL plus `#chain/store`) to the chain and store ID.
    archive_stores: Mutex<HashMap<String, (String, String)>>,
}

fn client() -> Result<Client, String> {
    Client::builder()
        .user_agent("List2Cart/0.0.1 public-price-list-reader")
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())
}
async fn download(client: &Client, url: &str, limit: usize) -> Result<Vec<u8>, String> {
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Dohvat cjenika nije uspio: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Trgovina nije vratila cjenik: {e}"))?;
    if response.content_length().is_some_and(|n| n > limit as u64) {
        return Err("Cjenik je prevelik".into());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if bytes.len() + chunk.len() > limit {
            return Err("Cjenik je prevelik".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
fn text(bytes: &[u8]) -> Result<String, String> {
    if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        let encoding = if bytes[0] == 0xff {
            encoding_rs::UTF_16LE
        } else {
            encoding_rs::UTF_16BE
        };
        let (s, _, bad) = encoding.decode(bytes);
        return if bad {
            Err("Neispravno UTF-16 kodiranje cjenika".into())
        } else {
            Ok(s.into_owned())
        };
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return Ok(s.trim_start_matches('\u{feff}').to_owned());
    }
    let (s, _, bad) = encoding_rs::WINDOWS_1250.decode(bytes);
    if bad {
        return Err("Nepodrzano kodiranje cjenika".into());
    }
    Ok(s.into_owned())
}
fn normalized(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            'č' | 'ć' => 'c',
            'š' => 's',
            'ž' => 'z',
            'đ' => 'd',
            _ => c,
        })
        .filter(|c| c.is_alphanumeric())
        .collect()
}
#[cfg(test)]
fn price_date(label: &str) -> Option<NaiveDate> {
    price_date_at(label).map(|(date, _)| date)
}
/// The price list date and the byte offset where it starts in the label.
fn price_date_at(label: &str) -> Option<(NaiveDate, usize)> {
    for (i, _) in label.char_indices() {
        // Studenac writes 2026-11-19, Plodine's daily ZIP 19_11_2026.
        for (length, formats) in [
            (10, &["%d.%m.%Y", "%Y-%m-%d", "%d_%m_%Y"][..]),
            (8, &["%Y%m%d", "%d%m%Y"][..]),
        ] {
            if let Some(piece) = label.get(i..i + length) {
                for format in formats {
                    if let Ok(date) = NaiveDate::parse_from_str(piece, format)
                        && (2025..=Utc::now().year() + 1).contains(&date.year())
                    {
                        return Some((date, i));
                    }
                }
            }
        }
    }
    None
}
/// Words of the label before the date, without long numbers (postal codes and daily
/// file numbers), so the same branch gets the same key in every day's price list.
fn branch_key(label: &str, date_at: usize) -> String {
    label[..date_at]
        .split(|c: char| !c.is_alphanumeric())
        .map(normalized)
        .filter(|t| !t.is_empty() && !(t.len() >= 5 && t.bytes().all(|b| b.is_ascii_digit())))
        .collect::<Vec<_>>()
        .join("-")
}
fn postal_code(label: &str) -> Option<String> {
    label
        .split(|c: char| !c.is_alphanumeric())
        .find(|t| {
            t.len() == 5
                && t.bytes().all(|b| b.is_ascii_digit())
                && t.parse::<u32>()
                    .is_ok_and(|n| (10_000..=53_999).contains(&n))
        })
        .map(str::to_owned)
}
fn source(chain: &str, base: &str, href: &str, label: &str) -> Option<PriceSource> {
    let url = Url::parse(base).ok()?.join(href).ok()?;
    let host = Url::parse(base).ok()?;
    if url.scheme() != "https" || url.host_str() != host.host_str() {
        return None;
    }
    let decoded = url
        .query_pairs()
        .find(|(key, _)| key == "title")
        .map(|(_, value)| value.into_owned());
    let label = decoded.unwrap_or_else(|| label.split_whitespace().collect::<Vec<_>>().join(" "));
    let (date, date_at) = price_date_at(&label)?;
    Some(PriceSource {
        chain: chain.into(),
        branch_key: branch_key(&label, date_at),
        postal_code: postal_code(&label[..date_at]),
        label: label.replace('_', " "),
        url: url.to_string(),
        price_date: date.to_string(),
    })
}
fn html_sources(chain: &str, base: &str, html: &str) -> Vec<PriceSource> {
    let document = Html::parse_document(html);
    let selector = Selector::parse("a[href]").expect("constant selector");
    document
        .select(&selector)
        .filter_map(|a| {
            let href = a.value().attr("href")?;
            if !href.to_lowercase().contains(".csv") && !href.contains("/cjenici/download?") {
                return None;
            }
            source(chain, base, href, &a.text().collect::<String>())
        })
        .collect()
}
/// The newest daily ZIP linked from a chain's price list page, on the chain's own host.
/// Eurospin offers its archives in a drop-down list rather than as links.
fn latest_zip(base: &str, html: &str) -> Option<String> {
    let host = Url::parse(base).ok()?;
    let document = Html::parse_document(html);
    let selector = Selector::parse("a[href], option[value]").expect("constant selector");
    document
        .select(&selector)
        .filter_map(|a| {
            let link = a.value().attr("href").or_else(|| a.value().attr("value"))?;
            let url = host.join(link).ok()?;
            if url.scheme() != "https"
                || url.host_str() != host.host_str()
                || !url.path().to_lowercase().ends_with(".zip")
            {
                return None;
            }
            let file = url.path_segments()?.next_back()?.to_owned();
            let (date, _) = price_date_at(&file)?;
            Some((date, file, url.to_string()))
        })
        .filter(|(date, _, _)| *date <= Utc::now().date_naive())
        .max()
        .map(|(_, _, url)| url)
}
impl RetailSearch {
    pub async fn list_sources(&self, chain: &str) -> Result<Vec<PriceSource>, String> {
        let client = client()?;
        let mut sources = match chain {
            "spar" => {
                let date = Utc::now().format("%Y%m%d");
                let url = format!("https://www.spar.hr/datoteke_cjenici/Cjenik{date}.json");
                let data: serde_json::Value =
                    serde_json::from_slice(&download(&client, &url, 2_000_000).await?)
                        .map_err(|e| e.to_string())?;
                data["files"]
                    .as_array()
                    .ok_or("Promijenjen format SPAR indeksa")?
                    .iter()
                    .filter_map(|v| {
                        source(
                            chain,
                            "https://www.spar.hr",
                            v["URL"].as_str()?,
                            v["name"].as_str()?,
                        )
                    })
                    .collect()
            }
            "kaufland" => {
                let url = "https://www.kaufland.hr/akcije-novosti/popis-mpc.assetSearch.id=assetList_1599847924.json";
                let data: Vec<serde_json::Value> =
                    serde_json::from_slice(&download(&client, url, 3_000_000).await?)
                        .map_err(|e| e.to_string())?;
                data.iter()
                    .filter_map(|v| {
                        source(
                            chain,
                            "https://www.kaufland.hr",
                            v["path"].as_str()?,
                            v["label"].as_str()?,
                        )
                    })
                    .collect()
            }
            "lidl" => {
                let html = text(
                    &download(&client, "https://www.lidl.hr/c/cijene/s10073252", 3_000_000).await?,
                )?;
                html_sources(chain, "https://www.lidl.hr", &html)
            }
            "konzum" => {
                let mut found = Vec::new();
                for page in 1..=30 {
                    let url = format!("https://www.konzum.hr/cjenici?page={page}");
                    let html = text(&download(&client, &url, 2_000_000).await?)?;
                    let next = Html::parse_document(&html)
                        .select(&Selector::parse("a[rel=next]").expect("constant selector"))
                        .next()
                        .is_some();
                    found.extend(html_sources(chain, "https://www.konzum.hr", &html));
                    if !next {
                        break;
                    }
                    if page == 30 {
                        return Err("Indeks poslovnica je prevelik; nije potpuno ucitan".into());
                    }
                }
                found
            }
            "plodine" => {
                self.zip_sources_for(&client, chain, "https://www.plodine.hr/info-o-cijenama")
                    .await?
            }
            "eurospin" => {
                self.zip_sources_for(&client, chain, "https://www.eurospin.hr/cjenik/")
                    .await?
            }
            "studenac" => {
                self.zip_sources_for(
                    &client,
                    chain,
                    "https://www.studenac.hr/popis-maloprodajnih-cijena",
                )
                .await?
            }
            other => self.archive_sources(&client, other).await?,
        };
        let today = Utc::now().date_naive().to_string();
        sources.retain(|s| s.price_date <= today);
        let latest = sources
            .iter()
            .map(|s| s.price_date.clone())
            .max()
            .ok_or("Trgovina trenutačno nema citljivih cjenika")?;
        sources.retain(|s| s.price_date == latest);
        let unique: BTreeMap<_, _> = sources.into_iter().map(|s| (s.url.clone(), s)).collect();
        let mut result: Vec<_> = unique.into_values().collect();
        result.sort_by(|a, b| a.label.cmp(&b.label));
        {
            let mut registry = self.sources.lock().map_err(|_| "Greska registra cjenika")?;
            registry.retain(|_, s| s.chain != chain);
            for s in &result {
                registry.insert(s.url.clone(), s.clone());
            }
        }
        self.indexes
            .lock()
            .map_err(|_| "Greska registra cjenika")?
            .insert(chain.to_owned(), (Instant::now(), result.clone()));
        Ok(result)
    }

    /// Branches of a chain that publishes one ZIP a day with a file per branch.
    /// Only the archive's directory is fetched here; a branch's file is read when
    /// it is searched. A ZIP stored inside the ZIP (Studenac) is opened in place.
    async fn zip_sources_for(
        &self,
        client: &Client,
        chain: &str,
        page_url: &str,
    ) -> Result<Vec<PriceSource>, String> {
        use crate::retail_zip as zip;
        let html = text(&download(client, page_url, 3_000_000).await?)?;
        let zip_url =
            latest_zip(page_url, &html).ok_or("Na stranici trgovine nema dnevne arhive cjenika")?;
        let outer = zip::open(client, &zip_url).await?;
        let is_list = |name: &str| {
            let lower = name.to_lowercase();
            lower.ends_with(".csv") || lower.ends_with(".xml")
        };
        let mut files = Vec::new();
        for entry in zip::list(client, &zip_url, outer).await? {
            if entry.name.to_lowercase().ends_with(".zip") && entry.method == 0 {
                let inner = zip::locate(client, &zip_url, outer, &entry).await?;
                for file in zip::list(client, &zip_url, inner).await? {
                    if is_list(&file.name) {
                        files.push((format!("{}/{}", entry.name, file.name), inner, file));
                    }
                }
            } else if is_list(&entry.name) {
                files.push((entry.name.clone(), outer, entry));
            }
        }
        let mut sources = Vec::new();
        let mut located = Vec::new();
        for (path, container, entry) in files {
            let file = path.rsplit('/').next().unwrap_or(&path).to_owned();
            let Some((date, date_at)) = price_date_at(&file) else {
                continue;
            };
            let mut url = Url::parse(&zip_url).map_err(|e| e.to_string())?;
            url.set_fragment(Some(&path));
            let url = url.to_string();
            located.push((
                url.clone(),
                ZipSource {
                    chain: chain.to_owned(),
                    zip_url: zip_url.clone(),
                    container,
                    entry,
                },
            ));
            sources.push(PriceSource {
                chain: chain.into(),
                branch_key: branch_key(&file, date_at),
                postal_code: postal_code(&file[..date_at]),
                label: file.replace('_', " "),
                url,
                price_date: date.to_string(),
            });
        }
        let mut registry = self
            .zip_sources
            .lock()
            .map_err(|_| "Greska registra cjenika")?;
        registry.retain(|_, z| z.chain != chain);
        registry.extend(located);
        Ok(sources)
    }

    /// Like `list_sources`, but reuses an index loaded in the last two hours.
    /// Konzum's index alone takes many page loads.
    async fn sources_cached(&self, chain: &str) -> Result<Vec<PriceSource>, String> {
        let cached = self
            .indexes
            .lock()
            .map_err(|_| "Greska registra cjenika")?
            .get(chain)
            .filter(|(at, _)| at.elapsed() < INDEX_TTL)
            .map(|(_, sources)| sources.clone());
        match cached {
            Some(sources) => Ok(sources),
            None => self.list_sources(chain).await,
        }
    }

    /// Branches of every chain at once, for choosing them by place.
    /// One chain failing does not hide the others.
    pub async fn all_sources(&self) -> Vec<ChainSources> {
        let mut chains: Vec<String> = CHAINS.iter().map(|c| (*c).to_owned()).collect();
        let mut archive_error = None;
        match async { self.archive_index(&client()?).await }.await {
            Ok(index) => chains.extend(
                index
                    .chains()
                    .into_iter()
                    .filter(|c| !CHAINS.contains(&c.as_str())),
            ),
            Err(error) => archive_error = Some(error),
        }
        let load = |chain: String| async move {
            match self.sources_cached(&chain).await {
                Ok(sources) => ChainSources {
                    chain,
                    sources,
                    error: None,
                },
                Err(error) => ChainSources {
                    chain,
                    sources: vec![],
                    error: Some(error),
                },
            }
        };
        let mut all = futures_util::future::join_all(chains.into_iter().map(load)).await;
        if let Some(error) = archive_error {
            all.push(ChainSources {
                chain: "cijene.dev".into(),
                sources: vec![],
                error: Some(format!("Ostali lanci nisu ucitani: {error}")),
            });
        }
        all
    }

    /// Today's cijene.dev archive directory, kept for two hours.
    async fn archive_index(
        &self,
        client: &Client,
    ) -> Result<Arc<crate::retail_archive::ArchiveIndex>, String> {
        let cached = self
            .archive
            .lock()
            .map_err(|_| "Greska registra cjenika")?
            .as_ref()
            .filter(|(at, _)| at.elapsed() < INDEX_TTL)
            .map(|(_, index)| index.clone());
        if let Some(index) = cached {
            return Ok(index);
        }
        let list = download(client, crate::retail_archive::LIST_URL, 1_000_000).await?;
        let (url, date) = crate::retail_archive::latest_archive(&list)?;
        let index = Arc::new(crate::retail_archive::open_index(client, &url, &date).await?);
        *self.archive.lock().map_err(|_| "Greska registra cjenika")? =
            Some((Instant::now(), index.clone()));
        Ok(index)
    }

    /// Branches of a chain without an adapter of its own, from the cijene.dev archive.
    async fn archive_sources(
        &self,
        client: &Client,
        chain: &str,
    ) -> Result<Vec<PriceSource>, String> {
        let index = self.archive_index(client).await?;
        if !index.chains().iter().any(|c| c == chain) {
            return Err(format!("Nepodrzan trgovacki lanac: {chain}"));
        }
        let stores = crate::retail_archive::stores(client, &index, chain).await?;
        let mut registry = self
            .archive_stores
            .lock()
            .map_err(|_| "Greska registra cjenika")?;
        registry.retain(|_, (c, _)| c != chain);
        Ok(stores
            .into_iter()
            .map(|(source, store_id)| {
                registry.insert(source.url.clone(), (chain.to_owned(), store_id));
                source
            })
            .collect())
    }

    /// The registered source for a URL from the last loaded index.
    pub fn registered_source(&self, source_url: &str) -> Result<PriceSource, String> {
        self.sources
            .lock()
            .map_err(|_| "Greska registra")?
            .get(source_url)
            .cloned()
            .ok_or_else(|| "Ponovno ucitajte i odaberite poslovnicu".into())
    }

    /// Searches today's price list of each saved branch at the same time.
    pub async fn search_branches(&self, branches: &[HomeBranch], query: &str) -> Vec<BranchSearch> {
        futures_util::future::join_all(branches.iter().map(|branch| async move {
            let outcome = async {
                let sources = self.sources_cached(&branch.chain).await?;
                let source = sources
                    .iter()
                    .filter(|s| s.branch_key == branch.branch_key)
                    .max_by(|a, b| a.label.cmp(&b.label))
                    .ok_or("Za ovu poslovnicu nema današnjeg cjenika")?;
                self.search(&source.url, query).await
            }
            .await;
            match outcome {
                Ok(results) => BranchSearch {
                    chain: branch.chain.clone(),
                    branch_label: branch.label.clone(),
                    results: Some(results),
                    error: None,
                },
                Err(error) => BranchSearch {
                    chain: branch.chain.clone(),
                    branch_label: branch.label.clone(),
                    results: None,
                    error: Some(error),
                },
            }
        }))
        .await
    }

    pub async fn search(&self, source_url: &str, query: &str) -> Result<RetailResults, String> {
        let tokens: Vec<_> = query
            .split_whitespace()
            .map(normalized)
            .filter(|s| !s.is_empty())
            .collect();
        if !(2..=100).contains(&query.trim().chars().count()) || tokens.is_empty() {
            return Err("Upisite 2 do 100 znakova naziva proizvoda".into());
        }
        let source = self
            .sources
            .lock()
            .map_err(|_| "Greska registra")?
            .get(source_url)
            .cloned()
            .ok_or("Ponovno ucitajte i odaberite poslovnicu")?;
        {
            let cache = self.cache.lock().map_err(|_| "Greska predmemorije")?;
            if let Some(c) = cache
                .get(source_url)
                .filter(|c| c.at.elapsed() < PRICE_LIST_TTL)
            {
                return Ok(results(&source, c, &tokens));
            }
        }
        let client = client()?;
        let archived = self
            .archive_stores
            .lock()
            .map_err(|_| "Greska registra")?
            .get(source_url)
            .cloned();
        let products = if let Some((chain, store_id)) = archived {
            let index = self.archive_index(&client).await?;
            let archive_url = source_url.split('#').next().unwrap_or(source_url);
            // A branch chosen from yesterday's archive is still read from that archive.
            let index = if index.url == archive_url {
                index
            } else {
                Arc::new(
                    crate::retail_archive::open_index(&client, archive_url, &source.price_date)
                        .await?,
                )
            };
            crate::retail_archive::store_products(&client, &index, &chain, &store_id).await?
        } else {
            let zipped = self
                .zip_sources
                .lock()
                .map_err(|_| "Greska registra")?
                .get(source_url)
                .cloned();
            let bytes = match zipped {
                Some(z) => {
                    let data =
                        crate::retail_zip::locate(&client, &z.zip_url, z.container, &z.entry)
                            .await?;
                    crate::retail_zip::read(&client, &z.zip_url, data, &z.entry).await?
                }
                None => download(&client, source_url, 12_000_000).await?,
            };
            if looks_like_xml(&bytes) {
                parse_xml(&bytes)?
            } else {
                parse_csv(&bytes)?
            }
        };
        let cached = Cached {
            at: Instant::now(),
            fetched_at: Utc::now().to_rfc3339(),
            products,
        };
        let result = results(&source, &cached, &tokens);
        let mut cache = self.cache.lock().map_err(|_| "Greska predmemorije")?;
        cache.retain(|_, c| c.at.elapsed() < PRICE_LIST_TTL);
        // Searching every saved branch needs one list per chain at once; drop only the oldest.
        while cache.len() >= PRICE_LIST_CACHE_MAX {
            let Some(oldest) = cache
                .iter()
                .min_by_key(|(_, c)| c.at)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            cache.remove(&oldest);
        }
        cache.insert(source_url.into(), cached);
        Ok(result)
    }
}
fn results(source: &PriceSource, cached: &Cached, tokens: &[String]) -> RetailResults {
    let matched: Vec<_> = cached
        .products
        .iter()
        .filter(|p| {
            let hay = normalized(&format!("{} {}", p.name, p.brand));
            tokens.iter().all(|t| hay.contains(t))
        })
        .collect();
    RetailResults {
        source: source.clone(),
        fetched_at: cached.fetched_at.clone(),
        total_matches: matched.len(),
        products: matched.into_iter().take(100).cloned().collect(),
    }
}
pub(crate) fn cents(value: &str) -> Option<i64> {
    let value = value.trim().trim_end_matches('€').trim().replace(',', ".");
    let mut parts = value.split('.');
    let whole = parts.next()?;
    let fraction = parts.next().unwrap_or("");
    // Plodine writes prices under one euro without the zero: ",75".
    if (whole.is_empty() && fraction.is_empty()) || !whole.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if parts.next().is_some() || fraction.len() > 2 || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let frac = match fraction.len() {
        0 => 0,
        1 => fraction.parse::<i64>().ok()? * 10,
        _ => fraction.parse().ok()?,
    };
    let whole = if whole.is_empty() { "0" } else { whole };
    let amount = whole
        .parse::<i64>()
        .ok()?
        .checked_mul(100)?
        .checked_add(frac)?;
    (amount <= crate::calc::MAX_SAFE_CENTS).then_some(amount)
}
/// Price per unit of measure. Some retailers publish it with three or four
/// decimals (Kaufland: "1,124"); it is rounded to whole cents, half up.
pub(crate) fn unit_cents(value: &str) -> Option<i64> {
    let value = value.trim().trim_end_matches('€').trim().replace(',', ".");
    let (whole, fraction) = value.split_once('.').unwrap_or((&value, ""));
    let whole = if whole.is_empty() && !fraction.is_empty() {
        "0"
    } else {
        whole
    };
    if whole.is_empty()
        || fraction.len() > 4
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let digits = format!("{fraction:0<4}");
    let ten_thousandths = whole
        .parse::<i64>()
        .ok()?
        .checked_mul(10_000)?
        .checked_add(digits.parse::<i64>().ok()?)?;
    let amount = (ten_thousandths + 50) / 100;
    (amount > 0 && amount <= crate::calc::MAX_SAFE_CENTS).then_some(amount)
}
/// What a unit price is measured in, read from the package or unit-of-measure text:
/// "0.20 kg", "g (1kg)", "1,5 l", "KOM".
pub(crate) fn unit_basis(text: &str) -> Option<&'static str> {
    let words: Vec<String> = text
        .split(|c: char| !c.is_alphabetic())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect();
    let has = |names: &[&str]| words.iter().any(|w| names.contains(&w.as_str()));
    if has(&["kg", "g", "gr", "dag"]) {
        Some("kg")
    } else if has(&["l", "ml", "cl", "dl", "lit"]) {
        Some("L")
    } else if has(&["kom", "ko", "kos", "pak"]) {
        Some("kom")
    } else {
        None
    }
}
/// Weight or volume written with a number in a product name: "120 g", "1,5L",
/// "500ml". A bare letter ("JAJA L") is a size, not litres.
pub(crate) fn name_size_basis(name: &str) -> Option<&'static str> {
    let lower = name.to_lowercase();
    let chars: Vec<char> = lower.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_ascii_digit() {
            i += 1;
            continue;
        }
        while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == ',' || chars[i] == '.') {
            i += 1;
        }
        while i < chars.len() && chars[i] == ' ' {
            i += 1;
        }
        let unit: String = chars[i..]
            .iter()
            .take_while(|c| c.is_alphabetic())
            .collect();
        match unit.as_str() {
            "kg" | "g" | "gr" | "dag" => return Some("kg"),
            "l" | "ml" | "cl" | "dl" => return Some("L"),
            _ => {}
        }
    }
    None
}
fn looks_like_xml(bytes: &[u8]) -> bool {
    let start = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    start.iter().find(|b| !b.is_ascii_whitespace()) == Some(&b'<')
}
fn xml_unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let Some(end) = tail.find(';').filter(|&e| e <= 10) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let decoded = match &tail[1..end] {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            n if n.starts_with("#x") => u32::from_str_radix(&n[2..], 16)
                .ok()
                .and_then(char::from_u32),
            n if n.starts_with('#') => n[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &tail[end + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}
/// Text of `<tag>…</tag>` in a product block; an empty `<tag/>` gives None.
fn xml_field(block: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let start = block.find(&open)? + open.len();
    let end = block[start..].find(&format!("</{tag}>"))? + start;
    let value = xml_unescape(block[start..end].trim());
    (!value.is_empty()).then_some(value)
}
/// Studenac's XML price list: `<Proizvod>` elements with the fields the price
/// decree names. A product on promotion may have only the promotion price.
pub fn parse_xml(bytes: &[u8]) -> Result<Vec<RetailProduct>, String> {
    if bytes.len() > 12_000_000 {
        return Err("Cjenik je prevelik (najviše 12 MB)".into());
    }
    let data = text(bytes)?;
    let mut products = Vec::new();
    let mut rest = data.as_str();
    while let Some(start) = rest.find("<Proizvod>") {
        let after = &rest[start + "<Proizvod>".len()..];
        let end = after
            .find("</Proizvod>")
            .ok_or("Neispravan XML cjenik: proizvod nije zatvoren")?;
        let block = &after[..end];
        rest = &after[end + "</Proizvod>".len()..];
        let field = |tag: &str| xml_field(block, tag);
        let regular = field("MaloprodajnaCijena").as_deref().and_then(cents);
        let promotion = field("MaloprodajnaCijenaAkcija").as_deref().and_then(cents);
        let Some(price_cents) = regular.or(promotion) else {
            continue;
        };
        let Some(name) = field("NazivProizvoda") else {
            continue;
        };
        let package = format!(
            "{} {}",
            field("NetoKolicina").unwrap_or_default(),
            field("JedinicaMjere").unwrap_or_default()
        )
        .trim()
        .to_owned();
        let unit_price_cents = field("CijenaZaJedinicuMjere")
            .as_deref()
            .and_then(unit_cents);
        // Studenac gives "kom" as the unit even for "… 120 g", so a size in the name wins.
        let basis = name_size_basis(&name).or_else(|| unit_basis(&package));
        products.push(RetailProduct {
            barcode: field("Barkod").filter(|b| b.bytes().all(|c| c.is_ascii_digit())),
            brand: field("MarkaProizvoda").unwrap_or_default(),
            special_price_cents: promotion.filter(|&p| regular.is_some_and(|r| p > 0 && p < r)),
            unit_basis: unit_price_cents.and(basis).map(str::to_owned),
            unit_price_cents,
            package,
            price_cents,
            name,
        });
        if products.len() > 100_000 {
            return Err("Cjenik ima previse proizvoda".into());
        }
    }
    if products.is_empty() {
        return Err("Cjenik ne sadrzi citljive proizvode i cijene".into());
    }
    Ok(products)
}
pub fn parse_csv(bytes: &[u8]) -> Result<Vec<RetailProduct>, String> {
    let inspection = inspect_csv(bytes)?;
    let mapping = inspection
        .suggested
        .ok_or("Nepoznati ili dvosmisleni stupci: odaberite stupce naziva i maloprodajne cijene")?;
    parse_csv_mapped(bytes, &mapping)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CsvMapping {
    pub delimiter: u8,
    pub name: usize,
    pub price: usize,
    pub barcode: Option<usize>,
    pub brand: Option<usize>,
    pub quantity: Option<usize>,
    pub unit: Option<usize>,
    #[serde(default)]
    pub unit_price: Option<usize>,
    /// Kaufland names the unit of its unit price in a column of its own.
    #[serde(default)]
    pub unit_price_basis: Option<usize>,
    #[serde(default)]
    pub special_price: Option<usize>,
}

#[derive(Serialize)]
pub struct CsvInspection {
    pub headers: Vec<String>,
    pub sample: Vec<Vec<String>>,
    pub delimiter: u8,
    pub suggested: Option<CsvMapping>,
}

pub fn inspect_csv(bytes: &[u8]) -> Result<CsvInspection, String> {
    if bytes.len() > 12_000_000 {
        return Err("Cjenik je prevelik (najviše 12 MB)".into());
    }
    let data = text(bytes)?;
    // Let the CSV reader handle quoted delimiters and multiline fields. A candidate
    // must have consistent field counts in the preview, not just many separators.
    let delimiter = b";\t,|"
        .iter()
        .copied()
        .filter_map(|delimiter| {
            let mut reader = csv::ReaderBuilder::new()
                .delimiter(delimiter)
                .from_reader(data.as_bytes());
            let width = reader.headers().ok()?.len();
            if width < 2 || reader.records().take(20).any(|r| r.is_err()) {
                return None;
            }
            Some((width, delimiter))
        })
        .max()
        .map(|(_, d)| d)
        .ok_or("Nije prepoznata tablica s barem dva stupca")?;
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .from_reader(data.as_bytes());
    let headers: Vec<String> = reader
        .headers()
        .map_err(|e| e.to_string())?
        .iter()
        .map(str::to_owned)
        .collect();
    let normalized_headers: Vec<_> = headers.iter().map(|h| normalized(h)).collect();
    let col = |aliases: &[&str]| {
        let matches: Vec<_> = normalized_headers
            .iter()
            .enumerate()
            .filter(|(_, h)| aliases.contains(&h.as_str()))
            .map(|(i, _)| i)
            .collect();
        if matches.len() == 1 {
            Some(matches[0])
        } else {
            None
        }
    };
    let suggested = col(&[
        "naziv",
        "nazivproizvoda",
        "nazivartikla",
        "proizvod",
        "artikl",
        "name",
        "productname",
    ])
    .zip(col(&[
        "maloprodajnacijena",
        "mpceur",
        "maloprodcijenaeur",
        "maloprodajnacijenaeur",
        "mpc",
        "cijena",
        "cijenaeur",
        "price",
        "priceeur",
        "retailprice",
    ]))
    .map(|(name, price)| CsvMapping {
        delimiter,
        name,
        price,
        barcode: col(&["barkod", "barcode", "ean", "ean13", "gtin"]),
        brand: col(&["marka", "markaproizvoda", "brand"]),
        quantity: col(&["netokolicina", "netokolicinakg"]),
        unit: col(&["jedinicamjere"]),
        // Konzum, Spar, Lidl (with its spelling "JEDINICIN") and Kaufland.
        unit_price: col(&[
            "cijenazajedinicumjere",
            "cijenazajedinicumjereeur",
            "cijenazajedinicinmjere",
            "cijenajedmjeur",
            // Plodine.
            "cijenapojm",
        ]),
        unit_price_basis: col(&["jedmj1komlkg"]),
        special_price: col(&[
            "mpczavrijemeposebnogoblikaprodaje",
            "mpczavrijemeposebnogoblikaprodajeeur",
            "mpcposeboblikprod",
        ]),
    });
    let sample = reader
        .records()
        .take(5)
        .map(|row| {
            row.map(|r| r.iter().map(str::to_owned).collect())
                .map_err(|e| e.to_string())
        })
        .collect::<Result<_, _>>()?;
    Ok(CsvInspection {
        headers,
        sample,
        delimiter,
        suggested,
    })
}

pub fn parse_csv_mapped(bytes: &[u8], mapping: &CsvMapping) -> Result<Vec<RetailProduct>, String> {
    if bytes.len() > 12_000_000 {
        return Err("Cjenik je prevelik (najviše 12 MB)".into());
    }
    if !b";\t,|".contains(&mapping.delimiter) {
        return Err("Nepodržani razdjelnik".into());
    }
    let data = text(bytes)?;
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(mapping.delimiter)
        .from_reader(data.as_bytes());
    let width = reader.headers().map_err(|e| e.to_string())?.len();
    let CsvMapping {
        name,
        price,
        barcode,
        brand,
        quantity,
        unit,
        unit_price,
        unit_price_basis,
        special_price,
        ..
    } = *mapping;
    if name == price
        || [
            Some(name),
            Some(price),
            barcode,
            brand,
            quantity,
            unit,
            unit_price,
            unit_price_basis,
            special_price,
        ]
        .into_iter()
        .flatten()
        .any(|i| i >= width)
    {
        return Err("Neispravan odabir stupaca".into());
    }
    let mut products = Vec::new();
    for (index, row) in reader.records().enumerate() {
        if index >= 100_000 {
            return Err("Cjenik ima previše redaka".into());
        }
        let row = row.map_err(|e| format!("Neispravan redak cjenika: {e}"))?;
        let Some(price_cents) = row.get(price).and_then(cents) else {
            continue;
        };
        let name = row.get(name).unwrap_or("").trim();
        if name.is_empty() {
            continue;
        }
        let barcode = barcode
            .and_then(|i| row.get(i))
            .map(str::trim)
            .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            .map(str::to_owned);
        let package: String = format!(
            "{} {}",
            quantity.and_then(|i| row.get(i)).unwrap_or(""),
            unit.and_then(|i| row.get(i)).unwrap_or("")
        )
        .trim()
        .into();
        let unit_price_cents = unit_price.and_then(|i| row.get(i)).and_then(unit_cents);
        let basis_text = unit_price_basis
            .and_then(|i| row.get(i))
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(&package);
        products.push(RetailProduct {
            name: name.into(),
            barcode,
            brand: brand.and_then(|i| row.get(i)).unwrap_or("").trim().into(),
            special_price_cents: special_price
                .and_then(|i| row.get(i))
                .and_then(cents)
                .filter(|&special| special > 0 && special < price_cents),
            unit_basis: unit_price_cents
                .and_then(|_| unit_basis(basis_text))
                .map(str::to_owned),
            unit_price_cents,
            package,
            price_cents,
        });
        if products.len() > 100_000 {
            return Err("Cjenik ima previse proizvoda".into());
        }
    }
    if products.is_empty() {
        return Err("Cjenik ne sadrzi citljive proizvode i cijene".into());
    }
    Ok(products)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_retailer_schemas_parse_integer_cents_without_barcode_requirement() {
        for csv in [
            "NAZIV,ŠIFRA,NETO_KOLICINA,JEDINICA_MJERE,MARKA,MALOPRODAJNA_CIJENA,BARKOD\nMlijeko,1,1,kom,Test,1.19,3850000000001\n",
            "NAZIV PROIZVODA,MARKA PROIZVODA,NETO KOLIČINA,JEDINICA MJERE,MALOPRODAJNA CIJENA,BARKOD\nMlijeko,Test,1 L,ko,1.19,\n",
            "naziv;marka;neto količina;jedinica mjere;MPC (EUR);barkod\nMlijeko;Test;1,000;L;1,19;3850000000001\n",
            "naziv proizvoda\tmarka proizvoda\tneto količina(KG)\tjedinica mjere\tmaloprod.cijena(EUR)\tbarkod\nMlijeko\tTest\t1\tKOM\t 1,19\t3850000000001\n",
        ] {
            let products = parse_csv(csv.as_bytes()).unwrap();
            assert_eq!(products.len(), 1);
            assert_eq!(products[0].price_cents, 119);
        }
        assert_eq!(cents("1,005"), None);
        assert_eq!(cents("-1,00"), None);
        assert_eq!(cents("9.223.372"), None);
    }

    #[test]
    fn official_sources_and_dates_are_validated_and_query_normalizes_croatian() {
        assert_eq!(
            price_date("SPAR_20261117_0330.csv").unwrap().to_string(),
            "2026-11-17"
        );
        assert_eq!(
            price_date("Supermarket_1630_17112026_7-30.csv")
                .unwrap()
                .to_string(),
            "2026-11-17"
        );
        assert_eq!(
            price_date("Lidl_17.11.2026_7.15h.csv").unwrap().to_string(),
            "2026-11-17"
        );
        assert!(
            source(
                "lidl",
                "https://www.lidl.hr",
                "https://example.org/17.11.2026.csv",
                "17.11.2026"
            )
            .is_none()
        );
        assert!(
            source(
                "lidl",
                "https://www.lidl.hr",
                "http://www.lidl.hr/17.11.2026.csv",
                "17.11.2026"
            )
            .is_none()
        );
        assert_eq!(normalized("Čokolada ŠEĆER"), "cokoladasecer");
        let html = "<a href='/prices/17.11.2026.csv'>Lidl_17.11.2026.csv</a><a href='https://evil.test/17.11.2026.csv'>17.11.2026</a>";
        assert_eq!(html_sources("lidl", "https://www.lidl.hr", html).len(), 1);
    }

    #[test]
    fn unit_and_promotion_prices_follow_each_retailers_header() {
        // Header lines as published on 19.11.2026, one product row each.
        let konzum = "NAZIV PROIZVODA,ŠIFRA PROIZVODA,MARKA PROIZVODA,NETO KOLIČINA,JEDINICA MJERE,MALOPRODAJNA CIJENA,POSEBAN OBLIK PRODAJE,NAZIV POSEBNOG OBLIKA PRODAJE,CIJENA ZA JEDINICU MJERE,BARKOD,BAZNA CIJENA\nČIPI ČIPS 200g,01310113,ČIPI,0.20 kg,ko,2.99,NE,,14.95,3856021300547,2.99\n";
        let spar = "naziv;šifra;marka;neto količina;jedinica mjere;MPC (EUR);cijena za jedinicu mjere (EUR);MPC za vrijeme posebnog oblika prodaje (EUR);Naziv posebnog oblika prodaje;Najniža cijena u posljednjih 30 dana (EUR);sidrena cijena (EUR);barkod;kategorija proizvoda\nNJOKI 1 kg;1010;;1,0000;kg;2,99;2,99;2,49;Akcija;;2,99;8000506014107;Ostalo\n";
        let lidl = "NAZIV,ŠIFRA,NETO_KOLICINA,JEDINICA_MJERE,MARKA,MALOPRODAJNA_CIJENA,MPC_ZA_VRIJEME_POSEBNOG_OBLIKA_PRODAJE,NAJNIZA_CIJENA_U_POSLJ_30_DANA,CIJENA_ZA_JEDINICIN_MJERE,BARKOD,SIDRENA_CIJENA\nVoda,4,1.5,ml (1 L),Saguaro,0.45,,,0.30,4304493256583,x\n";
        let kaufland = "naziv proizvoda\tšifra proizvoda\tmarka proizvoda\tneto količina(KG)\tjedinica mjere\tmaloprod.cijena(EUR)\takc.cijena, A=akcija\tkol.jed.mj.\tjed.mj. (1 KOM/L/KG)\tcijena jed.mj.(EUR)\tMPC poseb.oblik prod\tNajniža MPC u 30dana\tSidrena cijena\tbarkod\tkategorija proizvoda\tDostupan/Nedostupan\nKrpa 10kom\t00010016\tVileda\t0.300\tKOM\t       8,99\t\t1\tKOM\t1,124\t\t\tx\t3838447000195\tx\tdostupno\n";
        let first = |csv: &str| parse_csv(csv.as_bytes()).unwrap().remove(0);
        let p = first(konzum);
        assert_eq!(
            (p.unit_price_cents, p.unit_basis.as_deref()),
            (Some(1495), Some("kg"))
        );
        assert_eq!(p.special_price_cents, None);
        let p = first(spar);
        assert_eq!(
            (p.unit_price_cents, p.unit_basis.as_deref()),
            (Some(299), Some("kg"))
        );
        assert_eq!(p.special_price_cents, Some(249));
        let p = first(lidl);
        assert_eq!(
            (p.unit_price_cents, p.unit_basis.as_deref()),
            (Some(30), Some("L"))
        );
        let p = first(kaufland);
        assert_eq!(
            (p.unit_price_cents, p.unit_basis.as_deref()),
            (Some(112), Some("kom"))
        );
        assert_eq!(p.price_cents, 899);

        assert_eq!(unit_cents("1,125"), Some(113));
        assert_eq!(unit_cents("24.88"), Some(2488));
        assert_eq!(unit_cents("0"), None);
        assert_eq!(unit_cents("1,12345"), None);
        assert_eq!(unit_basis("g (1kg)"), Some("kg"));
        assert_eq!(unit_basis("1 kom"), Some("kom"));
    }

    #[test]
    fn plodine_eurospin_and_studenac_lists_parse() {
        // Plodine, 18.11.2026: prices under one euro have no leading zero.
        let plodine = "Naziv proizvoda;Sifra proizvoda;Marka proizvoda;Neto kolicina;Jedinica mjere;Cijena po JM;MPC;Poseban oblik prodaje;Naziv posebnog oblika prodaje;Sidrena cijena;Barkod;Dostupno nedostupno;\nVODA JAMNICA 1 L BOCA;010102;JAMNICA;1 L;KOM;0,75;,75;NE;;,69;3859888152014;DOSTUPNO;\n";
        let p = parse_csv(plodine.as_bytes()).unwrap().remove(0);
        assert_eq!(p.price_cents, 75);
        assert_eq!(
            (p.unit_price_cents, p.unit_basis.as_deref()),
            (Some(75), Some("L"))
        );
        // Eurospin, 19.11.2026: quoted fields; "0" means no promotion.
        let eurospin = "\"NAZIV_PROIZVODA\";\"ŠIFRA_PROIZVODA\";\"MARKA_PROIZVODA\";\"NETO_KOLIČINA\";\"JEDINICA_MJERE\";\"MALOPROD.CIJENA(EUR)\";\"CIJENA_ZA_JEDINICU_MJERE\";\"MPC_POSEB.OBLIK_PROD\";\"POSEB.OBLIK_PROD__ZA_JEDINICU_MJERE\";\"NAJNIŽA_MPC_U_30DANA\";\"SIDRENA_CIJENA\";\"BARKOD\";\"KATEGORIJA_PROIZVODA\"\n\"*AZIJSKI RAVIOLI S MESOM 120g\";\"018097501\";\"NO BRAND\";\"0,12\";\"KG\";\"3,99\";\"33,25\";\"0\";\"0\";\"0\";\"3,99\";\"8053369401379\";\"HRANA\"\n";
        let p = parse_csv(eurospin.as_bytes()).unwrap().remove(0);
        assert_eq!((p.price_cents, p.special_price_cents), (399, None));
        assert_eq!(
            (p.unit_price_cents, p.unit_basis.as_deref()),
            (Some(3325), Some("kg"))
        );
        // Studenac, 19.11.2026: XML; the regular price may be missing on promotion.
        let studenac = "<Proizvodi><ProdajniObjekt><Adresa>Velebitska 72 SPLIT</Adresa><Proizvodi>\
            <Proizvod><NazivProizvoda>ČOK. BANANKO MINI BNG 120 g</NazivProizvoda><MarkaProizvoda>KRAŠ</MarkaProizvoda><NetoKolicina/><JedinicaMjere>kom</JedinicaMjere><MaloprodajnaCijena/><CijenaZaJedinicuMjere>4.08</CijenaZaJedinicuMjere><MaloprodajnaCijenaAkcija>0.49</MaloprodajnaCijenaAkcija><Barkod>3850102016372</Barkod></Proizvod>\
            <Proizvod><NazivProizvoda>JAJA L 10/1 &amp; više</NazivProizvoda><MarkaProizvoda/><JedinicaMjere>kom</JedinicaMjere><MaloprodajnaCijena>3.99</MaloprodajnaCijena><CijenaZaJedinicuMjere>0.40</CijenaZaJedinicuMjere><MaloprodajnaCijenaAkcija>3.49</MaloprodajnaCijenaAkcija></Proizvod>\
            </Proizvodi></ProdajniObjekt></Proizvodi>";
        assert!(looks_like_xml(studenac.as_bytes()));
        let products = parse_xml(studenac.as_bytes()).unwrap();
        assert_eq!(products[0].price_cents, 49);
        assert_eq!(products[0].special_price_cents, None);
        assert_eq!(products[0].unit_basis.as_deref(), Some("kg"));
        assert_eq!(products[1].name, "JAJA L 10/1 & više");
        assert_eq!(
            (products[1].price_cents, products[1].special_price_cents),
            (399, Some(349))
        );
        assert_eq!(products[1].unit_basis.as_deref(), Some("kom"));

        assert_eq!(cents(",75"), Some(75));
        assert_eq!(cents(","), None);
        assert_eq!(name_size_basis("Mlijeko 2,8% 1,5L"), Some("L"));
        assert_eq!(name_size_basis("JAJA L 10/1"), None);
    }

    #[test]
    fn daily_zip_and_branch_files_of_the_new_chains_are_dated() {
        let html = "<a href='https://www.plodine.hr/cjenici/cjenici_17_11_2026_07_00_01.zip'>a</a>\
            <a href='https://www.plodine.hr/cjenici/cjenici_18_11_2026_07_00_01.zip'>b</a>\
            <a href='https://evil.test/cjenici_19_11_2026_07_00_01.zip'>c</a>";
        let eurospin = "<select><option value='https://www.eurospin.hr/wp-content/themes/eurospin/documenti-prezzi/cjenik_19.11.2026-7.30.zip'>x</option>\
            <option value='https://www.eurospin.hr/wp-content/themes/eurospin/documenti-prezzi/cjenik_18.11.2026-7.30.zip'>y</option></select>";
        assert!(
            latest_zip("https://www.eurospin.hr/cjenik/", eurospin)
                .unwrap()
                .ends_with("cjenik_19.11.2026-7.30.zip")
        );
        assert_eq!(
            latest_zip("https://www.plodine.hr/info-o-cijenama", html).as_deref(),
            Some("https://www.plodine.hr/cjenici/cjenici_18_11_2026_07_00_01.zip")
        );
        let key = |label: &str| {
            let (date, at) = price_date_at(label).unwrap();
            (
                date.to_string(),
                branch_key(label, at),
                postal_code(&label[..at]),
            )
        };
        let plodine = key("HIPERMARKET_MIJE_HALEUSA_1_10000_ZAGREB_055_553_18112026040401.csv");
        assert_eq!(plodine.0, "2026-11-18");
        assert_eq!(plodine.2.as_deref(), Some("10000"));
        let eurospin = key(
            "diskontna_prodavaonica-310002-I_Štefanovecki_zavoj_12-Zagreb-10000-310002191126-19.11.2026-7.30.csv",
        );
        assert_eq!(eurospin.0, "2026-11-19");
        assert_eq!(eurospin.2.as_deref(), Some("10000"));
        assert!(eurospin.1.contains("zagreb"));
        let studenac =
            key("Prodavaonica-Velebitska_72_SPLIT-T003-554-2026-11-19-07-00-25-567688.xml");
        assert_eq!(studenac.0, "2026-11-19");
        assert_eq!(studenac.1, "prodavaonica-velebitska-72-split-t003-554");
    }

    #[test]
    fn branch_key_survives_a_new_day_and_postal_code_is_found() {
        let key = |label: &str| {
            let (_, at) = price_date_at(label).unwrap();
            (branch_key(label, at), postal_code(&label[..at]))
        };
        let konzum_today =
            key("SUPER,ŽITNA 1A 10310 IVANIĆ GRAD,0204,108657,19.11.2026, 05-23.CSV");
        let konzum_tomorrow =
            key("SUPER,ŽITNA 1A 10310 IVANIĆ GRAD,0204,108912,20.11.2026, 05-11.CSV");
        assert_eq!(konzum_today, konzum_tomorrow);
        assert_eq!(konzum_today.0, "super-zitna-1a-ivanic-grad-0204");
        assert_eq!(konzum_today.1.as_deref(), Some("10310"));
        let lidl = key(
            "Supermarket 104_Ulica Dr. Franje Tuđmana_30_10450_Jastrebarsko_1_19.11.2026_7.15h.csv",
        );
        assert_eq!(lidl.1.as_deref(), Some("10450"));
        assert!(lidl.0.ends_with("jastrebarsko-1"));
        let spar = key(
            "hipermarket_zadar_bleiburskih_zrtava_18_8701_interspar_8701_zd_bleib._0567_20261119_0330.csv",
        );
        assert_eq!(spar.1, None);
        assert!(spar.0.starts_with("hipermarket-zadar"));
    }

    #[test]
    fn windows1250_quotes_and_changed_schema_are_handled() {
        let csv = "naziv;MPC (EUR)\n\"Čokolada; mliječna\";2,35\n";
        let (bytes, _, _) = encoding_rs::WINDOWS_1250.encode(csv);
        assert_eq!(parse_csv(&bytes).unwrap()[0].name, "Čokolada; mliječna");
        assert!(parse_csv(b"name;unknown\nitem;1\n").is_err());
    }

    #[test]
    fn detects_quoted_delimiters_multiline_fields_and_utf16() {
        let csv = "name|price|brand\n\"Milk, fresh; daily\n1 L\"|1.29|Test\n";
        let products = parse_csv(csv.as_bytes()).unwrap();
        assert_eq!(products[0].price_cents, 129);
        assert!(products[0].name.contains('\n'));
        for little_endian in [true, false] {
            let mut bytes = if little_endian {
                vec![0xff, 0xfe]
            } else {
                vec![0xfe, 0xff]
            };
            for unit in "naziv;cijena\nČokolada;2,49\n".encode_utf16() {
                bytes.extend(if little_endian {
                    unit.to_le_bytes()
                } else {
                    unit.to_be_bytes()
                });
            }
            let product = &parse_csv(&bytes).unwrap()[0];
            assert_eq!(product.name, "Čokolada");
            assert_eq!(product.price_cents, 249);
        }
    }

    #[test]
    fn unknown_and_ambiguous_headers_require_mapping_and_validate_indices() {
        let bytes = b"opis;iznos;ean\nJogurt;0,99;0012345\n";
        let inspection = inspect_csv(bytes).unwrap();
        assert!(inspection.suggested.is_none());
        assert_eq!(inspection.sample[0][0], "Jogurt");
        assert!(parse_csv(bytes).is_err());
        let mut mapping = CsvMapping {
            delimiter: b';',
            name: 0,
            price: 1,
            barcode: Some(2),
            brand: None,
            quantity: None,
            unit: None,
            unit_price: None,
            unit_price_basis: None,
            special_price: None,
        };
        let products = parse_csv_mapped(bytes, &mapping).unwrap();
        assert_eq!(products[0].barcode.as_deref(), Some("0012345"));
        assert_eq!(products[0].price_cents, 99);
        mapping.price = 0;
        assert!(parse_csv_mapped(bytes, &mapping).is_err());
        mapping.price = 99;
        assert!(parse_csv_mapped(bytes, &mapping).is_err());
        assert!(
            inspect_csv(b"naziv;cijena;MPC\nJogurt;1;2\n")
                .unwrap()
                .suggested
                .is_none()
        );
        assert!(parse_csv(b"naziv;cijena\nJogurt;1;extra\n").is_err());
        assert!(parse_csv(b"naziv;cijena\nJogurt;1,001\n").is_err());
    }
}
