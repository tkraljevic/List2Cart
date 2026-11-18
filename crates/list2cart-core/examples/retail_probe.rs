//! Manual live check of public retailer data; never touches the user database.
#[tokio::main]
async fn main() {
    let service = list2cart_core::retail::RetailSearch::default();
    for chain in std::env::args().skip(1) {
        match service.list_sources(&chain).await {
            Ok(sources) => {
                println!("{chain}: {} poslovnica", sources.len());
                if let Some(source) = sources.first() {
                    match service.search(&source.url, "mlijeko").await {
                        Ok(result) => println!(
                            "  {}: {} rezultata, prvi: {:?}",
                            source.price_date,
                            result.total_matches,
                            result.products.first()
                        ),
                        Err(error) => println!("  SEARCH ERROR: {error}"),
                    }
                }
            }
            Err(error) => println!("{chain}: INDEX ERROR: {error}"),
        }
    }
}
