export type Unit = "kom" | "kg" | "g" | "l" | "ml";

export interface Store {
  id: number;
  sync_id: string;
  name: string;
  location: string;
  color: string;
  item_count: number;
  updated_at: string;
}

export interface Product {
  barcode: string;
  sync_id: string;
  name: string;
  category: string;
  default_unit: Unit;
  last_price_cents: number;
  last_store_name: string;
  image_data_url?: string | null;
  updated_at: string;
}

export interface StorePrice {
  store_name: string;
  price_cents: number;
  updated_at: string;
  is_cheapest: boolean;
}

export interface CartItem {
  id: string;
  barcode: string;
  name: string;
  quantity: number;
  unit: Unit;
  unit_price_cents: number;
  total_cents: number;
  image_data_url?: string | null;
}

export interface ReceiptVerification {
  cart_total_cents: number;
  receipt_total_cents: number;
  difference_cents: number;
  is_matched: boolean;
}

export interface ReceiptSummary {
  id: number;
  sync_id: string;
  store_id: number;
  store_name: string;
  created_at: string;
  items: CartItem[];
  cart_total_cents: number;
  receipt_total_cents: number;
  difference_cents: number;
}

export interface PlannedItem {
  id: number;
  sync_id: string;
  list_id: number;
  barcode?: string | null;
  name: string;
  quantity: number;
  unit: Unit;
  estimated_price_cents: number;
  checked: boolean;
  updated_at: string;
}

export interface ShoppingList {
  id: number;
  sync_id: string;
  name: string;
  store_id?: number | null;
  store_name?: string | null;
  items: PlannedItem[];
  total_estimated_cents: number;
  updated_at: string;
  /** Seen and edited by every member of the network, family members included. */
  shared?: boolean;
}

export interface OnlineProductInfo {
  barcode: string;
  name: string;
  brand?: string | null;
  image_url?: string | null;
  source: string;
}

export interface SyncReport {
  pushed_stores: number;
  pushed_products: number;
  pushed_receipts: number;
  pulled_stores: number;
  pulled_products: number;
  synced_at: string;
}

export type PeerRole = "own_device" | "family_member";

export interface NetworkMember {
  peer_id: string;
  name: string;
  role: PeerRole;
  sas_code: string;
  channels: string;
  shared_lists: string;
  added_at: string;
}

export interface LedgerStats {
  node_id: string;
  network_name: string;
  total_versions: number;
  unique_records: number;
  tombstones: number;
  frontier_nodes: number;
  db_size_bytes: number;
}
