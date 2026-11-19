use list2cart_core::{
    calc::{calculate_line_total_cents, verify_receipt as core_verify_receipt},
    db::Database,
    db::UpsertProductInput,
    excel::{export_list_to_xlsx, export_receipt_to_xlsx, export_receipts_to_xlsx},
    ledger::{LedgerStats, NetworkMember, PeerRole, generate_sas_code},
    models::{
        CartItem, OnlineProductInfo, PlannedItem, PocketBaseConfig, Product, ReceiptSummary,
        ReceiptVerification, ShoppingList, Store, StorePrice, SyncReport, Unit,
    },
    online::fetch_openfoodfacts_product,
    sync::sync_with_pocketbase_http,
};
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager, State};

pub struct AppState {
    pub db: Arc<Mutex<Database>>,
    pub retail: list2cart_core::retail::RetailSearch,
    pub pairing: list2cart_core::pairing_session::PairingController,
    pub lan: Mutex<Option<Arc<list2cart_core::lan_service::LanSyncService>>>,
    pub data_dir: std::path::PathBuf,
    pub export_dir: std::path::PathBuf,
}

impl AppState {
    fn notify_lan_change(&self) {
        if let Ok(guard) = self.lan.lock()
            && let Some(svc) = guard.as_ref()
        {
            svc.notify_local_change();
        }
    }
}

#[tauri::command]
fn inspect_price_csv(bytes: Vec<u8>) -> Result<list2cart_core::retail::CsvInspection, String> {
    list2cart_core::retail::inspect_csv(&bytes)
}
#[tauri::command]
fn read_price_csv(
    bytes: Vec<u8>,
    mapping: list2cart_core::retail::CsvMapping,
) -> Result<Vec<list2cart_core::retail::RetailProduct>, String> {
    list2cart_core::retail::parse_csv_mapped(&bytes, &mapping)
}
#[tauri::command]
async fn retail_sources(
    state: State<'_, AppState>,
    chain: String,
) -> Result<Vec<list2cart_core::retail::PriceSource>, String> {
    state.retail.list_sources(&chain).await
}
#[tauri::command]
async fn retail_search(
    state: State<'_, AppState>,
    source_url: String,
    query: String,
) -> Result<list2cart_core::retail::RetailResults, String> {
    state.retail.search(&source_url, &query).await
}

const HOME_BRANCHES_KEY: &str = "retail_home_branches";

fn read_home_branches(state: &AppState) -> Result<Vec<list2cart_core::retail::HomeBranch>, String> {
    let saved = state
        .db
        .lock()
        .map_err(|_| "Greska zakljucavanja baze")?
        .get_setting(HOME_BRANCHES_KEY)?;
    // A damaged value only means the branches have to be chosen again.
    Ok(saved
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default())
}
fn write_home_branches(
    state: &AppState,
    branches: &[list2cart_core::retail::HomeBranch],
) -> Result<(), String> {
    let json = serde_json::to_string(branches).map_err(|e| e.to_string())?;
    state
        .db
        .lock()
        .map_err(|_| "Greska zakljucavanja baze")?
        .set_setting(HOME_BRANCHES_KEY, &json)
}
#[tauri::command]
async fn retail_all_sources(
    state: State<'_, AppState>,
) -> Result<Vec<list2cart_core::retail::ChainSources>, String> {
    Ok(state.retail.all_sources().await)
}
#[tauri::command]
fn retail_home_branches(
    state: State<'_, AppState>,
) -> Result<Vec<list2cart_core::retail::HomeBranch>, String> {
    read_home_branches(&state)
}
/// Saves the chosen branches, one per chain; a chain not mentioned keeps its branch.
#[tauri::command]
fn set_retail_home_branches(
    state: State<'_, AppState>,
    source_urls: Vec<String>,
) -> Result<Vec<list2cart_core::retail::HomeBranch>, String> {
    let mut branches = read_home_branches(&state)?;
    for url in source_urls {
        let source = state.retail.registered_source(&url)?;
        branches.retain(|b| b.chain != source.chain);
        branches.push(list2cart_core::retail::HomeBranch {
            chain: source.chain,
            branch_key: source.branch_key,
            label: source.label,
        });
    }
    branches.sort_by_key(|b| {
        list2cart_core::retail::CHAINS
            .iter()
            .position(|c| *c == b.chain)
            .unwrap_or(usize::MAX)
    });
    write_home_branches(&state, &branches)?;
    Ok(branches)
}
#[tauri::command]
fn remove_retail_home_branch(
    state: State<'_, AppState>,
    chain: String,
) -> Result<Vec<list2cart_core::retail::HomeBranch>, String> {
    let mut branches = read_home_branches(&state)?;
    branches.retain(|b| b.chain != chain);
    write_home_branches(&state, &branches)?;
    Ok(branches)
}
/// Searches the saved branches; `chains` limits it to some of them (one store).
#[tauri::command]
async fn retail_search_home(
    state: State<'_, AppState>,
    query: String,
    chains: Option<Vec<String>>,
) -> Result<Vec<list2cart_core::retail::BranchSearch>, String> {
    let mut branches = read_home_branches(&state)?;
    if let Some(chains) = chains {
        branches.retain(|b| chains.contains(&b.chain));
    }
    if branches.is_empty() {
        return Err("Najprije odaberite svoje poslovnice".into());
    }
    Ok(state.retail.search_branches(&branches, &query).await)
}

#[tauri::command]
fn identity_conflicts(
    state: State<'_, AppState>,
) -> Result<Vec<list2cart_core::db::IdentityConflict>, String> {
    state
        .db
        .lock()
        .map_err(|_| "Greska zakljucavanja baze")?
        .identity_conflicts()
}

#[tauri::command]
fn resolve_identity_conflict(
    state: State<'_, AppState>,
    entity: String,
    keep_id: String,
    expected_ids: Vec<String>,
) -> Result<(), String> {
    let res = state
        .db
        .lock()
        .map_err(|_| "Greska zakljucavanja baze")?
        .resolve_identity_conflict(&entity, &keep_id, &expected_ids);
    if res.is_ok() {
        state.notify_lan_change();
    }
    res
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
    let res = {
        let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
        db.add_store(&name, &location, &color)
    };
    if res.is_ok() {
        state.notify_lan_change();
    }
    res
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
#[allow(clippy::too_many_arguments)]
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
    let res = {
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
    };
    if res.is_ok() {
        state.notify_lan_change();
    }
    res
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
    let res = {
        let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
        db.save_receipt(store_id, &store_name, &items, receipt_total_cents)
    };
    if res.is_ok() {
        state.notify_lan_change();
    }
    res
}

#[tauri::command]
fn list_receipts(state: State<'_, AppState>) -> Result<Vec<ReceiptSummary>, String> {
    let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
    db.list_receipts()
}

#[tauri::command]
fn incomplete_receipt_count(state: State<'_, AppState>) -> Result<usize, String> {
    state
        .db
        .lock()
        .map_err(|_| "Greska zakljucavanja baze")?
        .incomplete_receipt_count()
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

/// Saves the receipts the Računi view shows in one workbook, oldest first; returns
/// its path. The receipts are read from the database by their IDs.
#[tauri::command]
fn export_receipts_xlsx(
    state: State<'_, AppState>,
    receipt_ids: Vec<i64>,
    selection: String,
    exported_on: String,
) -> Result<String, String> {
    let mut receipts: Vec<ReceiptSummary> = {
        let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
        db.list_receipts()?
            .into_iter()
            .filter(|r| receipt_ids.contains(&r.id))
            .collect()
    };
    if receipts.is_empty() {
        return Err("Nema racuna za izvoz".into());
    }
    receipts.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    std::fs::create_dir_all(&state.export_dir)
        .map_err(|e| format!("Greska kreiranja mape za izvoz: {e}"))?;
    let date: String = exported_on.chars().take(20).collect();
    let safe_date: String = date
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.')
        .collect::<String>()
        .trim_matches('.')
        .replace('.', "-");
    let path = state
        .export_dir
        .join(format!("List2Cart_racuni_{safe_date}.xlsx"));
    let selection: String = selection.chars().take(200).collect();
    export_receipts_to_xlsx(&receipts, &selection, &date, &path)?;
    Ok(path.to_string_lossy().into_owned())
}

/// Saves one shopping list as an Excel table in the export folder; returns its path.
/// The list is read from the database, so the file shows what is stored.
#[tauri::command]
fn export_list_xlsx(
    state: State<'_, AppState>,
    list_id: i64,
    exported_on: String,
) -> Result<String, String> {
    let list = {
        let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
        db.list_shopping_lists()?
            .into_iter()
            .find(|l| l.id == list_id)
            .ok_or("Popis nije pronadjen")?
    };
    std::fs::create_dir_all(&state.export_dir)
        .map_err(|e| format!("Greska kreiranja mape za izvoz: {e}"))?;
    let safe_name: String = list
        .name
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    let path = state
        .export_dir
        .join(format!("List2Cart_popis_{safe_name}_{}.xlsx", list.id));
    let date: String = exported_on.chars().take(20).collect();
    export_list_to_xlsx(&list, &date, &path)?;
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
fn list_shopping_lists(state: State<'_, AppState>) -> Result<Vec<ShoppingList>, String> {
    let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
    db.list_shopping_lists()
}

#[tauri::command]
fn delete_shopping_list(state: State<'_, AppState>, list_id: i64) -> Result<(), String> {
    let res = {
        let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
        db.delete_shopping_list(list_id)
    };
    if res.is_ok() {
        state.notify_lan_change();
    }
    res
}

/// Shares a private list with every member of the network, or stops sharing it.
#[tauri::command]
fn set_list_shared(
    state: State<'_, AppState>,
    list_id: i64,
    shared: bool,
) -> Result<ShoppingList, String> {
    let res = {
        let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
        if shared {
            db.share_shopping_list(list_id)
        } else {
            db.unshare_shopping_list(list_id)
        }
    };
    if res.is_ok() {
        state.notify_lan_change();
    }
    res
}

#[tauri::command]
fn create_shopping_list(
    state: State<'_, AppState>,
    name: String,
    store_id: Option<i64>,
    shared: Option<bool>,
) -> Result<ShoppingList, String> {
    let res = {
        let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
        if shared.unwrap_or(false) {
            db.create_shared_shopping_list(&name, store_id)
        } else {
            db.create_shopping_list(&name, store_id)
        }
    };
    if res.is_ok() {
        state.notify_lan_change();
    }
    res
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
    let res = {
        let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
        db.add_planned_item(
            list_id,
            barcode.as_deref(),
            &name,
            quantity,
            unit,
            estimated_price_cents,
        )
    };
    if res.is_ok() {
        state.notify_lan_change();
    }
    res
}

#[tauri::command]
fn toggle_planned_item(
    state: State<'_, AppState>,
    item_id: i64,
    checked: bool,
) -> Result<(), String> {
    let res = {
        let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
        db.toggle_planned_item(item_id, checked)
    };
    if res.is_ok() {
        state.notify_lan_change();
    }
    res
}

#[tauri::command]
fn remove_planned_item(state: State<'_, AppState>, item_sync_id: String) -> Result<(), String> {
    let res = {
        let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
        db.remove_planned_item(&item_sync_id)
    };
    if res.is_ok() {
        state.notify_lan_change();
    }
    res
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
    shared_lists: Option<String>,
) -> Result<String, String> {
    let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
    let bundle = db.export_sync_bundle_filtered(
        role.unwrap_or(PeerRole::OwnDevice),
        shared_lists.as_deref(),
    )?;
    serde_json::to_string_pretty(&bundle).map_err(|e| format!("Greska serijalizacije: {e}"))
}

#[tauri::command]
fn save_sync_bundle_file(
    state: State<'_, AppState>,
    json_str: String,
    family_only: bool,
) -> Result<String, String> {
    use std::io::Write;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let prefix = if family_only { "OBITELJ" } else { "SVE" };
    std::fs::create_dir_all(&state.export_dir).map_err(|e| e.to_string())?;
    let path = state
        .export_dir
        .join(format!("List2Cart_{prefix}_{stamp}.list2cart.json"));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    file.write_all(json_str.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|e| format!("Paket nije potpuno spremljen: {e}"))?;
    Ok(path.to_string_lossy().into_owned())
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
    let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
    db.import_sync_package_json(&json_str)
}

#[tauri::command]
fn get_sync_identity(
    state: State<'_, AppState>,
) -> Result<list2cart_core::db::SyncIdentity, String> {
    let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
    Ok(db.sync_identity())
}

#[tauri::command]
fn create_sync_request(
    state: State<'_, AppState>,
    peer: String,
) -> Result<list2cart_core::exchange::SyncRequest, String> {
    let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
    db.sync_request(&peer)
}

#[tauri::command]
fn create_sync_reply(
    state: State<'_, AppState>,
    request: list2cart_core::exchange::SyncRequest,
    max_bytes: Option<usize>,
) -> Result<String, String> {
    let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
    let bundle = db.sync_reply(&request, max_bytes.unwrap_or(256 * 1024))?;
    serde_json::to_string_pretty(&bundle).map_err(|e| format!("Greska serijalizacije: {e}"))
}

#[tauri::command]
fn receive_sync_page(
    state: State<'_, AppState>,
    bundle_json: String,
) -> Result<list2cart_core::exchange::SyncReceiveOutcome, String> {
    let bundle: list2cart_core::apply::ProtectedBundleV2 = serde_json::from_str(&bundle_json)
        .map_err(|e| format!("Neispravan zaštićeni v2 paket: {e}"))?;
    let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
    db.import_sync_page(&bundle)
}

#[tauri::command]
fn get_pair_status(
    state: State<'_, AppState>,
) -> Result<list2cart_core::pairing_session::PairStatusView, String> {
    let can_admit = {
        let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
        db.can_admit_members()?
    };
    Ok(state.pairing.status(can_admit))
}

#[tauri::command]
async fn start_pair_listening(
    state: State<'_, AppState>,
    port: Option<u16>,
    device_name: Option<String>,
) -> Result<list2cart_core::pairing_session::PairStatusView, String> {
    let (sk, identity, can_admit) = {
        let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
        let sync_id = db.sync_identity();
        let name = device_name
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "List2Cart uredjaj".to_owned());
        (
            db.clone_device_signing_key(),
            list2cart_core::pairing_session::PairingController::build_pair_identity(
                &sync_id.device_public_key,
                &name,
                list2cart_core::apply::PROTOCOL_NAME,
            ),
            db.can_admit_members()?,
        )
    };
    let bind_addr = std::net::SocketAddr::from(([0, 0, 0, 0], port.unwrap_or(0)));
    let status = state
        .pairing
        .start_listening(bind_addr, sk, identity, can_admit)
        .await?;
    if let Some(listen_port) = status.listen_port {
        let _ = list2cart_core::lan_service::write_pairing_flag(&state.data_dir, listen_port);
    }
    Ok(status)
}

#[tauri::command]
fn stop_pair_listening(
    state: State<'_, AppState>,
) -> Result<list2cart_core::pairing_session::PairStatusView, String> {
    list2cart_core::lan_service::clear_pairing_flag(&state.data_dir);
    let can_admit = {
        let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
        db.can_admit_members()?
    };
    Ok(state.pairing.stop_listening(can_admit))
}

#[tauri::command]
async fn dial_pair_peer(
    state: State<'_, AppState>,
    addr: String,
    device_name: Option<String>,
) -> Result<list2cart_core::pairing_session::PairStatusView, String> {
    let parsed_addr: std::net::SocketAddr = addr
        .trim()
        .parse()
        .map_err(|e| format!("Neispravna adresa uredjaja (IP:port): {e}"))?;
    let (sk, identity, can_admit) = {
        let db = state.db.lock().map_err(|_| "Greska zakljucavanja baze")?;
        let sync_id = db.sync_identity();
        let name = device_name
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "List2Cart uredjaj".to_owned());
        (
            db.clone_device_signing_key(),
            list2cart_core::pairing_session::PairingController::build_pair_identity(
                &sync_id.device_public_key,
                &name,
                list2cart_core::apply::PROTOCOL_NAME,
            ),
            db.can_admit_members()?,
        )
    };
    state
        .pairing
        .dial(parsed_addr, sk, identity, can_admit)
        .await
}

#[tauri::command]
async fn confirm_pair_peer(
    state: State<'_, AppState>,
    approved: bool,
    role: Option<PeerRole>,
    shared_lists: Option<String>,
    admit: Option<bool>,
) -> Result<list2cart_core::pairing_session::PairOutcomeView, String> {
    let outcome = state
        .pairing
        .confirm(
            &state.db,
            approved,
            role.unwrap_or(PeerRole::OwnDevice),
            shared_lists.as_deref().unwrap_or("*"),
            admit.unwrap_or(true),
        )
        .await?;
    list2cart_core::lan_service::clear_pairing_flag(&state.data_dir);
    if outcome.paired {
        state.notify_lan_change();
    }
    Ok(outcome)
}

fn lan_status_snapshot(
    state: &AppState,
) -> Result<list2cart_core::lan_service::LanServiceStatus, String> {
    if let Ok(guard) = state.lan.lock()
        && let Some(svc) = guard.as_ref()
    {
        return Ok(svc.status());
    }
    let device_public_key = state
        .db
        .lock()
        .map_err(|_| "Greska zakljucavanja baze".to_owned())?
        .sync_identity()
        .device_public_key;
    Ok(list2cart_core::lan_service::LanServiceStatus {
        running: false,
        paused: false,
        exchange_addr: String::new(),
        discovery_addr: None,
        device_public_key,
        active_conversations: 0,
        last_round_at: None,
        last_error: Some("LAN servis nije pokrenut".to_owned()),
        peers: Vec::new(),
    })
}

fn emit_lan_status(app: &tauri::AppHandle, state: &AppState) {
    if let Ok(status) = lan_status_snapshot(state) {
        let _ = app.emit("lan-status-changed", status);
    }
}

async fn spawn_lan_service(
    db: Arc<Mutex<Database>>,
    data_dir: std::path::PathBuf,
) -> Result<Arc<list2cart_core::lan_service::LanSyncService>, String> {
    let mut cfg = list2cart_core::lan_service::LanServiceConfig {
        data_dir: Some(data_dir),
        ..Default::default()
    };
    match list2cart_core::lan_service::LanSyncService::start(Arc::clone(&db), cfg.clone()).await {
        Ok(svc) => Ok(Arc::new(svc)),
        Err(err) => {
            eprintln!("LAN discovery bind nije uspio ({err}); pokrecem bez UDP otkrivanja");
            cfg.discovery_bind_addr = None;
            list2cart_core::lan_service::LanSyncService::start(db, cfg)
                .await
                .map(Arc::new)
        }
    }
}

#[tauri::command]
fn get_lan_status(
    state: State<'_, AppState>,
) -> Result<list2cart_core::lan_service::LanServiceStatus, String> {
    lan_status_snapshot(&state)
}

#[tauri::command]
async fn start_lan_sync(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<list2cart_core::lan_service::LanServiceStatus, String> {
    let already_running = state
        .lan
        .lock()
        .map_err(|_| "Greska zakljucavanja LAN servisa".to_owned())?
        .as_ref()
        .is_some_and(|svc| svc.status().running);
    if !already_running {
        let svc = spawn_lan_service(Arc::clone(&state.db), state.data_dir.clone()).await?;
        let mut guard = state
            .lan
            .lock()
            .map_err(|_| "Greska zakljucavanja LAN servisa".to_owned())?;
        *guard = Some(svc);
    }
    let status = lan_status_snapshot(&state)?;
    let _ = app.emit("lan-status-changed", &status);
    Ok(status)
}

#[tauri::command]
fn stop_lan_sync(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<list2cart_core::lan_service::LanServiceStatus, String> {
    let existing = {
        let mut guard = state
            .lan
            .lock()
            .map_err(|_| "Greska zakljucavanja LAN servisa".to_owned())?;
        guard.take()
    };
    if let Some(svc) = existing {
        svc.stop();
    }
    let status = lan_status_snapshot(&state)?;
    let _ = app.emit("lan-status-changed", &status);
    Ok(status)
}

#[tauri::command]
fn set_lan_paused(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    paused: bool,
) -> Result<list2cart_core::lan_service::LanServiceStatus, String> {
    {
        let guard = state
            .lan
            .lock()
            .map_err(|_| "Greska zakljucavanja LAN servisa".to_owned())?;
        let svc = guard
            .as_ref()
            .ok_or_else(|| "LAN servis nije pokrenut".to_owned())?;
        svc.set_paused(paused);
    }
    let status = lan_status_snapshot(&state)?;
    let _ = app.emit("lan-status-changed", &status);
    Ok(status)
}

#[tauri::command]
async fn discover_lan_peers() -> Result<Vec<list2cart_core::lan_service::FoundPeer>, String> {
    list2cart_core::lan_service::discover_lan_peers(std::time::Duration::from_millis(700)).await
}

#[tauri::command]
async fn trigger_lan_sync(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<usize, String> {
    let svc = {
        let guard = state
            .lan
            .lock()
            .map_err(|_| "Greska zakljucavanja LAN servisa".to_owned())?;
        guard
            .as_ref()
            .cloned()
            .ok_or_else(|| "LAN servis nije pokrenut".to_owned())?
    };
    let res = svc.sync_now().await;
    emit_lan_status(&app, &state);
    res
}

#[tauri::command]
fn set_manual_peer_addr(
    state: State<'_, AppState>,
    peer_public_key: String,
    addr: Option<String>,
) -> Result<(), String> {
    let parsed = match addr.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(raw) => Some(
            raw.parse::<std::net::SocketAddr>()
                .map_err(|e| format!("Neispravna adresa partnera (IP:port): {e}"))?,
        ),
        None => None,
    };
    let guard = state
        .lan
        .lock()
        .map_err(|_| "Greska zakljucavanja LAN servisa".to_owned())?;
    let svc = guard
        .as_ref()
        .ok_or_else(|| "LAN servis nije pokrenut".to_owned())?;
    svc.set_manual_peer_addr(&peer_public_key, parsed);
    Ok(())
}

#[tauri::command]
async fn sync_lan_peer_at(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    peer_public_key: String,
    addr: String,
) -> Result<list2cart_core::session::SessionOutcome, String> {
    let parsed: std::net::SocketAddr = addr
        .trim()
        .parse()
        .map_err(|e| format!("Neispravna adresa partnera (IP:port): {e}"))?;
    let svc = {
        let guard = state
            .lan
            .lock()
            .map_err(|_| "Greska zakljucavanja LAN servisa".to_owned())?;
        guard
            .as_ref()
            .cloned()
            .ok_or_else(|| "LAN servis nije pokrenut".to_owned())?
    };
    let res = svc.sync_peer_at(&peer_public_key, parsed).await;
    emit_lan_status(&app, &state);
    res
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
    if applied > 0 {
        state.notify_lan_change();
    }
    Ok(report)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Opens https links (a retailer's price list, Ko-fi) in the system browser.
        .plugin(tauri_plugin_opener::init())
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

            let export_dir = app
                .path()
                .download_dir()
                .unwrap_or_else(|_| data_dir.clone());
            let db_arc = Arc::new(Mutex::new(db));

            app.manage(AppState {
                db: Arc::clone(&db_arc),
                retail: list2cart_core::retail::RetailSearch::default(),
                pairing: list2cart_core::pairing_session::PairingController::new(),
                lan: Mutex::new(None),
                data_dir: data_dir.clone(),
                export_dir,
            });
            let lan_handle = app.handle().clone();
            let lan_db = Arc::clone(&db_arc);
            let lan_data_dir = data_dir;
            tauri::async_runtime::spawn(async move {
                match spawn_lan_service(lan_db, lan_data_dir).await {
                    Ok(svc) => {
                        let state = lan_handle.state::<AppState>();
                        if let Ok(mut guard) = state.lan.lock() {
                            *guard = Some(svc);
                        }
                        emit_lan_status(&lan_handle, &state);
                    }
                    Err(err) => eprintln!("LAN servis se nije mogao pokrenuti: {err}"),
                }
            });
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                loop {
                    let worker = handle.clone();
                    let result = tauri::async_runtime::spawn_blocking(move || {
                        let state = worker.state::<AppState>();
                        let db = state
                            .db
                            .lock()
                            .map_err(|_| "Greska zakljucavanja baze".to_owned())?;
                        db.run_scheduled_maintenance()
                    })
                    .await;
                    match result {
                        Ok(Ok(_)) => {}
                        Ok(Err(error)) => eprintln!("Odrzavanje baze nije uspjelo: {error}"),
                        Err(error) => eprintln!("Radnik odrzavanja nije uspio: {error}"),
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            retail_sources,
            retail_search,
            retail_all_sources,
            retail_home_branches,
            set_retail_home_branches,
            remove_retail_home_branch,
            retail_search_home,
            inspect_price_csv,
            read_price_csv,
            identity_conflicts,
            resolve_identity_conflict,
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
            incomplete_receipt_count,
            export_receipt_xlsx,
            export_list_xlsx,
            export_receipts_xlsx,
            list_shopping_lists,
            create_shopping_list,
            set_list_shared,
            delete_shopping_list,
            add_planned_item,
            toggle_planned_item,
            remove_planned_item,
            lookup_barcode_online,
            save_pocketbase_url,
            get_pocketbase_url,
            sync_pocketbase,
            export_sync_bundle_json,
            save_sync_bundle_file,
            import_sync_bundle_json,
            get_sync_identity,
            create_sync_request,
            create_sync_reply,
            receive_sync_page,
            get_pair_status,
            start_pair_listening,
            stop_pair_listening,
            dial_pair_peer,
            confirm_pair_peer,
            get_lan_status,
            start_lan_sync,
            stop_lan_sync,
            set_lan_paused,
            discover_lan_peers,
            trigger_lan_sync,
            set_manual_peer_addr,
            sync_lan_peer_at,
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
