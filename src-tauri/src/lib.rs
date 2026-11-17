use list2cart_core::{
    calc::{calculate_line_total_cents, verify_receipt as core_verify_receipt},
    db::Database,
    db::UpsertProductInput,
    excel::export_receipt_to_xlsx,
    ledger::{LedgerStats, NetworkMember, PeerRole, generate_sas_code},
    models::{
        CartItem, OnlineProductInfo, PlannedItem, PocketBaseConfig, Product, ReceiptSummary,
        ReceiptVerification, ShoppingList, Store, StorePrice, SyncReport, Unit,
    },
    online::fetch_openfoodfacts_product,
    sync::sync_with_pocketbase_http,
};
use std::sync::Mutex;
use tauri::{Manager, State};

pub struct AppState {
    pub db: Mutex<Database>,
    pub export_dir: std::path::PathBuf,
}

#[tauri::command]
fn list_stores(state: State<'_, AppState>) -> Result<Vec<Store>, String> {
    let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
    db.list_stores()
}

#[tauri::command]
fn add_store(
    state: State<'_, AppState>,
    name: String,
    location: String,
    color: String,
) -> Result<Store, String> {
    let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
    db.add_store(&name, &location, &color)
}

#[tauri::command]
fn list_products(
    state: State<'_, AppState>,
    search: Option<String>,
) -> Result<Vec<Product>, String> {
    let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
    db.list_products(search.as_deref())
}

#[tauri::command]
fn get_product(state: State<'_, AppState>, barcode: String) -> Result<Option<Product>, String> {
    let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
    db.get_product(&barcode)
}

#[tauri::command]
fn upsert_product(
    state: State<'_, AppState>,
    barcode: String,
    name: String,
    category: String,
    default_unit: Unit,
    price_cents: i64,
    store_name: String,
    image_data_url: Option<String>,
) -> Result<Product, String> {
    let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
    db.upsert_product(&UpsertProductInput {
        barcode: &barcode,
        name: &name,
        category: &category,
        default_unit,
        price_cents,
        store_name: &store_name,
        image_data_url: image_data_url.as_deref(),
    })
}

#[tauri::command]
fn get_product_store_prices(
    state: State<'_, AppState>,
    barcode: String,
) -> Result<Vec<StorePrice>, String> {
    let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
    db.get_product_store_prices(&barcode)
}

#[tauri::command]
fn calculate_line_total(quantity: f64, unit: Unit, unit_price_cents: i64) -> i64 {
    calculate_line_total_cents(quantity, unit, unit_price_cents)
}

#[tauri::command]
fn verify_receipt(cart_total_cents: i64, receipt_total_cents: i64) -> ReceiptVerification {
    core_verify_receipt(cart_total_cents, receipt_total_cents)
}

#[tauri::command]
fn save_receipt(
    state: State<'_, AppState>,
    store_id: i64,
    store_name: String,
    items: Vec<CartItem>,
    receipt_total_cents: i64,
) -> Result<ReceiptSummary, String> {
    let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
    db.save_receipt(store_id, &store_name, &items, receipt_total_cents)
}

#[tauri::command]
fn list_receipts(state: State<'_, AppState>) -> Result<Vec<ReceiptSummary>, String> {
    let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
    db.list_receipts()
}

#[tauri::command]
fn export_receipt_xlsx(
    state: State<'_, AppState>,
    receipt: ReceiptSummary,
) -> Result<String, String> {
    std::fs::create_dir_all(&state.export_dir)
        .map_err(|e| format!("Greska kreiranja mape za izvoz: {e}"))?;
    let safe_store = receipt
        .store_name
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect::<String>();
    let filename = format!("List2Cart_{safe_store}_{}.xlsx", receipt.id);
    let path = state.export_dir.join(filename);
    export_receipt_to_xlsx(&receipt, &path)?;
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
fn list_shopping_lists(state: State<'_, AppState>) -> Result<Vec<ShoppingList>, String> {
    let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
    db.list_shopping_lists()
}

#[tauri::command]
fn create_shopping_list(
    state: State<'_, AppState>,
    name: String,
    store_id: Option<i64>,
) -> Result<ShoppingList, String> {
    let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
    db.create_shopping_list(&name, store_id)
}

#[tauri::command]
fn add_planned_item(
    state: State<'_, AppState>,
    list_id: i64,
    barcode: Option<String>,
    name: String,
    quantity: f64,
    unit: Unit,
    estimated_price_cents: i64,
) -> Result<PlannedItem, String> {
    let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
    db.add_planned_item(
        list_id,
        barcode.as_deref(),
        &name,
        quantity,
        unit,
        estimated_price_cents,
    )
}

#[tauri::command]
fn toggle_planned_item(
    state: State<'_, AppState>,
    item_id: i64,
    checked: bool,
) -> Result<(), String> {
    let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
    db.toggle_planned_item(item_id, checked)
}

#[tauri::command]
async fn lookup_barcode_online(barcode: String) -> Result<Option<OnlineProductInfo>, String> {
    fetch_openfoodfacts_product(&barcode).await
}

#[tauri::command]
fn save_pocketbase_url(state: State<'_, AppState>, url: String) -> Result<(), String> {
    let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
    db.set_setting("pocketbase_url", url.trim())
}

#[tauri::command]
fn get_pocketbase_url(state: State<'_, AppState>) -> Result<Option<String>, String> {
    let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
    db.get_setting("pocketbase_url")
}

#[tauri::command]
fn export_sync_bundle_json(
    state: State<'_, AppState>,
    role: Option<PeerRole>,
) -> Result<String, String> {
    let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
    let bundle = db.export_sync_bundle_for_role(role.unwrap_or(PeerRole::OwnDevice))?;
    serde_json::to_string_pretty(&bundle).map_err(|e| format!("Greska serijalizacije: {e}"))
}

#[tauri::command]
fn set_network_name(state: State<'_, AppState>, name: String) -> Result<String, String> {
    let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
    db.set_network_name(&name)
}

#[tauri::command]
fn new_sas_code() -> String {
    generate_sas_code()
}

#[tauri::command]
fn list_network_members(state: State<'_, AppState>) -> Result<Vec<NetworkMember>, String> {
    let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
    db.list_members()
}

#[tauri::command]
fn add_network_member(
    state: State<'_, AppState>,
    name: String,
    role: PeerRole,
    sas_code: String,
    shared_lists: String,
) -> Result<NetworkMember, String> {
    let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
    db.add_member(&name, role, &sas_code, &shared_lists)
}

#[tauri::command]
fn remove_network_member(state: State<'_, AppState>, peer_id: String) -> Result<(), String> {
    let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
    db.remove_member(&peer_id)
}

#[tauri::command]
fn import_sync_bundle_json(
    state: State<'_, AppState>,
    json_str: String,
) -> Result<(usize, usize, usize), String> {
    let bundle: list2cart_core::models::SyncBundle = serde_json::from_str(&json_str)
        .map_err(|e| format!("Neispravna .list2cart datoteka: {e}"))?;
    let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
    db.import_sync_bundle(&bundle)
}

#[tauri::command]
fn get_ledger_stats(state: State<'_, AppState>) -> Result<LedgerStats, String> {
    let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
    db.ledger_stats()
}

#[tauri::command]
fn run_ledger_compact(state: State<'_, AppState>) -> Result<usize, String> {
    let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
    db.compact_ledger()
}

#[tauri::command]
async fn sync_pocketbase(
    state: State<'_, AppState>,
    url: String,
    auth_token: Option<String>,
) -> Result<SyncReport, String> {
    let config = PocketBaseConfig { url, auth_token };
    let (stores, products) = {
        let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
        (db.list_stores()?, db.list_products(None)?)
    };

    let (mut report, pulled_stores) = sync_with_pocketbase_http(stores, products, &config).await?;

    let mut applied = 0usize;
    {
        let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
        for s in pulled_stores {
            if db.add_store(&s.name, &s.location, &s.color).is_ok() {
                applied += 1;
            }
        }
    }
    report.pulled_stores = applied;
    Ok(report)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            #[cfg(mobile)]
            app.handle().plugin(tauri_plugin_barcode_scanner::init())?;

            let data_dir = app
                .path()
                .app_data_dir()
                .map_err(|e| format!("Greska putanje podataka: {e}"))?;
            std::fs::create_dir_all(&data_dir)?;
            let db_path = data_dir.join("list2cart.sqlite");
            let db = Database::open(&db_path)?;

            let export_dir = app.path().download_dir().unwrap_or(data_dir);

            app.manage(AppState {
                db: Mutex::new(db),
                export_dir,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_stores,
            add_store,
            list_products,
            get_product,
            upsert_product,
            get_product_store_prices,
            calculate_line_total,
            verify_receipt,
            save_receipt,
            list_receipts,
            export_receipt_xlsx,
            list_shopping_lists,
            create_shopping_list,
            add_planned_item,
            toggle_planned_item,
            lookup_barcode_online,
            save_pocketbase_url,
            get_pocketbase_url,
            sync_pocketbase,
            export_sync_bundle_json,
            import_sync_bundle_json,
            get_ledger_stats,
            run_ledger_compact,
            set_network_name,
            new_sas_code,
            list_network_members,
            add_network_member,
            remove_network_member,
        ])
        .run(tauri::generate_context!())
        .expect("Greska pri pokretanju List2Cart aplikacije");
}
