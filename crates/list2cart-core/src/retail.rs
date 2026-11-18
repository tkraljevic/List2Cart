//! Public retailer price lists, fetched directly and searched on the device.
use chrono::{Datelike, NaiveDate, Utc};
use reqwest::{Client, Url};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceSource {
    pub chain: String,
    pub label: String,
    pub url: String,
    pub price_date: String,
}
#[derive(Debug, Clone, Serialize)]
pub struct RetailProduct {
    pub name: String,
    pub barcode: Option<String>,
    pub brand: String,
    pub package: String,
    pub price_cents: i64,
}
#[derive(Serialize)]
pub struct RetailResults {
    pub source: PriceSource,
    pub fetched_at: String,
    pub products: Vec<RetailProduct>,
    pub total_matches: usize,
}
struct Cached {
    at: Instant,
    fetched_at: String,
    products: Vec<RetailProduct>,
}
#[derive(Default)]
pub struct RetailSearch {
    sources: Mutex<HashMap<String, PriceSource>>,
    cache: Mutex<HashMap<String, Cached>>,
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
fn price_date(label: &str) -> Option<NaiveDate> {
    for (i, _) in label.char_indices() {
        for (length, formats) in [(10, &["%d.%m.%Y"][..]), (8, &["%Y%m%d", "%d%m%Y"][..])] {
            if let Some(piece) = label.get(i..i + length) {
                for format in formats {
                    if let Ok(date) = NaiveDate::parse_from_str(piece, format)
                        && (2025..=Utc::now().year() + 1).contains(&date.year())
                    {
                        return Some(date);
                    }
                }
            }
        }
    }
    None
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
    let date = price_date(&label)?;
    Some(PriceSource {
        chain: chain.into(),
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
            _ => return Err("Podrzani lanci: Konzum, Lidl, Kaufland i Spar".into()),
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
        let mut registry = self.sources.lock().map_err(|_| "Greska registra cjenika")?;
        registry.retain(|_, s| s.chain != chain);
        for s in &result {
            registry.insert(s.url.clone(), s.clone());
        }
        Ok(result)
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
                .filter(|c| c.at.elapsed() < Duration::from_secs(1800))
            {
                return Ok(results(&source, c, &tokens));
            }
        }
        let bytes = download(&client()?, source_url, 12_000_000).await?;
        let products = parse_csv(&bytes)?;
        let cached = Cached {
            at: Instant::now(),
            fetched_at: Utc::now().to_rfc3339(),
            products,
        };
        let result = results(&source, &cached, &tokens);
        let mut cache = self.cache.lock().map_err(|_| "Greska predmemorije")?;
        if cache.len() >= 4 {
            cache.clear();
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
fn cents(value: &str) -> Option<i64> {
    let value = value.trim().trim_end_matches('€').trim().replace(',', ".");
    let mut parts = value.split('.');
    let whole = parts.next()?;
    if whole.is_empty() || !whole.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let fraction = parts.next().unwrap_or("");
    if parts.next().is_some() || fraction.len() > 2 || !fraction.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    let frac = match fraction.len() {
        0 => 0,
        1 => fraction.parse::<i64>().ok()? * 10,
        _ => fraction.parse().ok()?,
    };
    let amount = whole
        .parse::<i64>()
        .ok()?
        .checked_mul(100)?
        .checked_add(frac)?;
    (amount <= crate::calc::MAX_SAFE_CENTS).then_some(amount)
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
        ..
    } = *mapping;
    if name == price
        || [Some(name), Some(price), barcode, brand, quantity, unit]
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
        products.push(RetailProduct {
            name: name.into(),
            barcode,
            brand: brand.and_then(|i| row.get(i)).unwrap_or("").trim().into(),
            package: format!(
                "{} {}",
                quantity.and_then(|i| row.get(i)).unwrap_or(""),
                unit.and_then(|i| row.get(i)).unwrap_or("")
            )
            .trim()
            .into(),
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
