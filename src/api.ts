import { invoke } from "@tauri-apps/api/core";
import type {
  CartItem,
  LedgerStats,
  NetworkMember,
  OnlineProductInfo,
  PeerRole,
  PlannedItem,
  Product,
  ReceiptSummary,
  ReceiptVerification,
  ShoppingList,
  Store,
  StorePrice,
  SyncReport,
  Unit,
} from "./types";

const isTauri = () => typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export interface PriceSource {chain:string;label:string;url:string;price_date:string;}
export interface RetailProduct {name:string;barcode:string|null;brand:string;package:string;price_cents:number;}
export interface RetailResults {source:PriceSource;fetched_at:string;products:RetailProduct[];total_matches:number;}
export interface CsvMapping {delimiter:number;name:number;price:number;barcode:number|null;brand:number|null;quantity:number|null;unit:number|null;}
export interface CsvInspection {headers:string[];sample:string[][];delimiter:number;suggested:CsvMapping|null;}
export async function inspectPriceCsv(bytes:number[]):Promise<CsvInspection> {
  return invoke<CsvInspection>("inspect_price_csv",{bytes});
}
export async function readPriceCsv(bytes:number[],mapping:CsvMapping):Promise<RetailProduct[]> {
  return invoke<RetailProduct[]>("read_price_csv",{bytes,mapping});
}
export async function retailSources(chain:string):Promise<PriceSource[]> {
  return invoke<PriceSource[]>("retail_sources",{chain});
}
export async function retailSearch(sourceUrl:string,query:string):Promise<RetailResults> {
  return invoke<RetailResults>("retail_search",{sourceUrl,query});
}

export interface IdentityConflict {
  entity: "stores" | "products";
  value: string;
  sync_ids: string[];
}

export async function identityConflicts(): Promise<IdentityConflict[]> {
  if (!isTauri()) return [];
  return invoke<IdentityConflict[]>("identity_conflicts");
}

export async function resolveIdentityConflict(group: IdentityConflict, keepId: string): Promise<void> {
  return invoke<void>("resolve_identity_conflict", { entity: group.entity, keepId, expectedIds: group.sync_ids });
}

export function formatCents(cents: number): string {
  if (!Number.isSafeInteger(cents)) return "—";
  const value = BigInt(cents < 0 ? -cents : cents);
  return `${cents < 0 ? "-" : ""}${value / 100n},${(value % 100n).toString().padStart(2, "0")} €`;
}

export function calcLineCents(qty: number, unit: Unit, priceCents: number): number {
  const millis = Math.round(qty * 1000);
  if (!Number.isFinite(qty) || millis <= 0 || millis > 1_000_000_000_000
    || Math.abs(millis / 1000 - qty) > 0.0000001
    || !Number.isSafeInteger(priceCents) || priceCents < 0) return 0;
  const denominator = unit === "g" || unit === "ml" ? 1_000_000n : 1000n;
  const cents = (BigInt(millis) * BigInt(priceCents) + denominator / 2n) / denominator;
  return cents <= BigInt(Number.MAX_SAFE_INTEGER) ? Number(cents) : 0;
}

export async function listStores(): Promise<Store[]> {
  if (!isTauri()) {
    return [
      { id: 1, sync_id: "s1", name: "Super Konzum", location: "Zagreb", color: "#DC2626", item_count: 12, updated_at: "" },
      { id: 2, sync_id: "s2", name: "Kaufland", location: "Zagreb", color: "#1768D1", item_count: 8, updated_at: "" },
      { id: 3, sync_id: "s3", name: "Lidl", location: "Zagreb", color: "#D97706", item_count: 5, updated_at: "" },
    ];
  }
  return invoke<Store[]>("list_stores");
}

export async function addStore(name: string, location: string, color: string): Promise<Store> {
  return invoke<Store>("add_store", { name, location, color });
}

export async function listProducts(search?: string): Promise<Product[]> {
  if (!isTauri()) return [];
  return invoke<Product[]>("list_products", { search: search || null });
}

export async function getProduct(barcode: string): Promise<Product | null> {
  if (!isTauri()) return null;
  return invoke<Product | null>("get_product", { barcode });
}

export async function getProductStorePrices(barcode: string): Promise<StorePrice[]> {
  if (!isTauri()) return [];
  return invoke<StorePrice[]>("get_product_store_prices", { barcode });
}

export async function upsertProduct(params: {
  barcode: string;
  name: string;
  category: string;
  defaultUnit: Unit;
  priceCents: number;
  storeName: string;
  imageDataUrl?: string | null;
}): Promise<Product> {
  return invoke<Product>("upsert_product", {
    barcode: params.barcode,
    name: params.name,
    category: params.category,
    defaultUnit: params.defaultUnit,
    priceCents: params.priceCents,
    storeName: params.storeName,
    imageDataUrl: params.imageDataUrl ?? null,
  });
}

export async function verifyReceipt(
  cartTotalCents: number,
  receiptTotalCents: number,
): Promise<ReceiptVerification> {
  if (!isTauri()) {
    const diff = cartTotalCents - receiptTotalCents;
    return { cart_total_cents: cartTotalCents, receipt_total_cents: receiptTotalCents, difference_cents: diff, is_matched: diff === 0 };
  }
  return invoke<ReceiptVerification>("verify_receipt", { cartTotalCents, receiptTotalCents });
}

export async function saveReceipt(
  storeId: number,
  storeName: string,
  items: CartItem[],
  receiptTotalCents: number,
): Promise<ReceiptSummary> {
  return invoke<ReceiptSummary>("save_receipt", {
    storeId,
    storeName,
    items,
    receiptTotalCents,
  });
}

export async function listReceipts(): Promise<ReceiptSummary[]> {
  if (!isTauri()) return [];
  return invoke<ReceiptSummary[]>("list_receipts");
}

export async function incompleteReceiptCount(): Promise<number> {
  if (!isTauri()) return 0;
  return invoke<number>("incomplete_receipt_count");
}

export async function exportReceiptXlsx(receipt: ReceiptSummary): Promise<string> {
  return invoke<string>("export_receipt_xlsx", { receipt });
}

export async function listShoppingLists(): Promise<ShoppingList[]> {
  if (!isTauri()) return [];
  return invoke<ShoppingList[]>("list_shopping_lists");
}

export async function createShoppingList(name: string, storeId?: number | null): Promise<ShoppingList> {
  return invoke<ShoppingList>("create_shopping_list", { name, storeId: storeId ?? null });
}

export async function addPlannedItem(params: {
  listId: number;
  barcode?: string | null;
  name: string;
  quantity: number;
  unit: Unit;
  estimatedPriceCents: number;
}): Promise<PlannedItem> {
  return invoke<PlannedItem>("add_planned_item", {
    listId: params.listId,
    barcode: params.barcode ?? null,
    name: params.name,
    quantity: params.quantity,
    unit: params.unit,
    estimatedPriceCents: params.estimatedPriceCents,
  });
}

export async function togglePlannedItem(itemId: number, checked: boolean): Promise<void> {
  return invoke<void>("toggle_planned_item", { itemId, checked });
}

export async function lookupBarcodeOnline(barcode: string): Promise<OnlineProductInfo | null> {
  return invoke<OnlineProductInfo | null>("lookup_barcode_online", { barcode });
}

export async function getPocketbaseUrl(): Promise<string | null> {
  if (!isTauri()) return null;
  return invoke<string | null>("get_pocketbase_url");
}

export async function savePocketbaseUrl(url: string): Promise<void> {
  return invoke<void>("save_pocketbase_url", { url });
}

export async function syncPocketbase(url: string, authToken?: string | null): Promise<SyncReport> {
  return invoke<SyncReport>("sync_pocketbase", { url, authToken: authToken ?? null });
}

export async function exportSyncBundleJson(
  role: PeerRole = "own_device",
  sharedLists?: string | null,
): Promise<string> {
  return invoke<string>("export_sync_bundle_json", {
    role,
    sharedLists: sharedLists ?? null,
  });
}

export async function importSyncBundleJson(jsonStr: string): Promise<[number, number, number]> {
  return invoke<[number, number, number]>("import_sync_bundle_json", { jsonStr });
}

export async function saveSyncPackage(json: string, familyOnly: boolean): Promise<string | null> {
  if (!isTauri()) return null;
  return invoke<string>("save_sync_bundle_file", {jsonStr:json,familyOnly});
}

export async function getLedgerStats(): Promise<LedgerStats> {
  if (!isTauri()) {
    return {
      node_id: "l2c-dev01",
      network_name: "Obiteljska mreža",
      total_versions: 8,
      unique_records: 5,
      tombstones: 0,
      frontier_nodes: 1,
      db_size_bytes: 32768,
    };
  }
  return invoke<LedgerStats>("get_ledger_stats");
}

export async function runLedgerCompact(): Promise<number> {
  if (!isTauri()) return 0;
  return invoke<number>("run_ledger_compact");
}

export async function setNetworkName(name: string): Promise<string> {
  if (!isTauri()) return name;
  return invoke<string>("set_network_name", { name });
}

export async function newSasCode(): Promise<string> {
  if (!isTauri()) return "482-915";
  return invoke<string>("new_sas_code");
}

export async function listNetworkMembers(): Promise<NetworkMember[]> {
  if (!isTauri()) return [];
  return invoke<NetworkMember[]>("list_network_members");
}

export async function addNetworkMember(params: {
  name: string;
  role: PeerRole;
  sasCode: string;
  sharedLists: string;
}): Promise<NetworkMember> {
  return invoke<NetworkMember>("add_network_member", {
    name: params.name,
    role: params.role,
    sasCode: params.sasCode,
    sharedLists: params.sharedLists,
  });
}

export async function removeNetworkMember(peerId: string): Promise<void> {
  return invoke<void>("remove_network_member", { peerId });
}
