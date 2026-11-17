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

export function formatCents(cents: number): string {
  return (cents / 100).toFixed(2).replace(".", ",") + " €";
}

export function calcLineCents(qty: number, unit: Unit, priceCents: number): number {
  if (qty <= 0 || priceCents <= 0) return 0;
  const eff = unit === "g" || unit === "ml" ? qty / 1000 : qty;
  return Math.round(eff * priceCents);
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

export async function exportSyncBundleJson(role: PeerRole = "own_device"): Promise<string> {
  return invoke<string>("export_sync_bundle_json", { role });
}

export async function importSyncBundleJson(jsonStr: string): Promise<[number, number, number]> {
  return invoke<[number, number, number]>("import_sync_bundle_json", { jsonStr });
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
