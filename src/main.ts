import {
  Format,
  cancel as cancelNativeBarcodeScan,
  checkPermissions as checkBarcodePermissions,
  requestPermissions as requestBarcodePermissions,
  scan as scanNativeBarcode,
} from "@tauri-apps/plugin-barcode-scanner";
import {
  addNetworkMember,
  retailSources,
  retailSearch,
  inspectPriceCsv,
  readPriceCsv,
  saveSyncPackage,
  incompleteReceiptCount,
  identityConflicts,
  resolveIdentityConflict,
  addPlannedItem,
  addStore,
  calcLineCents,
  confirmPairPeer,
  createShoppingList,
  dialPairPeer,
  discoverLanPeers,
  exportReceiptXlsx,
  exportSyncBundleJson,
  formatCents,
  getLanStatus,
  getLedgerStats,
  getPairStatus,
  getPocketbaseUrl,
  getProduct,
  getProductStorePrices,
  importSyncBundleJson,
  listenLanStatusChanged,
  listNetworkMembers,
  listProducts,
  listReceipts,
  listShoppingLists,
  listStores,
  lookupBarcodeOnline,
  newSasCode,
  removeNetworkMember,
  runLedgerCompact,
  savePocketbaseUrl,
  saveReceipt,
  setLanPaused,
  setManualPeerAddr,
  setNetworkName,
  startLanSync,
  startPairListening,
  stopLanSync,
  stopPairListening,
  syncLanPeerAt,
  syncPocketbase,
  togglePlannedItem,
  triggerLanSync,
  upsertProduct,
  verifyReceipt,
} from "./api";
import type {
  FoundLanPeer,
  IdentityConflict,
  LanServiceStatus,
  PairStatusView,
  PeerSyncRecord,
} from "./api";
import type { PriceSource, RetailResults, RetailProduct, CsvInspection, CsvMapping } from "./api";
import { SyncTracker } from "./sync-progress";
import { compareBasket, formatDate, formatQuantity, resolveBasketProduct, sharedListNames } from "./shopping";
import type { BasketComparison } from "./shopping";
import { icon } from "./icons";

const syncTracker = new SyncTracker(() => render());
const retail = {sources:[] as PriceSource[],results:null as RetailResults|null,query:"",branch:"",busy:false,error:"",storeId:0};
const localPriceCsv = {bytes:[] as number[],inspection:null as CsvInspection|null,filename:"",products:null as RetailProduct[]|null,storeId:0};
import type {
  CartItem,
  LedgerStats,
  NetworkMember,
  PeerRole,
  Product,
  ReceiptSummary,
  ShoppingList,
  Store,
  StorePrice,
  Unit,
} from "./types";

type Tab = "pregled" | "kosarica" | "planer" | "racuni" | "katalog" | "cijene" | "trgovine" | "sync" | "napredno";

interface BarcodeDetectorLike {
  detect(source: ImageBitmapSource): Promise<Array<{ rawValue?: string }>>;
}

declare global {
  interface Window {
    BarcodeDetector?: new (options?: { formats?: string[] }) => BarcodeDetectorLike;
  }
}

export function escapeHtml(raw: unknown): string {
  const str = String(raw ?? "");
  return str
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

function sanitizeImageUrl(url: string | null | undefined): string | null {
  if (!url) return null;
  const trimmed = url.trim();
  if (/^data:image\/(png|jpeg|jpg|webp|gif);base64,[A-Za-z0-9+/=\s]+$/i.test(trimmed)) {
    return trimmed;
  }
  if (/^https:\/\/[^\s"'<>]+$/i.test(trimmed)) {
    return trimmed;
  }
  return null;
}

function sanitizeColor(color: string | null | undefined): string {
  if (color && /^#[0-9a-fA-F]{3,8}$/.test(color.trim())) {
    return color.trim();
  }
  return "#1768D1";
}

interface AppModel {
  incompleteReceipts: number;
  conflicts: IdentityConflict[];
  conflictReview: { group: IdentityConflict; keepId: string } | null;
  resolvingConflict: boolean;
  tab: Tab;
  stores: Store[];
  selectedStoreId: number;
  cart: CartItem[];
  receiptTotalInput: string;
  products: Product[];
  receipts: ReceiptSummary[];
  lists: ShoppingList[];
  pocketbaseUrl: string;
  toast: string | null;
  productSearch: string;
  draftPhotoDataUrl: string | null;
  ledgerStats: LedgerStats | null;
  members: NetworkMember[];
  pendingSasCode: string;
  pairingRole: PeerRole;
  scannerOpen: boolean;
  selectedProduct: Product | null;
  selectedProductPrices: StorePrice[];
  draftBarcode: string;
  draftName: string;
  draftQty: string;
  draftUnit: Unit;
  draftPrice: string;
  draftCatBarcode: string;
  draftCatName: string;
  draftCatPrice: string;
  draftNewList: string;
  draftPlanName: string;
  draftPlanQty: string;
  draftPlanUnit: Unit;
  draftPlanPrice: string;
  draftStoreName: string;
  draftStoreLoc: string;
  draftNetworkName: string | null;
  draftMemberName: string;
  draftMemberLists: string | null;
  draftModalBarcode: string;
  selectedListId: number | null;
  newListOpen: boolean;
  selectedReceipt: ReceiptSummary | null;
  receiptSearch: string;
  receiptStore: string;
  pairingOpen: boolean;
  pairingConfirmed: boolean;
  pairingBusy: boolean;
  pairMode: "lan" | "offline";
  pairStatus: PairStatusView | null;
  draftPairAddr: string;
  draftDeviceName: string;
  lanStatus: LanServiceStatus | null;
  lanPeersDiscovered: FoundLanPeer[];
  lanBusy: boolean;
  draftManualAddrs: Record<string, string>;
  accessMember: NetworkMember | null;
  removalMember: NetworkMember | null;
  comparisonSource: string;
  comparisons: BasketComparison[];
  comparisonBusy: boolean;
  comparisonError: string;
  comparisonLoaded: boolean;
  retailOpen: boolean;
  exportListIds: string[];
}

const model: AppModel = {
  incompleteReceipts: 0,
  conflicts: [],
  conflictReview: null,
  resolvingConflict: false,
  tab: window.matchMedia("(max-width: 768px)").matches ? "planer" : "pregled",
  stores: [],
  selectedStoreId: 1,
  cart: [],
  receiptTotalInput: "",
  products: [],
  receipts: [],
  lists: [],
  pocketbaseUrl: "http://unraid.local:8090",
  toast: null,
  productSearch: "",
  draftPhotoDataUrl: null,
  ledgerStats: null,
  members: [],
  pendingSasCode: "482-915",
  pairingRole: "family_member",
  scannerOpen: false,
  selectedProduct: null,
  selectedProductPrices: [],
  draftBarcode: "",
  draftName: "",
  draftQty: "1",
  draftUnit: "kom",
  draftPrice: "",
  draftCatBarcode: "",
  draftCatName: "",
  draftCatPrice: "",
  draftNewList: "",
  draftPlanName: "",
  draftPlanQty: "1",
  draftPlanUnit: "kom",
  draftPlanPrice: "",
  draftStoreName: "",
  draftStoreLoc: "",
  draftNetworkName: null,
  draftMemberName: "",
  draftMemberLists: null,
  draftModalBarcode: "",
  selectedListId: null,
  newListOpen: false,
  selectedReceipt: null,
  receiptSearch: "",
  receiptStore: "",
  pairingOpen: false,
  pairingConfirmed: false,
  pairingBusy: false,
  pairMode: "lan",
  pairStatus: null,
  draftPairAddr: "",
  draftDeviceName: "",
  lanStatus: null,
  lanPeersDiscovered: [],
  lanBusy: false,
  draftManualAddrs: {},
  accessMember: null,
  removalMember: null,
  comparisonSource: "cart",
  comparisons: [],
  comparisonBusy: false,
  comparisonError: "",
  comparisonLoaded: false,
  retailOpen: false,
  exportListIds: [],
};

let scannerDecodeTimer: number | null = null;
let pairPollTimer: number | null = null;
let nativeScannerActive = false;

function renderToastOnly(): void {
  const container = document.getElementById("toast-container");
  if (!container) return;
  container.innerHTML = model.toast ? `<div class="toast">${escapeHtml(model.toast)}</div>` : "";
}

function showToast(msg: string) {
  model.toast = msg;
  if (document.getElementById("toast-container")) {
    renderToastOnly();
  } else {
    render();
  }
  setTimeout(() => {
    if (model.toast === msg) {
      model.toast = null;
      renderToastOnly();
    }
  }, 3200);
}

function activeStore(): Store | undefined {
  return model.stores.find((s) => s.id === model.selectedStoreId) ?? model.stores[0];
}

function cartTotalCents(): number {
  return model.cart.reduce((acc, i) => acc + i.total_cents, 0);
}

function parseEuroToCents(raw: string): number {
  const normalized = raw.replace(",", ".").replace(/[^0-9.]/g, "");
  const val = Number.parseFloat(normalized);
  return Number.isFinite(val) && val > 0 ? Math.round(val * 100) : 0;
}

const APP_ICON_SVG = `
<svg class="brand-logo" viewBox="0 0 128 128" fill="none" xmlns="http://www.w3.org/2000/svg">
  <defs>
    <linearGradient id="bgGrad" x1="16" y1="8" x2="112" y2="120" gradientUnits="userSpaceOnUse">
      <stop offset="0%" stop-color="#2D8CFF"/>
      <stop offset="55%" stop-color="#1768D1"/>
      <stop offset="100%" stop-color="#0E4999"/>
    </linearGradient>
  </defs>
  <rect x="4" y="4" width="120" height="120" rx="28" fill="url(#bgGrad)"/>
  <path d="M24 34H34.5C36.6 34 38.4 35.4 38.9 37.5L47.2 74.5C47.7 76.6 49.5 78 51.6 78H89.5C91.6 78 93.4 76.6 93.9 74.6L100.8 46.8C101.4 44.3 99.5 42 97 42H41" stroke="#FFFFFF" stroke-width="7" stroke-linecap="round" stroke-linejoin="round"/>
  <circle cx="54" cy="95" r="6.5" fill="#FFFFFF"/>
  <circle cx="88" cy="95" r="6.5" fill="#FFFFFF"/>
  <path d="M56 58L65 67L84 48" stroke="#60BD68" stroke-width="7.5" stroke-linecap="round" stroke-linejoin="round"/>
  <path d="M56 58L65 67L84 48" stroke="#FFFFFF" stroke-width="4" stroke-linecap="round" stroke-linejoin="round"/>
</svg>`;

const NAV_ITEMS: { id: Exclude<Tab, "napredno">; label: string }[] = [
  { id: "pregled", label: "Pregled" },
  { id: "planer", label: "Popisi" },
  { id: "kosarica", label: "Kupnja" },
  { id: "racuni", label: "Računi" },
  { id: "katalog", label: "Katalog" },
  { id: "cijene", label: "Cijene" },
  { id: "trgovine", label: "Trgovine" },
  { id: "sync", label: "Obitelj i uređaji" },
];

function renderSidebar(): string {
  const navButton = (id: Exclude<Tab, "napredno">, label: string, mobile = false) => {
    const active = model.tab === id || (id === "sync" && model.tab === "napredno");
    return `<button class="nav-btn ${active ? "active" : ""}" data-tab="${id}" ${active ? 'aria-current="page"' : ""}>
      ${icon(id)}<span>${escapeHtml(mobile && id === "sync" ? "Obitelj" : label)}</span></button>`;
  };
  return `<aside class="sidebar no-print">
    <div class="brand-box">${APP_ICON_SVG}<div><div class="brand-title">List2Cart</div><div class="brand-tag">Privatna kupnja</div></div></div>
    <nav class="nav-list" aria-label="Glavna navigacija">${NAV_ITEMS.map(n => navButton(n.id, n.label)).join("")}</nav>
    <div class="sidebar-footer"><strong>Lokalno spremljeno</strong><p>Razmjena samo s odabranim uređajima.</p></div>
  </aside>
  <nav class="mobile-nav no-print" aria-label="Mobilna navigacija">
    ${NAV_ITEMS.filter(n => ["planer", "kosarica", "racuni", "sync"].includes(n.id)).map(n => navButton(n.id, n.label, true)).join("")}
  </nav>`;
}

function pageHeader(title: string, description: string, action = ""): string {
  const secondary = ["pregled", "katalog", "cijene", "trgovine", "napredno"].includes(model.tab);
  return `<header class="page-header">
    <div>${secondary ? `<button class="back-link mobile-only no-print" data-goto="sync">${icon("back")} Obitelj i više</button>` : ""}
      <h1>${title}</h1>${description ? `<p>${description}</p>` : ""}</div>${action}</header>`;
}

function currentList(): ShoppingList | undefined {
  return model.lists.find(l => l.id === model.selectedListId) ?? model.lists[0];
}

function renderPregled(): string {
  const total = cartTotalCents();
  const store = activeStore();
  const lastReceipt = model.receipts[0];
  const hasReceipt = model.receiptTotalInput.trim().length > 0;
  const diff = total - parseEuroToCents(model.receiptTotalInput);
  return `${pageHeader("Vaša kupnja na jednom mjestu", "Popisi, košarica i spremljeni računi — na vašim uređajima.",
    '<button class="btn btn-outline" data-goto="kosarica">Otvori kupnju</button>')}
    <section class="card overview-card">
      <div class="item-sub">${store ? escapeHtml(store.name) : "Odaberite trgovinu"} · Aktivna košarica</div>
      <div class="big-stat">${escapeHtml(formatCents(total))} u košarici</div>
      <div class="item-sub">${model.cart.length} stavki u ovoj kupnji</div>
      <div class="highlight-box">
        <span class="pill">${hasReceipt && model.cart.length ? "USPOREDBA S RAČUNOM" : "VAŠA SLJEDEĆA KUPNJA"}</span>
        <h2>${hasReceipt && model.cart.length ? (diff === 0 ? "Zbroj se podudara · razlika 0,00 €" : `Razlika unesenih iznosa: ${escapeHtml(formatCents(diff))}`) : "Pripremite popis i krenite u kupnju"}</h2>
        <p class="item-sub">${hasReceipt && model.cart.length ? "Usporedba se odnosi na unesene iznose." : "Skenirajte artikle ili ih dodajte ručno, a na blagajni usporedite zbroj."}</p>
        <div><button class="btn btn-primary" data-goto="kosarica">Otvori košaricu →</button></div>
      </div>
    </section>
    <h2 class="section-title">Što još možete učiniti</h2>
    <div class="grid-2">
      <button class="card action-card" data-goto="planer"><strong>Pripremi sljedeći popis →</strong><span class="item-sub">${model.lists.length} popisa za vašu sljedeću kupnju.</span></button>
      <button class="card action-card" data-goto="racuni"><strong>Pregledaj spremljene račune →</strong><span class="item-sub">${lastReceipt ? `${escapeHtml(lastReceipt.store_name)} · ${escapeHtml(formatCents(lastReceipt.receipt_total_cents))} · ${escapeHtml(formatDate(lastReceipt.created_at))}` : "Računi i njihove stavke čuvaju se na uređaju."}</span></button>
    </div>`;
}

function unitOptions(unit: Unit): string {
  return (["kom", "kg", "g", "l", "ml"] as Unit[]).map(u => `<option value="${u}" ${unit === u ? "selected" : ""}>${u === "l" ? "L" : u}</option>`).join("");
}

function renderKosarica(): string {
  const total = cartTotalCents();
  const diff = total - parseEuroToCents(model.receiptTotalInput);
  const hasReceipt = model.receiptTotalInput.trim().length > 0;
  const storeSelect = `<select id="cart-store-select" aria-label="Trgovina za kupnju">${model.stores.length ? model.stores.map(s => `<option value="${s.id}" ${s.id === model.selectedStoreId ? "selected" : ""}>${escapeHtml(s.name)}</option>`).join("") : '<option value="">Dodajte trgovinu</option>'}</select>`;
  return `${pageHeader("Košarica", "Dodajte artikle i na blagajni usporedite unesene iznose.", storeSelect)}
    <section class="card cart-entry no-print" aria-label="Dodaj artikl u košaricu">
      <button class="btn btn-primary scan-button" id="btn-camera-scan">${icon("scan")} Skeniraj barkod</button>
      <div class="cart-inputs">
        <label class="field field-barcode">Barkod<input id="inp-barcode" value="${escapeHtml(model.draftBarcode)}" placeholder="EAN barkod" inputmode="numeric" /></label>
        <label class="field field-name">Naziv artikla<input id="inp-name" value="${escapeHtml(model.draftName)}" placeholder="Naziv artikla" /></label>
        <label class="field field-qty">Količina<input id="inp-qty" type="number" min="0.001" step="0.001" value="${escapeHtml(model.draftQty)}" /></label>
        <label class="field field-unit">Jedinica<select id="inp-unit">${unitOptions(model.draftUnit)}</select></label>
        <label class="field field-price">Cijena (€)<input id="inp-price" inputmode="decimal" value="${escapeHtml(model.draftPrice)}" placeholder="0,00" /></label>
      </div>
      <button class="btn btn-outline add-cart-button" id="btn-add-cart">${icon("plus")} Dodaj u košaricu</button>
      <div class="entry-tools"><button class="text-button" id="btn-online-lookup">Dohvati naziv po barkodu</button><button class="text-button" data-toggle-retail>Pretraži cjenik trgovine</button></div>
    </section>
    <div class="cart-layout">
      <section class="card basket-items" aria-label="Stavke u košarici">
        ${model.cart.length === 0 ? `<div class="empty-state">${icon("kosarica")}<h2>Košarica je prazna</h2><p>Skenirajte barkod ili unesite prvi artikl.</p>${!model.stores.length ? '<button class="btn btn-outline" data-goto="trgovine">Dodaj trgovinu</button>' : ""}</div>` : model.cart.map(item => {
          const img = sanitizeImageUrl(item.image_data_url);
          return `<div class="basket-row">
            ${img ? `<div class="item-thumb"><img src="${escapeHtml(img)}" alt="" /></div>` : ""}
            <div class="item-copy"><div class="item-title">${escapeHtml(item.name)}</div><div class="item-sub">${formatQuantity(item.quantity)} ${item.unit === "l" ? "L" : item.unit} × ${escapeHtml(formatCents(item.unit_price_cents))}${item.unit === "g" || item.unit === "kg" ? "/kg" : item.unit === "ml" || item.unit === "l" ? "/L" : ""}</div></div>
            <div class="item-price">${escapeHtml(formatCents(item.total_cents))}</div>
            <button class="icon-button btn-danger-ghost no-print" data-remove-cart="${escapeHtml(item.id)}" aria-label="Ukloni ${escapeHtml(item.name)}">${icon("close")}</button>
          </div>`;
        }).join("")}
      </section>
      <section class="card checkout-card" aria-label="Provjera računa">
        <div class="row space-between"><span class="item-sub">Ukupno u košarici</span><strong class="checkout-total">${escapeHtml(formatCents(total))}</strong></div>
        <label class="receipt-input"><span>Iznos računa</span><input id="inp-receipt-total" inputmode="decimal" value="${escapeHtml(model.receiptTotalInput)}" placeholder="0,00 €" /></label>
        ${hasReceipt ? `<div class="match-status ${diff === 0 ? "matched" : "unmatched"}" role="status">${diff === 0 ? "Zbroj se podudara · razlika 0,00 €" : `Razlika: ${escapeHtml(formatCents(diff))}`}</div>` : '<p class="item-sub">Unesite iznos s računa za usporedbu.</p>'}
        <button class="btn btn-primary no-print" id="btn-save-receipt" ${!model.cart.length || !model.stores.length ? "disabled" : ""}>Spremi račun</button>
        <p class="caption">Uspoređuju se uneseni iznosi.</p>
        <div class="row-wrap no-print"><button class="text-button" id="btn-print-receipt">Ispis</button><button class="text-button" id="btn-export-excel" ${model.cart.length ? "" : "disabled"}>Izvezi u Excel</button></div>
      </section>
    </div>`;
}

function renderPlaner(): string {
  const list = currentList();
  const items = list?.items ?? [];
  const checked = items.filter(i => i.checked).length;
  const remaining = items.filter(i => !i.checked);
  const lineTotal = (item: typeof items[number]) => calcLineCents(item.quantity, item.unit, item.estimated_price_cents);
  const total = items.reduce((sum, i) => sum + lineTotal(i), 0);
  const remainingTotal = remaining.reduce((sum, i) => sum + lineTotal(i), 0);
  return `${pageHeader("Popisi", "Pripremite kupnju i označite kupljene artikle.", `<div class="header-actions">${model.lists.length ? `<select id="active-list-select" aria-label="Aktivni popis">${model.lists.map(l => `<option value="${l.id}" ${l.id === list?.id ? "selected" : ""}>${escapeHtml(l.name)}</option>`).join("")}</select>` : ""}<button class="btn btn-outline" id="btn-new-list" aria-expanded="${model.newListOpen}">${icon("plus")} Novi popis</button></div>`)}
    ${model.newListOpen ? `<section class="card no-print" aria-label="Novi popis"><h2>Novi popis za kupnju</h2><div class="row-wrap">
      <label class="field grow">Naziv popisa<input id="inp-new-list" value="${escapeHtml(model.draftNewList)}" placeholder="Npr. Tjedna nabavka" /></label>
      <label class="field">Trgovina<select id="sel-list-store"><option value="">Sve trgovine</option>${model.stores.map(s => `<option value="${s.id}" ${s.id === model.selectedStoreId ? "selected" : ""}>${escapeHtml(s.name)}</option>`).join("")}</select></label>
      <button class="btn btn-primary" id="btn-create-list">Kreiraj popis</button></div></section>` : ""}
    ${list ? `<section class="card plan-add no-print"><div class="row"><input id="inp-plan-name" aria-label="Artikl za popis" value="${escapeHtml(model.draftPlanName)}" placeholder="Dodaj artikl na popis…" /><button class="btn btn-primary" id="btn-add-planned">Dodaj</button></div>
      <details class="plan-details" ${model.draftPlanPrice || model.draftPlanQty !== "1" || model.draftPlanUnit !== "kom" ? "open" : ""}><summary>Količina i procjena cijene</summary><div class="row-wrap">
        <label class="field grow">Količina<input id="inp-plan-qty" type="number" min="0.001" step="0.001" value="${escapeHtml(model.draftPlanQty)}" /></label>
        <label class="field grow">Jedinica<select id="inp-plan-unit">${unitOptions(model.draftPlanUnit)}</select></label>
        <label class="field grow">Cijena (€)<input id="inp-plan-price" inputmode="decimal" value="${escapeHtml(model.draftPlanPrice)}" placeholder="0,00" /></label>
      </div></details></section>
      <section class="card plan-progress"><div class="row space-between"><span class="item-sub">${checked} od ${items.length} kupljeno</span><strong class="item-price">${escapeHtml(formatCents(total))}</strong></div><div class="progress-track" role="progressbar" aria-label="Kupljeni artikli" aria-valuemin="0" aria-valuemax="${Math.max(items.length, 1)}" aria-valuenow="${checked}"><div class="progress-fill" style="width:${items.length ? checked / items.length * 100 : 0}%"></div></div></section>
      <div class="plan-layout"><section class="card plan-items" aria-label="Stavke popisa ${escapeHtml(list.name)}">
        ${items.length ? items.map(item => `<label class="plan-row ${item.checked ? "is-checked" : ""}"><input type="checkbox" data-toggle-plan="${item.id}" ${item.checked ? "checked" : ""} /><span class="item-copy"><span class="item-title">${escapeHtml(item.name)}</span><span class="item-sub">${formatQuantity(item.quantity)} ${item.unit === "l" ? "L" : item.unit}</span></span><span class="item-price">${escapeHtml(formatCents(lineTotal(item)))}</span></label>`).join("") : '<div class="empty-state"><h2>Što trebate kupiti?</h2><p>Dodajte prvi artikl na ovaj popis.</p></div>'}
      </section><section class="card plan-summary"><div class="row space-between"><span class="item-sub">Preostalo · ${remaining.length} artikala</span><strong class="checkout-total">${escapeHtml(formatCents(remainingTotal))}</strong></div>
        <button class="btn btn-primary" id="btn-transfer-cart" ${remaining.length ? "" : "disabled"}>Kreni u kupnju</button><p class="item-sub">Preostali artikli dodaju se u košaricu s unesenim procjenama cijena.</p><button class="text-button" data-goto="cijene">Usporedi cijene popisa →</button><button class="text-button no-print" data-toggle-retail>Pronađi artikl u cjeniku</button>
      </section></div>` : `<section class="card empty-state">${icon("planer")}<h2>Vaš prvi popis za kupnju</h2><p>Odaberite Novi popis i zapišite što trebate.</p></section>`}`;
}

function renderReceipts(): string {
  const query = model.receiptSearch.trim().toLocaleLowerCase("hr");
  const receipts = model.receipts.filter(r => (!model.receiptStore || r.store_name === model.receiptStore)
    && `${r.store_name} ${r.items.map(i => i.name).join(" ")} ${formatDate(r.created_at)}`.toLocaleLowerCase("hr").includes(query));
  const total = receipts.reduce((sum, r) => sum + r.receipt_total_cents, 0);
  return `${pageHeader("Računi", "Povijest kupnje i spremljene stavke na jednom mjestu.")}
    <div class="receipt-filters no-print"><input id="receipt-search" aria-label="Pretraži račune" placeholder="Trgovina, artikl ili datum…" value="${escapeHtml(model.receiptSearch)}" /><select id="receipt-store" aria-label="Filtriraj račune po trgovini"><option value="">Sve trgovine</option>${[...new Set(model.receipts.map(r => r.store_name))].map(name => `<option ${model.receiptStore === name ? "selected" : ""} value="${escapeHtml(name)}">${escapeHtml(name)}</option>`).join("")}</select></div>
    <section class="card receipt-summary"><div><span class="item-sub">${receipts.length} spremljenih računa${query || model.receiptStore ? " u odabiru" : ""}</span><div class="big-stat">${escapeHtml(formatCents(total))}</div></div><p class="item-sub">Zbroj unesenih iznosa računa.</p></section>
    <div class="receipt-list">${receipts.length ? receipts.map(r => `<button class="card receipt-card" data-receipt-id="${r.id}"><div class="receipt-icon">${icon("racuni")}</div><div class="item-copy"><strong>${escapeHtml(r.store_name)}</strong><span class="item-sub">${escapeHtml(formatDate(r.created_at))} · ${r.items.length} stavki</span><span class="item-sub">${r.difference_cents === 0 ? "Zbroj se podudara" : `Razlika ${escapeHtml(formatCents(r.difference_cents))}`}</span></div><div class="receipt-amount"><strong>${escapeHtml(formatCents(r.receipt_total_cents))}</strong><span class="item-sub">Otvori račun →</span></div></button>`).join("") : `<section class="card empty-state">${icon("racuni")}<h2>${model.receipts.length ? "Nema računa za ovaj odabir" : "Još nema spremljenih računa"}</h2><p>${model.receipts.length ? "Promijenite pretragu ili trgovinu." : "Nakon kupnje spremite račun iz košarice."}</p><button class="btn btn-outline" data-goto="kosarica">Otvori kupnju</button></section>`}</div>`;
}

function renderReceiptDialog(): string {
  const r = model.selectedReceipt;
  if (!r) return "";
  return `<div class="modal-backdrop receipt-dialog"><section class="modal-card" role="dialog" aria-modal="true" aria-labelledby="receipt-title"><div class="row space-between no-print"><span class="pill">SPREMLJEN RAČUN</span><button class="icon-button" data-close-dialog aria-label="Zatvori račun">${icon("close")}</button></div><div><h2 id="receipt-title">${escapeHtml(r.store_name)}</h2><p class="item-sub">${escapeHtml(formatDate(r.created_at))} · ${r.items.length} stavki</p></div>
    <div class="receipt-lines">${r.items.map(i => `<div class="basket-row"><div class="item-copy"><strong>${escapeHtml(i.name)}</strong><span class="item-sub">${formatQuantity(i.quantity)} ${i.unit === "l" ? "L" : i.unit} × ${escapeHtml(formatCents(i.unit_price_cents))}${["g", "kg"].includes(i.unit) ? "/kg" : ["l", "ml"].includes(i.unit) ? "/L" : ""}</span></div><strong>${escapeHtml(formatCents(i.total_cents))}</strong></div>`).join("")}</div>
    <dl class="receipt-totals"><div><dt>Zbroj stavki</dt><dd>${escapeHtml(formatCents(r.cart_total_cents))}</dd></div><div><dt>Iznos računa</dt><dd>${escapeHtml(formatCents(r.receipt_total_cents))}</dd></div><div><dt>Razlika</dt><dd>${escapeHtml(formatCents(r.difference_cents))}</dd></div></dl>
    <p class="match-status ${r.difference_cents === 0 ? "matched" : "unmatched"}">${r.difference_cents === 0 ? "Zbroj se podudara" : "Zabilježena je razlika unesenih iznosa"}</p><p class="caption">Uspoređuju se uneseni iznosi.</p><div class="dialog-actions no-print"><button class="btn btn-outline" id="btn-print-saved">Ispis</button><button class="btn btn-primary" id="btn-export-saved">Izvezi u Excel</button></div></section></div>`;
}

function renderKatalog(): string {
  const selectedImg = sanitizeImageUrl(model.selectedProduct?.image_data_url);
  return `
    ${pageHeader("Katalog artikala", "Pretražite bazu barkodova, dodajte fotografiju proizvoda i pratite zadnje cijene.", '<button class="btn btn-outline no-print" data-toggle-retail>Javni cjenik trgovine</button>')}

    <div class="card">
      <div class="row-wrap" style="margin-bottom: 12px;">
        <input id="inp-cat-barcode" value="${escapeHtml(model.draftCatBarcode)}" placeholder="Barkod" style="width:140px;" />
        <input id="inp-cat-name" value="${escapeHtml(model.draftCatName)}" placeholder="Naziv artikla" style="flex:1;" />
        <input id="inp-cat-price" value="${escapeHtml(model.draftCatPrice)}" placeholder="Zadnja cijena (€)" style="width:120px;" />
        <label class="btn btn-outline" style="cursor:pointer;">
          📷 Uslikaj / Slika
          <input id="inp-cat-photo" type="file" accept="image/*" capture="environment" style="display:none;" />
        </label>
        <button class="btn btn-primary" id="btn-save-product">Spremi artikl →</button>
      </div>
      <input id="inp-cat-search" value="${escapeHtml(model.productSearch)}" placeholder="Pretraži po nazivu ili barkodu..." style="width:100%;" />
    </div>

    ${
      model.selectedProduct
        ? `
      <div class="card">
        <div class="row space-between" style="margin-bottom:10px;">
          <div class="row">
            <div class="item-thumb">
              ${
                selectedImg
                  ? `<img src="${escapeHtml(selectedImg)}" alt="" />`
                  : escapeHtml(model.selectedProduct.name.slice(0, 1).toUpperCase())
              }
            </div>
            <div>
              <span class="pill">DETALJ ARTIKLA I USPOREDBA CIJENA PO TRGOVINAMA</span>
              <div class="item-title" style="font-size:17px;margin-top:4px;">${escapeHtml(model.selectedProduct.name)} (${escapeHtml(model.selectedProduct.barcode)})</div>
            </div>
          </div>
          <button class="btn btn-outline" id="btn-close-detail">Zatvori detalj</button>
        </div>
        <div class="list-container">
          ${
            model.selectedProductPrices.length === 0
              ? `<div class="item-sub">Za ovaj artikl zabilježena je zadnja cijena: ${escapeHtml(formatCents(model.selectedProduct.last_price_cents))} (${escapeHtml(model.selectedProduct.last_store_name || "Trgovina")}).</div>`
              : model.selectedProductPrices
                  .map(
                    (sp) => `
              <div class="list-row">
                <div>
                  <div class="item-title">${escapeHtml(sp.store_name)}</div>
                  <div class="item-sub">Ažurirano: ${escapeHtml(sp.updated_at.slice(0, 10))}</div>
                </div>
                <div class="row">
                  ${sp.is_cheapest ? `<span class="pill success">★ NAJJEFTINIJE</span>` : ""}
                  <div class="item-price">${escapeHtml(formatCents(sp.price_cents))}</div>
                </div>
              </div>`,
                  )
                  .join("")
          }
        </div>
      </div>`
        : ""
    }

    <div class="list-container">
      ${
        model.products.length === 0
          ? `<div class="card item-sub">Nema pronađenih artikala. Dodajte proizvod skeniranjem ili unosom iznad.</div>`
          : model.products
              .map((p) => {
                const safeImg = sanitizeImageUrl(p.image_data_url);
                return `
          <div class="list-row" style="cursor:pointer;" data-inspect-product="${escapeHtml(p.barcode)}">
            <div class="list-row-left">
              <div class="item-thumb">
                ${safeImg ? `<img src="${escapeHtml(safeImg)}" alt="" />` : escapeHtml(p.name.slice(0, 1).toUpperCase())}
              </div>
              <div>
                <div class="item-title">${escapeHtml(p.name)}</div>
                <div class="item-sub">${escapeHtml(p.category)} • Barkod: ${escapeHtml(p.barcode)} • ${escapeHtml(p.last_store_name || "Nepoznato")}</div>
              </div>
            </div>
            <div class="row">
              <span class="btn btn-subtle" style="padding:5px 10px;font-size:11px;">Usporedi cijene →</span>
              <div class="item-price">${escapeHtml(formatCents(p.last_price_cents))}</div>
            </div>
          </div>`;
              })
              .join("")
      }
    </div>
  `;
}

function renderTrgovine(): string {
  return `
    ${pageHeader("Trgovine", "Dodajte vlastite trgovine za praćenje cijena i kreiranje popisa po dućanu.")}

    <div class="card">
      <div class="row-wrap">
        <input id="inp-store-name" value="${escapeHtml(model.draftStoreName)}" placeholder="Naziv trgovine (npr. Plodine, DM, Pekara)..." style="flex:1;" />
        <input id="inp-store-loc" value="${escapeHtml(model.draftStoreLoc)}" placeholder="Lokacija (opcionalno)" style="width:200px;" />
        <button class="btn btn-primary" id="btn-add-store">+ Dodaj trgovinu →</button>
      </div>
    </div>

    <div class="list-container">
      ${model.stores
        .map((s) => {
          const safeColor = sanitizeColor(s.color);
          return `
        <div class="list-row">
          <div class="list-row-left">
            <div class="item-thumb" style="background:${safeColor}18;color:${safeColor};">
              ${escapeHtml(s.name.slice(0, 1).toUpperCase())}
            </div>
            <div>
              <div class="item-title">${escapeHtml(s.name)}</div>
              <div class="item-sub">${escapeHtml(s.location || "Hrvatska")} • ${escapeHtml(s.item_count)} artikala u bazi</div>
            </div>
          </div>
          <button class="btn btn-subtle" data-store-plan="${escapeHtml(s.id)}">Kreiraj popis →</button>
        </div>`;
        })
        .join("")}
    </div>
  `;
}

function peerSyncRecord(peerId: string): PeerSyncRecord | undefined {
  return model.lanStatus?.peers.find((p) => p.peer_public_key === peerId);
}

function renderPeerSyncDetails(m: NetworkMember): string {
  const rec = peerSyncRecord(m.peer_id);
  const manualVal = model.draftManualAddrs[m.peer_id] ?? rec?.manual_addr ?? rec?.last_addr ?? "";
  const pathLabel = rec?.last_path === "lan" ? "LAN" : rec?.last_path === "manual" ? "Ručna adresa" : "—";
  const lastSync = rec?.last_sync_at ? formatDate(rec.last_sync_at) : "Još nije razmijenjeno";
  const summaryStats = rec ? `${pathLabel} · ${lastSync} · primljeno ${rec.last_applied}, poslano ${rec.last_sent}` : "Čeka prvu razmjenu";
  return `<details class="peer-sync-details">
    <summary class="item-sub">${escapeHtml(summaryStats)}${rec?.last_error ? ` · <span class="error-inline">Greška: ${escapeHtml(rec.last_error)}</span>` : ""}</summary>
    <div class="peer-sync-controls">
      <div class="item-sub">Zadnja adresa: ${escapeHtml(rec?.last_addr ?? "—")}</div>
      <div class="row-wrap">
        <input class="inp-manual-addr" data-manual-addr-input="${escapeHtml(m.peer_id)}" value="${escapeHtml(manualVal)}" placeholder="IP:port (npr. 192.168.1.25:4411)" />
        <button class="btn btn-outline" data-save-manual-addr="${escapeHtml(m.peer_id)}" ${model.lanBusy ? "disabled" : ""}>Spremi adresu</button>
        <button class="btn btn-primary" data-sync-peer-at="${escapeHtml(m.peer_id)}" ${model.lanBusy ? "disabled" : ""}>Razmijeni sada</button>
      </div>
    </div>
  </details>`;
}

function renderSync(): string {
  const ownDevices = model.members.filter(m => m.role === "own_device");
  const family = model.members.filter(m => m.role === "family_member");
  const progress = syncTracker.progress;
  const lan = model.lanStatus;
  const lanStateLabel = !lan
    ? "LAN servis nije dostupan"
    : !lan.running
      ? "LAN razmjena zaustavljena"
      : lan.paused
        ? "LAN razmjena pauzirana"
        : "LAN razmjena aktivna";
  const status = progress?.state === "running"
    ? "Razmjena je u tijeku"
    : progress?.state === "error"
      ? "Razmjena nije dovršena"
      : lan?.last_error && !lan.running
        ? lanStateLabel
        : lan?.running && !lan.paused
          ? "Automatska LAN razmjena je uključena"
          : "Podaci su spremljeni na uređaju";
  return `${pageHeader("Obitelj i uređaji", "Birajte što dijelite. Osobni računi ostaju na vašim uređajima.")}
    <section class="family-status ${progress?.state === "error" || lan?.last_error ? "has-error" : ""}">
      <div>
        <h2>${escapeHtml(status)}</h2>
        <p>${progress ? `Zadnja radnja: ${escapeHtml(progress.title)} · ${escapeHtml(new Date(progress.updatedAt).toLocaleString("hr-HR"))}` : lan?.running ? `${escapeHtml(lanStateLabel)} · sluša na ${escapeHtml(lan.exchange_addr || "—")}${lan.last_round_at ? ` · zadnji krug ${escapeHtml(formatDate(lan.last_round_at))}` : ""}` : "Za razmjenu s drugim uređajem pokrenite LAN servis ili izvezite/uvezite paket."}</p>
        ${lan?.last_error ? `<p class="item-sub error-inline">Zadnja greška: ${escapeHtml(lan.last_error)}</p>` : ""}
      </div>
      <div class="row-wrap">
        <button class="btn btn-primary" id="btn-open-pairing">Dodaj uređaj ili člana</button>
        ${lan?.running
          ? `<button class="btn btn-outline" id="btn-lan-sync-now" ${model.lanBusy || lan.paused ? "disabled" : ""}>${model.lanBusy ? "Razmjena…" : "Sinkroniziraj sada"}</button>
             <button class="btn btn-outline" id="btn-lan-pause" ${model.lanBusy ? "disabled" : ""}>${lan.paused ? "Nastavi" : "Pauziraj"}</button>
             <button class="btn btn-subtle" id="btn-lan-stop" ${model.lanBusy ? "disabled" : ""}>Zaustavi</button>`
          : `<button class="btn btn-outline" id="btn-lan-start" ${model.lanBusy ? "disabled" : ""}>Pokreni LAN</button>`}
      </div>
    </section>
    <section class="card"><h2>Moji uređaji</h2><p class="item-sub">Pristup mojim računima, popisima i katalogu.</p><div class="device-row"><strong>Ovaj uređaj</strong><span class="item-sub">Lokalno spremljeno${lan?.exchange_addr ? ` · ${escapeHtml(lan.exchange_addr)}` : ""}</span></div>${ownDevices.map(m => `<div class="member-block"><div class="device-row"><div class="item-copy"><strong>${escapeHtml(m.name)}</strong><span class="item-sub">Dodan ${escapeHtml(formatDate(m.added_at))} · svi osobni podaci</span></div><button class="btn btn-outline" data-member-access="${escapeHtml(m.peer_id)}">Pregled pristupa</button></div>${renderPeerSyncDetails(m)}</div>`).join("")}</section>
    <section class="card"><h2>Podijeljeno s obitelji</h2><p class="item-sub">Odabrani popisi i podaci o njihovim artiklima.</p>${family.length ? family.map(m => `<div class="member-block"><div class="member-row"><div class="item-copy"><strong>${escapeHtml(m.name)}</strong><span class="item-sub">${escapeHtml(sharedListNames(m.shared_lists, model.lists).join(" · ") || "Popisi nisu navedeni")}</span></div><button class="btn btn-outline" data-member-access="${escapeHtml(m.peer_id)}">Pregled pristupa</button></div>${renderPeerSyncDetails(m)}</div>`).join("") : '<div class="empty-state compact"><p>Još nema dodanih članova obitelji.</p></div>'}</section>
    <section class="card package-card"><h2>Ručna razmjena paketima</h2><p class="item-sub">Izvezite paket i otvorite ga na drugom uređaju kad nisu na istoj mreži. Izvoz ne potvrđuje primitak.</p><div class="row-wrap"><button class="btn btn-outline" id="btn-export-bundle">Izvezi za moj uređaj</button><label class="btn btn-primary file-button">Uvezi paket<input id="inp-import-bundle" type="file" accept=".json,.list2cart" /></label></div>
      <details><summary>Pošalji odabrane popise obitelji</summary><div class="list-choices">${model.lists.length ? model.lists.map(l => `<label class="check-option"><input type="checkbox" data-export-list="${escapeHtml(l.sync_id)}" ${model.exportListIds.includes(l.sync_id) ? "checked" : ""} /><span>${escapeHtml(l.name)}</span></label>`).join("") : '<p class="item-sub">Najprije kreirajte popis za kupnju.</p>'}</div><button class="btn btn-outline" id="btn-export-family-bundle" ${model.exportListIds.length ? "" : "disabled"}>Izvezi odabrane popise</button></details>
    </section>
    <div class="family-footer"><button class="btn btn-outline" data-goto="napredno">Napredne postavke</button><span class="item-sub">Vlastiti server je opcionalan.</span></div>
    <section class="mobile-only mobile-more no-print" aria-label="Više mogućnosti"><h2>Više mogućnosti</h2><div class="grid-2">${NAV_ITEMS.filter(n => ["pregled", "katalog", "cijene", "trgovine"].includes(n.id)).map(n => `<button class="card action-card" data-goto="${n.id}">${icon(n.id)}<strong>${n.label} →</strong></button>`).join("")}</div></section>`;
}

function renderPairingDialog(): string {
  if (!model.pairingOpen) return "";
  const family = model.pairingRole === "family_member";
  const selected = (model.draftMemberLists ?? "").split(",").filter(Boolean);
  const ps = model.pairStatus;
  const isLan = model.pairMode === "lan";

  if (isLan) {
    const discoverable = model.lanPeersDiscovered.filter((p) => (p.pairPort ?? 0) > 0);
    return `<div class="modal-backdrop"><section class="modal-card" role="dialog" aria-modal="true" aria-labelledby="pairing-title"><div class="row space-between"><h2 id="pairing-title">Dodaj uređaj ili člana</h2><button class="icon-button" data-close-dialog aria-label="Odustani od dodavanja" ${model.pairingBusy ? "disabled" : ""}>${icon("close")}</button></div>
      <div class="row-wrap"><button class="btn btn-primary" data-pair-mode="lan">Preko mreže (TLS 1.3)</button><button class="btn btn-outline" data-pair-mode="offline">Ručna evidencija</button></div>
      ${ps?.pending ? `
        <p class="item-sub">Usporedite ovaj 6-znamenkasti kod na oba uređaja prije potvrde:</p>
        <div class="sas-code" aria-label="Kod za usporedbu">${escapeHtml(ps.sas ?? "—")}</div>
        <div class="highlight-box"><strong>${escapeHtml(ps.peer_name || "Nepoznati uređaj")}</strong><div class="item-sub">Adresa: ${escapeHtml(ps.peer_host ?? "—")}</div><div class="item-sub">Ključ: <code>${escapeHtml((ps.peer_public_key ?? "").slice(0, 18))}…</code></div></div>
        <label class="field">Pristup podacima<select id="sel-member-role" ${model.pairingBusy ? "disabled" : ""}><option value="family_member" ${family ? "selected" : ""}>Član obitelji · odabrani popisi</option><option value="own_device" ${family ? "" : "selected"}>Moj uređaj · svi moji podaci</option></select></label>
        ${family ? `<fieldset class="list-choices"><legend>Popisi za dijeljenje</legend>${model.lists.length ? model.lists.map(l => `<label class="check-option"><input type="checkbox" data-pair-list="${escapeHtml(l.sync_id)}" ${selected.includes(l.sync_id) ? "checked" : ""} ${model.pairingBusy ? "disabled" : ""} /><span>${escapeHtml(l.name)}</span></label>`).join("") : '<p class="item-sub">Kreirajte popis prije dodavanja člana obitelji.</p>'}</fieldset>` : '<p class="item-sub">Ovaj uređaj moći će primati i osobne račune, katalog i sve popise.</p>'}
        <label class="check-option"><input id="pairing-confirmed" type="checkbox" ${model.pairingConfirmed ? "checked" : ""} ${model.pairingBusy ? "disabled" : ""} /><span>Kod je jednak na oba uređaja.</span></label>
        <div class="dialog-actions"><button class="btn btn-outline" id="btn-reject-live-pairing" ${model.pairingBusy ? "disabled" : ""}>Odbij</button><button class="btn btn-primary" id="btn-confirm-live-pairing" ${!model.pairingConfirmed || model.pairingBusy || (family && !selected.length) ? "disabled" : ""}>${model.pairingBusy ? "Potvrđivanje…" : "Potvrdi i upari"}</button></div>
      ` : `
        <label class="field">Naziv ovog uređaja<input id="inp-device-name" value="${escapeHtml(model.draftDeviceName)}" placeholder="Npr. Tomislav · MacBook" ${model.pairingBusy ? "disabled" : ""} /></label>
        <div class="row-wrap">
          ${ps?.waiting
            ? `<button class="btn btn-outline" id="btn-stop-pair-listen" ${model.pairingBusy ? "disabled" : ""}>Zaustavi čekanje (port ${ps.listen_port ?? "?"})</button>`
            : `<button class="btn btn-outline" id="btn-start-pair-listen" ${model.pairingBusy ? "disabled" : ""}>Čekaj uparivanje na ovom uređaju</button>`}
          <button class="btn btn-outline" id="btn-discover-pair-peers" ${model.pairingBusy ? "disabled" : ""}>Pronađi na LAN-u</button>
        </div>
        ${ps?.waiting ? `<p class="item-sub" role="status">Uređaj čeka partnera na portu <strong>${ps.listen_port ?? "?"}</strong>…</p>` : ""}
        ${discoverable.length ? `<div class="list-choices">${discoverable.map(p => `<div class="row space-between"><span><strong>${escapeHtml(p.name)}</strong> (${escapeHtml(p.addr)}:${p.pairPort})</span><button class="btn btn-primary" data-dial-discovered="${escapeHtml(`${p.addr}:${p.pairPort}`)}" ${model.pairingBusy ? "disabled" : ""}>Spoji se</button></div>`).join("")}</div>` : ""}
        <div class="row-wrap"><label class="field grow">Adresa drugog uređaja (IP:port)<input id="inp-pair-addr" value="${escapeHtml(model.draftPairAddr)}" placeholder="192.168.1.25:45123" ${model.pairingBusy ? "disabled" : ""} /></label><button class="btn btn-primary" id="btn-dial-pair" ${model.pairingBusy ? "disabled" : ""}>Spoji se</button></div>
        ${ps?.error ? `<p class="item-sub error-inline">${escapeHtml(ps.error)}</p>` : ""}
      `}
    </section></div>`;
  }

  return `<div class="modal-backdrop"><section class="modal-card" role="dialog" aria-modal="true" aria-labelledby="pairing-title"><div class="row space-between"><h2 id="pairing-title">Dodaj uređaj ili člana</h2><button class="icon-button" data-close-dialog aria-label="Odustani od dodavanja" ${model.pairingBusy ? "disabled" : ""}>${icon("close")}</button></div>
    <div class="row-wrap"><button class="btn btn-outline" data-pair-mode="lan">Preko mreže (TLS 1.3)</button><button class="btn btn-primary" data-pair-mode="offline">Ručna evidencija</button></div>
    <p class="item-sub">Usporedite kod s drugim uređajem prije dodavanja u evidenciju za razmjenu paketa.</p>
    <div class="sas-code" aria-label="Kod za usporedbu">${escapeHtml(model.pendingSasCode)}</div>
    <label class="field">Naziv uređaja ili osobe<input id="inp-member-name" value="${escapeHtml(model.draftMemberName)}" placeholder="Npr. Ana · iPhone" ${model.pairingBusy ? "disabled" : ""} /></label>
    <label class="field">Pristup podacima<select id="sel-member-role" ${model.pairingBusy ? "disabled" : ""}><option value="family_member" ${family ? "selected" : ""}>Član obitelji · odabrani popisi</option><option value="own_device" ${family ? "" : "selected"}>Moj uređaj · svi moji podaci</option></select></label>
    ${family ? `<fieldset class="list-choices"><legend>Popisi za dijeljenje</legend>${model.lists.length ? model.lists.map(l => `<label class="check-option"><input type="checkbox" data-pair-list="${escapeHtml(l.sync_id)}" ${selected.includes(l.sync_id) ? "checked" : ""} ${model.pairingBusy ? "disabled" : ""} /><span>${escapeHtml(l.name)}</span></label>`).join("") : '<p class="item-sub">Kreirajte popis prije dodavanja člana obitelji.</p>'}</fieldset><p class="item-sub">Osobni računi nisu dio obiteljskog paketa.</p>` : '<div class="highlight-box"><p>Ovaj uređaj moći će primati i osobne račune, katalog i sve popise.</p></div>'}
    <label class="check-option"><input id="pairing-confirmed" type="checkbox" ${model.pairingConfirmed ? "checked" : ""} ${model.pairingBusy ? "disabled" : ""} /><span>Kod je jednak na oba uređaja.</span></label>
    <div class="dialog-actions"><button class="btn btn-outline" id="btn-reject-pairing" ${model.pairingBusy ? "disabled" : ""}>Ne podudara se</button><button class="btn btn-primary" id="btn-add-member" ${!model.pairingConfirmed || model.pairingBusy || (family && !selected.length) ? "disabled" : ""}>${model.pairingBusy ? "Spremanje…" : "Potvrdi i dodaj"}</button></div></section></div>`;
}

function renderMemberDialog(): string {
  const m = model.removalMember ?? model.accessMember;
  if (!m) return "";
  if (model.removalMember) return `<div class="modal-backdrop"><section class="modal-card" role="dialog" aria-modal="true" aria-labelledby="member-title"><h2 id="member-title">Ukloni iz evidencije?</h2><p>${escapeHtml(m.name)}</p><p class="item-sub">Ovaj se uređaj ili član uklanja iz lokalne evidencije. Već poslane datoteke i podaci na drugim uređajima ostaju kod primatelja.</p><div class="dialog-actions"><button class="btn btn-outline" data-close-dialog>Odustani</button><button class="btn btn-danger" data-remove-member="${escapeHtml(m.peer_id)}">Ukloni</button></div></section></div>`;
  const names = m.role === "own_device" ? ["Svi moji računi, katalog i popisi"] : sharedListNames(m.shared_lists, model.lists);
  return `<div class="modal-backdrop"><section class="modal-card" role="dialog" aria-modal="true" aria-labelledby="member-title"><div class="row space-between"><h2 id="member-title">Pristup podacima</h2><button class="icon-button" data-close-dialog aria-label="Zatvori pregled pristupa">${icon("close")}</button></div><strong>${escapeHtml(m.name)}</strong><p class="item-sub">${m.role === "own_device" ? "Moj uređaj" : "Član obitelji"} · dodan ${escapeHtml(formatDate(m.added_at))}</p><ul class="access-list">${names.map(name => `<li>${escapeHtml(name)}</li>`).join("")}</ul><p class="item-sub">${m.role === "family_member" ? "Ovo je odabir popisa za razmjenu paketa. Osobni računi ostaju privatni." : "Osobni paket uključuje sve vaše podatke."}</p><div class="dialog-actions"><button class="btn btn-danger-ghost" id="btn-review-remove-member">Ukloni iz evidencije</button><button class="btn btn-primary" data-close-dialog>Zatvori</button></div></section></div>`;
}

function renderAdvanced(): string {
  const st = model.ledgerStats;
  return `${pageHeader("Napredne postavke", "Postavke mreže, održavanje i tehnički podaci.", '<button class="btn btn-outline" data-goto="sync">Natrag na obitelj</button>')}
    <section class="card"><h2>Naziv mreže</h2><div class="row-wrap"><label class="field grow">Naziv<input id="inp-network-name" value="${escapeHtml(model.draftNetworkName ?? st?.network_name ?? "Obiteljska mreža")}" /></label><button class="btn btn-primary" id="btn-save-net-name">Spremi naziv</button></div></section>
    <section class="card"><h2>Knjiga verzija</h2><dl class="technical-stats"><div><dt>Identitet uređaja</dt><dd>${escapeHtml(st?.node_id ?? "—")}</dd></div><div><dt>Aktivni zapisi</dt><dd>${st?.unique_records ?? 0}</dd></div><div><dt>Verzije</dt><dd>${st?.total_versions ?? 0}</dd></div><div><dt>Oznake arhiviranja</dt><dd>${st?.tombstones ?? 0}</dd></div><div><dt>Poznati čvorovi (Frontier)</dt><dd>${st?.frontier_nodes ?? 0}</dd></div><div><dt>Veličina baze</dt><dd>${formatQuantity((st?.db_size_bytes ?? 0) / 1024)} KB</dd></div></dl>
    <details><summary>Kako radi održavanje</summary><p class="item-sub">Pri pokretanju i svakog sata provjerava se održavanje. Najviše jednom dnevno sažimaju se suvišne međuv­erzije starije od 30 dana ili iznad zadnje 3 verzije. Posljednje verzije, oznake arhiviranja, dokazi ovlasti i zapisi koji čekaju primjenu ostaju sačuvani. Nakon sažimanja SQLite VACUUM oslobađa prostor.</p></details><button class="btn btn-outline" id="btn-compact-ledger">Očisti stare verzije</button></section>
    <section class="card"><h2>Način razmjene</h2><p>Automatska TLS 1.3 LAN razmjena i potpisani .list2cart paketi.</p><p class="item-sub">LAN servis: ${model.lanStatus?.running ? (model.lanStatus.paused ? "pauziran" : `aktivan (${escapeHtml(model.lanStatus.exchange_addr || "—")})`) : "zaustavljen"}. Legacy PocketBase uvoz služi samo za stari uvoz trgovina i sve promjene zapisuje kroz potpisani zapis promjena.</p></section>
    ${renderIdentityConflicts()}`;
}

function comparisonLines() {
  const listId = model.comparisonSource.startsWith("list:") ? Number(model.comparisonSource.slice(5)) : null;
  return listId === null ? model.cart : model.lists.find(l => l.id === listId)?.items ?? [];
}

async function refreshComparisons(): Promise<void> {
  if (model.comparisonBusy) return;
  model.comparisonBusy = true; model.comparisonError = ""; model.comparisonLoaded = false; render();
  try {
    const lines = comparisonLines();
    const products = await listProducts();
    const barcodes = [...new Set(lines.map(line => resolveBasketProduct(line, products)?.barcode).filter((v): v is string => !!v))];
    const prices = new Map(await Promise.all(barcodes.map(async barcode => [barcode, await getProductStorePrices(barcode)] as const)));
    model.comparisons = compareBasket(lines, products, prices);
    model.comparisonLoaded = true;
  } catch (e) { model.comparisonError = `Cijene nisu dohvaćene: ${e}`; model.comparisons = []; }
  finally { model.comparisonBusy = false; render(); }
}

function renderPrices(): string {
  const lines = comparisonLines();
  const complete = model.comparisons.filter(c => c.missing.length === 0);
  const partial = model.comparisons.filter(c => c.missing.length > 0);
  const row = (c: BasketComparison, best = false) => `<div class="comparison-row ${best ? "best-price" : ""}"><div class="item-copy"><strong>${escapeHtml(c.store)}</strong><span class="item-sub">${c.matched}/${c.count} artikala · ${c.missing.length ? "podzbroj poznatih cijena" : "isti artikli i količine"}</span><span class="item-sub">Najstarija cijena: ${escapeHtml(formatDate(c.oldestPrice))}</span>${c.missing.length ? `<span class="item-sub">Nedostaje: ${escapeHtml(c.missing.join(", "))}</span>` : ""}</div><strong class="comparison-price">${escapeHtml(formatCents(c.totalCents))}</strong></div>`;
  return `${pageHeader("Cijene iste košarice", "Usporedba istih artikala i količina prema zabilježenim cijenama.")}
    <section class="card comparison-controls"><label class="field grow">Popis za usporedbu<select id="comparison-source" ${model.comparisonBusy ? "disabled" : ""}><option value="cart" ${model.comparisonSource === "cart" ? "selected" : ""}>Aktivna košarica</option>${model.lists.map(l => `<option value="list:${l.id}" ${model.comparisonSource === `list:${l.id}` ? "selected" : ""}>${escapeHtml(l.name)}</option>`).join("")}</select></label><button class="btn btn-primary" id="btn-refresh-prices" ${model.comparisonBusy ? "disabled" : ""}>${model.comparisonBusy ? "Dohvaćanje…" : "Usporedi cijene"}</button></section>
    <div role="status" aria-live="polite">${model.comparisonError ? `<div class="card error-message">${escapeHtml(model.comparisonError)}</div>` : ""}${model.comparisonBusy ? '<p class="item-sub">Učitavanje zabilježenih cijena…</p>' : ""}</div>
    ${model.comparisonLoaded ? complete.length ? `<section class="card"><h2>Potpune košarice · ${lines.length}/${lines.length} artikala</h2>${complete.map((c, i) => row(c, i === 0 && complete.length > 1)).join("")}</section>` : '<section class="card"><h2>Još nema potpune košarice za usporedbu</h2><p class="item-sub">Potrebna je cijena svakog artikla u istoj trgovini. Artikl prepoznajemo po barkodu ili jedinstvenom nazivu iz kataloga.</p></section>' : ""}
    ${model.comparisonLoaded && partial.length ? `<section class="card"><h2>Nepotpune košarice · izvan poretka</h2><p class="item-sub">Podzbrojevi nisu cijena cijelog popisa.</p>${partial.map(c => row(c)).join("")}</section>` : ""}
    ${complete.length > 1 && model.comparisonLoaded ? `<p class="savings-note">Razlika između najviše i najniže potpune košarice: ${escapeHtml(formatCents(complete.at(-1)!.totalCents - complete[0].totalCents))} (${escapeHtml(formatCents(complete.at(-1)!.totalCents))} − ${escapeHtml(formatCents(complete[0].totalCents))}).</p>` : ""}
    ${!lines.length ? '<section class="card empty-state"><h2>Odaberite popis s artiklima</h2><p>Dodajte artikle na popis ili u košaricu pa usporedite cijene.</p><button class="btn btn-outline" data-goto="planer">Otvori popise</button></section>' : ""}
    <p class="item-sub">Cijene ne potvrđuju dostupnost na polici. Uvjeti kartica vjernosti nisu evidentirani u ovim podacima.</p><button class="btn btn-outline no-print" data-toggle-retail>Otvori javni cjenik trgovine</button>`;
}

function conflictLabel(group: IdentityConflict, id: string): string {
  const store = model.stores.find(s => s.sync_id === id);
  const product = model.products.find(p => p.sync_id === id);
  return group.entity === "stores"
    ? `${store?.name ?? group.value} · ${store?.location || "Bez lokacije"}`
    : `${product?.name ?? group.value} · ${product ? formatCents(product.last_price_cents) : ""}`;
}

function renderIdentityConflicts(): string {
  const review = model.conflictReview;
  return `<section class="card" aria-label="Duplikati zapisa">
    <div class="row space-between"><h2>Duplikati zapisa${model.conflicts.length ? ` (${model.conflicts.length})` : ""}</h2>
    <button class="btn btn-outline" id="refresh-conflicts" ${model.resolvingConflict ? "disabled" : ""}>Osvježi</button></div>
    ${review ? `<h3>Potvrdi odabir</h3>
      <p>Aktivan ostaje: <strong>${escapeHtml(conflictLabel(review.group, review.keepId))}</strong></p>
      <p>Arhivirat će se:</p><ul>${review.group.sync_ids.filter(id => id !== review.keepId).map(id => `<li>${escapeHtml(conflictLabel(review.group,id))} <small>${escapeHtml(id)}</small></li>`).join("")}</ul>
      <p>Izvorni zapisi, povijest cijena i računi ostaju sačuvani s postojećim vezama. Polja se ne prepisuju niti se povijest premješta na odabrani zapis.</p>
      <button class="btn btn-primary" id="confirm-conflict" ${model.resolvingConflict ? "disabled" : ""}>${model.resolvingConflict ? "Spremanje…" : "Potvrdi i arhiviraj duplikate"}</button>
      <button class="btn btn-outline" id="cancel-conflict" ${model.resolvingConflict ? "disabled" : ""}>Odustani</button>`
    : model.conflicts.length ? `<p>Odaberi zapis koji želiš koristiti ubuduće. Prije spremanja možeš pregledati odabir.</p>
      ${model.conflicts.map((group,index) => `<div class="card"><h3>${group.entity === "stores" ? "Trgovine istog naziva" : "Artikli istog barkoda"}: ${escapeHtml(group.value)}</h3>
      ${group.sync_ids.map(id => `<div class="row space-between"><div><strong>${escapeHtml(conflictLabel(group,id))}</strong><div><small>${escapeHtml(id)}</small></div></div>
      <button class="btn btn-outline" data-conflict-group="${index}" data-conflict-keep="${escapeHtml(id)}">Zadrži ovaj zapis</button></div>`).join("")}</div>`).join("")}`
    : "<p>Nema nerazriješenih duplikata.</p>"}
  </section>`;
}

async function refreshConflictData(): Promise<void> {
  const [conflicts, stores, products, receipts, lists, stats, incomplete] = await Promise.all([
    identityConflicts(), listStores(), listProducts(), listReceipts(), listShoppingLists(), getLedgerStats(), incompleteReceiptCount(),
  ]);
  model.conflicts = conflicts; model.stores = stores; model.products = products;
  model.receipts = receipts; model.lists = lists;
  model.ledgerStats = stats;
  model.incompleteReceipts = incomplete;
  if (!stores.some(s => s.id === model.selectedStoreId)) model.selectedStoreId = stores[0]?.id ?? 0;
}

function retailerChain(name:string):string|null {
  const lower=name.toLowerCase();
  return ["konzum","lidl","kaufland","spar"].find(chain=>lower.includes(chain)) ?? null;
}

function renderRetailSearch():string {
  const r=retail.results;
  const source=retail.sources.find(s=>s.url===retail.branch);
  const today=new Intl.DateTimeFormat("sv-SE",{timeZone:"Europe/Zagreb"}).format(new Date());
  return `<section class="card" aria-label="Internetska pretraga proizvoda"><h2>Pronađi proizvod bez barkoda</h2>
    <p>Odaberi svoju trgovinu i konkretnu poslovnicu. Javni cjenik preuzima se izravno od trgovca, a naziv tražimo na uređaju.</p>
    <div class="row-wrap"><select id="retail-store" aria-label="Trgovina za internetsku pretragu" ${retail.busy?"disabled":""}>
      ${model.stores.map(s=>`<option value="${s.id}" ${model.selectedStoreId===s.id?"selected":""}>${escapeHtml(s.name)} · ${escapeHtml(s.location)}</option>`).join("")}</select>
      <button class="btn btn-outline" id="retail-load" ${retail.busy?"disabled":""}>Učitaj poslovnice</button></div>
      ${retail.sources.length ? `<label>Traži poslovnicu po gradu ili adresi<input id="retail-location" placeholder="Npr. Zagreb, Ilica…" ${retail.busy?"disabled":""}/></label><label>Poslovnica i datum cjenika<select id="retail-branch" style="width:100%;max-width:100%;" ${retail.busy?"disabled":""}><option value="">Odaberi poslovnicu…</option>${retail.sources.map(s=>`<option value="${escapeHtml(s.url)}" ${s.url===retail.branch?"selected":""}>${escapeHtml(s.label)} · ${escapeHtml(s.price_date)}</option>`).join("")}</select></label>`:""}
      <details ${localPriceCsv.inspection?"open":""}><summary>Cjenik druge trgovine · otvori CSV</summary>
        <p>Odaberi CSV za gore odabranu trgovinu. Datoteka se čita na ovom uređaju. Cijene moraju biti u eurima; datum i poslovnicu provjeri u izvorniku.</p>
        <input id="retail-file" type="file" accept=".csv,.tsv,text/csv,text/tab-separated-values" aria-label="Lokalni CSV cjenik" ${retail.busy?"disabled":""}/>
        ${renderCsvMapping()}
      </details>
      ${retail.branch==="local"&&localPriceCsv.products?`<p role="status">Aktivan lokalni cjenik: ${escapeHtml(localPriceCsv.filename)} · ${localPriceCsv.products.length} proizvoda. Čuva se do promjene trgovine ili zatvaranja aplikacije.</p>`:""}
    ${source && source.price_date!==today ? `<p role="status">Cjenik je od ${escapeHtml(source.price_date)}. Nije današnji — cijenu provjeri u poslovnici.</p>`:""}
    <div class="row-wrap"><input id="retail-query" aria-label="Naziv proizvoda na internetu" value="${escapeHtml(retail.query)}" placeholder="Npr. mlijeko, kava, jabuke…" maxlength="100" style="flex:1;" ${retail.busy?"disabled":""} />
      <button class="btn btn-primary" id="retail-search" ${retail.busy || !retail.branch?"disabled":""}>${retail.busy?"Dohvaćanje cjenika…":"Pretraži cjenik"}</button></div>
    <div role="status" aria-live="polite">${retail.error?escapeHtml(retail.error):""}</div>
      ${r ? `<p>Izvor: ${r.source.chain==="local"?`lokalna datoteka ${escapeHtml(r.source.label)} · datum nije potvrđen`:`<a href="${escapeHtml(r.source.url)}" target="_blank" rel="noreferrer">${escapeHtml(r.source.chain)} · službeni cjenik ${escapeHtml(r.source.price_date)}</a>`}. Učitano ${escapeHtml(new Date(r.fetched_at).toLocaleString("hr-HR"))}. Rezultati: ${r.total_matches}${r.total_matches>100?" (prikazano prvih 100; suzi naziv)":""}. Cjenik ne jamči raspoloživost na polici.</p>
      ${r.products.length?r.products.map((p,index)=>`<div class="list-row"><div><strong>${escapeHtml(p.name)}</strong><div class="item-sub">${escapeHtml(p.brand)} · ${escapeHtml(p.package)}${p.barcode?` · ${escapeHtml(p.barcode)}`:" · bez barkoda"}</div></div><div><strong>${escapeHtml(formatCents(p.price_cents))}</strong> <button class="btn btn-outline" data-retail-pick="${index}">Unos u košaricu</button> <button class="btn btn-outline" data-retail-pick="${index}" data-retail-target="plan">Unos na popis</button></div></div>`).join(""):"<p>Nema proizvoda za taj naziv u odabranom cjeniku.</p>"}`:""}
      <p class="item-sub">Odabir popunjava unos u košarici ili na popisu. Provjeri količinu, jedinicu i cijenu prije dodavanja; ništa se ne sprema automatski.</p>
  </section>`;
}

function renderCsvMapping():string {
  const inspection=localPriceCsv.inspection;if(!inspection)return "";
  const select=(key:string,label:string,selected:number|null|undefined,optional=false)=>`<label>${label}<select id="csv-${key}" ${retail.busy?"disabled":""}><option value="">${optional?"Nema / ne koristi":"Odaberi stupac…"}</option>${inspection.headers.map((h,i)=>`<option value="${i}" ${selected===i?"selected":""}>${i+1}. ${escapeHtml(h)}</option>`).join("")}</select></label>`;
  const m=inspection.suggested;
  return `<p><strong>${escapeHtml(localPriceCsv.filename)}</strong> · razdjelnik: ${inspection.delimiter===9?"tabulator":escapeHtml(String.fromCharCode(inspection.delimiter))}. ${m?"Prijedlog stupaca — potvrdi prije korištenja.":"Stupci nisu jednoznačno prepoznati. Odaberi ih ručno."}</p>
    <div class="row-wrap">${select("name","Naziv proizvoda",m?.name)}${select("price","Maloprodajna cijena (EUR)",m?.price)}${select("barcode","Barkod (neobvezno)",m?.barcode,true)}${select("brand","Marka (neobvezno)",m?.brand,true)}</div>
    <div style="overflow:auto;max-height:220px"><table><thead><tr>${inspection.headers.map(h=>`<th>${escapeHtml(h)}</th>`).join("")}</tr></thead><tbody>${inspection.sample.map(row=>`<tr>${row.map(v=>`<td>${escapeHtml(v)}</td>`).join("")}</tr>`).join("")}</tbody></table></div>
    <p>Cijena mora biti iznos za prodajno pakiranje, s najviše dvije decimale. Retci bez naziva ili valjane cijene neće biti uključeni. Nemoj odabrati cijenu po kilogramu ili povijesnu cijenu.</p>
    <button id="csv-use" class="btn btn-outline" ${retail.busy?"disabled":""}>Potvrdi stupce i koristi cjenik u EUR</button>`;
}

function bindRetailSearch():void {
  const store=document.getElementById("retail-store") as HTMLSelectElement|null;
  if(store) store.onchange=()=>{model.selectedStoreId=Number(store.value);retail.sources=[];retail.results=null;retail.branch="";retail.error="";localPriceCsv.inspection=null;localPriceCsv.products=null;localPriceCsv.bytes=[];render();};
  const location=document.getElementById("retail-location") as HTMLInputElement|null;
  if(location) location.oninput=()=>{
    const select=document.getElementById("retail-branch") as HTMLSelectElement|null;if(!select)return;
    const normalize=(s:string)=>s.toLocaleLowerCase("hr").normalize("NFD").replace(/[\u0300-\u036f]/g,"");
    const query=normalize(location.value.trim());
    select.innerHTML=`<option value="">Odaberi poslovnicu…</option>`+retail.sources.filter(s=>normalize(s.label).includes(query)).map(s=>`<option value="${escapeHtml(s.url)}">${escapeHtml(s.label)} · ${escapeHtml(s.price_date)}</option>`).join("");
    retail.branch="";retail.results=null;
    const button=document.getElementById("retail-search") as HTMLButtonElement|null;if(button)button.disabled=true;
  };
  const file=document.getElementById("retail-file") as HTMLInputElement|null;
  if(file)file.onchange=async()=>{
    const chosen=file.files?.[0];if(!chosen||retail.busy)return;
    localPriceCsv.inspection=null;localPriceCsv.products=null;localPriceCsv.bytes=[];retail.results=null;retail.branch="";retail.error="";
    if(chosen.size>12_000_000){retail.error="Cjenik je prevelik (najviše 12 MB).";render();return;}
    retail.busy=true;localPriceCsv.storeId=model.selectedStoreId;localPriceCsv.filename=chosen.name;render();
    try{localPriceCsv.bytes=Array.from(new Uint8Array(await chosen.arrayBuffer()));localPriceCsv.inspection=await inspectPriceCsv(localPriceCsv.bytes);}
    catch(e){retail.error=`CSV nije učitan: ${e}`;}
    finally{retail.busy=false;render();}
  };
  const use=document.getElementById("csv-use");
  if(use)use.onclick=async()=>{
    if(retail.busy||!localPriceCsv.inspection)return;
    if(localPriceCsv.storeId!==model.selectedStoreId){retail.error="Trgovina se promijenila; ponovno odaberi CSV.";render();return;}
    const index=(key:string)=>{const value=(document.getElementById(`csv-${key}`) as HTMLSelectElement).value;return value===""?null:Number(value);};
    const name=index("name"),price=index("price");if(name===null||price===null){retail.error="Odaberi naziv i cijenu.";render();return;}
    const mapping:CsvMapping={delimiter:localPriceCsv.inspection.delimiter,name,price,barcode:index("barcode"),brand:index("brand"),quantity:null,unit:null};
    retail.busy=true;retail.error="";render();
    try{localPriceCsv.products=await readPriceCsv(localPriceCsv.bytes,mapping);retail.sources=[];retail.branch="local";retail.storeId=localPriceCsv.storeId;localPriceCsv.inspection=null;localPriceCsv.bytes=[];showToast(`Učitano ${localPriceCsv.products.length} proizvoda. Sada pretraži naziv.`);}
    catch(e){retail.error=`Cjenik nije prihvaćen: ${e}`;}
    finally{retail.busy=false;render();}
  };
  const branch=document.getElementById("retail-branch") as HTMLSelectElement|null;
  if(branch) branch.onchange=()=>{retail.branch=branch.value;retail.results=null;render();};
  const query=document.getElementById("retail-query") as HTMLInputElement|null;
  if(query) query.oninput=()=>{retail.query=query.value;};
  const load=document.getElementById("retail-load");
  if(load) load.onclick=async()=>{
    if(retail.busy)return;
    const selected=activeStore(); const chain=selected?retailerChain(selected.name):null;
    if(!chain){retail.error="Podržani su Konzum, Lidl, Kaufland i Spar. Odaberi jednu od tih trgovina.";render();return;}
    localPriceCsv.products=null;localPriceCsv.inspection=null;localPriceCsv.bytes=[];
    retail.busy=true;retail.error="";retail.sources=[];retail.results=null;retail.branch="";retail.storeId=selected!.id;render();
    try{retail.sources=await retailSources(chain);}
    catch(e){retail.error=`Dohvat poslovnica nije uspio: ${e}`;}
    finally{retail.busy=false;render();}
  };
  const search=document.getElementById("retail-search");
  if(search) search.onclick=async()=>{
    if(retail.busy)return;
    if(retail.storeId!==model.selectedStoreId){retail.error="Trgovina se promijenila; ponovno učitaj poslovnice.";render();return;}
    if(retail.branch==="local"&&localPriceCsv.products){
      const normalize=(s:string)=>s.toLowerCase().normalize("NFD").replace(/[\u0300-\u036f]/g,"").replace(/đ/g,"d");
      const tokens=normalize(retail.query.trim()).split(/\s+/).filter(Boolean);
      if(retail.query.trim().length<2||!tokens.length){retail.error="Upiši barem dva znaka naziva.";render();return;}
      const matches=localPriceCsv.products.filter(p=>tokens.every(t=>normalize(`${p.name} ${p.brand}`).includes(t)));
      retail.error="";retail.results={source:{chain:"local",url:"",label:localPriceCsv.filename,price_date:""},fetched_at:new Date().toISOString(),products:matches.slice(0,100),total_matches:matches.length};render();return;
    }
    retail.busy=true;retail.error="";retail.results=null;render();
    try{retail.results=await retailSearch(retail.branch,retail.query);}
    catch(e){retail.error=`Pretraga nije uspjela: ${e}`;}
    finally{retail.busy=false;render();}
  };
  document.querySelectorAll<HTMLButtonElement>("[data-retail-pick]").forEach(button=>{button.onclick=()=>{
    if(retail.storeId!==model.selectedStoreId){showToast("Ponovno odaberi trgovinu i cjenik.");return;}
    const product=retail.results?.products[Number(button.dataset.retailPick)];if(!product)return;
    if(button.dataset.retailTarget==="plan") {
      const list=currentList();
      if(!list || (list.store_id!==null && list.store_id!==model.selectedStoreId)) {
        showToast("Najprije kreiraj popis za odabranu trgovinu u Planeru.");return;
      }
      model.draftPlanName=product.name;model.draftPlanPrice=(product.price_cents/100).toFixed(2);model.draftPlanQty="1";model.draftPlanUnit="kom";
      retail.results=null;model.retailOpen=false;model.tab="planer";render();
      document.getElementById("inp-plan-name")?.focus();
      showToast(`Pripremljeno za popis ${list.name}. Provjeri jedinicu i količinu pa dodaj.`);return;
    }
    model.draftName=product.name;model.draftBarcode=product.barcode??"";model.draftPrice=(product.price_cents/100).toFixed(2);model.draftQty="1";model.draftUnit="kom";model.draftPhotoDataUrl=null;
    retail.results=null;model.retailOpen=false;model.tab="kosarica";render();document.getElementById("inp-name")?.focus();showToast("Unos je popunjen. Provjeri jedinicu, količinu i cijenu prije dodavanja.");
  };});
}

function renderSyncProgress(): string {
  const p = syncTracker.progress;
  if (!p) return "";
  return `<section class="card" aria-busy="${p.state === "running"}">
    <h2>${escapeHtml(p.title)} · ${p.state === "running" ? "U tijeku" : p.state === "done" ? "Dovršeno" : "Zaustavljeno greškom"}</h2>
    <ol>${p.steps.map((label,index) => `<li>${index < p.completed ? "✓" : index === p.current ? (p.state === "error" ? "✕" : "…") : "○"} ${escapeHtml(label)}${index === p.current && p.state === "running" ? " — u tijeku" : ""}</li>`).join("")}</ol>
    ${p.detail ? `<p>${escapeHtml(p.detail)}</p>` : ""}
    <small>Posljednja promjena: ${escapeHtml(new Date(p.updatedAt).toLocaleTimeString("hr-HR"))}</small>
  </section>`;
}

function offerPackageDownload(json: string, prefix: string): void {
  const url = URL.createObjectURL(new Blob([json], {type:"application/json"}));
  try {
    const link = document.createElement("a");
    link.href = url;
    link.download = `${prefix}_${new Date().toISOString().slice(0,10)}.list2cart.json`;
    link.click();
  } finally { URL.revokeObjectURL(url); }
}

async function trackPackageExport(role: PeerRole, filter?: string | null): Promise<void> {
  let json = "";
  let savedPath: string | null = null;
  await syncTracker.run(role === "own_device" ? "Izvoz osobnog paketa" : "Izvoz obiteljskih popisa",[
    {label:"Priprema podataka i izrada paketa",action:async () => {json=await exportSyncBundleJson(role,filter);}},
    {label:"Spremanje datoteke paketa",action:async () => {
      savedPath = await saveSyncPackage(json,role === "family_member");
      if (!savedPath) offerPackageDownload(json,role === "own_device" ? "List2Cart_SVE" : "List2Cart_OBITELJ_LISTE");
    }},
  ],() => `${savedPath ? `Paket spremljen: ${savedPath}.` : "Paket je pripremljen za spremanje."} Primitak na drugom uređaju još nije potvrđen.`);
}

function render(): void {
  const root = document.getElementById("app");
  if (!root) return;

  let content = "";
  switch (model.tab) {
    case "pregled":
      content = renderPregled();
      break;
    case "kosarica":
      content = renderKosarica();
      break;
    case "planer":
      content = renderPlaner();
      break;
    case "racuni":
      content = renderReceipts();
      break;
    case "katalog":
      content = renderKatalog();
      break;
    case "cijene":
      content = renderPrices();
      break;
    case "trgovine":
      content = renderTrgovine();
      break;
    case "sync":
      content = renderSync();
      break;
    case "napredno":
      content = renderAdvanced();
      break;
  }

  root.innerHTML = `
    <div class="app-shell">
      ${renderSidebar()}
      <main class="main-content">
          ${model.retailOpen && ["kosarica", "planer", "katalog", "cijene"].includes(model.tab) ? renderRetailSearch() : ""}
          <div role="status" aria-live="polite" aria-atomic="true">${renderSyncProgress()}</div>
          ${model.incompleteReceipts ? `<div class="card" role="status">Računi koji čekaju dovršetak prijenosa: ${model.incompleteReceipts}. Prikazat će se kada stignu sve potvrđene stavke i zbrojevi budu usklađeni.</div>` : ""}
          ${(model.conflicts.length || model.conflictReview) && model.tab !== "napredno" ? renderIdentityConflicts() : ""}
        ${content}
      </main>
    </div>
    ${renderReceiptDialog()}
    ${renderPairingDialog()}
    ${renderMemberDialog()}
    ${
      model.scannerOpen
        ? `
      <div class="modal-backdrop">
        <div class="modal-card">
          <div class="row space-between">
            <div>
              <span class="pill">BARKOD SKENER I FOTOGRAFIJA PROIZVODA</span>
              <div class="item-title" style="font-size:17px;margin-top:4px;">Usmjerite kameru prema EAN-13 barkodu</div>
            </div>
            <button class="btn btn-outline" id="btn-close-scanner">✕</button>
          </div>
          <div class="scanner-viewfinder">
            <video id="scanner-video" autoplay playsinline muted></video>
            <div class="scanner-laser"></div>
          </div>
          <div class="row-wrap">
            <button class="btn btn-outline" id="btn-capture-frame" style="flex:1;">📸 Uslikaj proizvod kao ikonu</button>
            <label class="btn btn-subtle" style="cursor:pointer;">
              🖼️ Odaberi sliku
              <input id="inp-modal-photo" type="file" accept="image/*" capture="environment" style="display:none;" />
            </label>
          </div>
          <div class="row">
            <input id="inp-modal-barcode" value="${escapeHtml(model.draftModalBarcode)}" placeholder="Ili upišite / potvrdite barkod (npr. 3850102...)" style="flex:1;" />
            <button class="btn btn-primary" id="btn-confirm-scan">Primijeni →</button>
          </div>
        </div>
      </div>`
        : ""
    }
    <div id="toast-container">${model.toast ? `<div class="toast">${escapeHtml(model.toast)}</div>` : ""}</div>
  `;

  bindEvents();
}

function stopScannerCamera(): void {
  if (scannerDecodeTimer !== null) {
    window.clearInterval(scannerDecodeTimer);
    scannerDecodeTimer = null;
  }
  if (nativeScannerActive) {
    nativeScannerActive = false;
    void cancelNativeBarcodeScan().catch(() => {});
  }
  const video = document.getElementById("scanner-video") as HTMLVideoElement | null;
  const stream = video?.srcObject as MediaStream | null;
  stream?.getTracks().forEach((t) => t.stop());
}

async function applyScannedBarcode(rawCode: string): Promise<void> {
  const code = rawCode.trim();
  if (!code) return;

  stopScannerCamera();
  model.scannerOpen = false;
  model.draftBarcode = code;
  model.draftModalBarcode = "";

  const found = await getProduct(code);
  if (found) {
    model.draftName = found.name;
    model.draftUnit = found.default_unit;
    if (found.last_price_cents > 0) {
      model.draftPrice = (found.last_price_cents / 100).toFixed(2);
    }
    if (found.image_data_url && !model.draftPhotoDataUrl) {
      model.draftPhotoDataUrl = found.image_data_url;
    }
    render();
    showToast(`Skenirano iz kataloga: ${found.name}`);
    return;
  }

  const online = await lookupBarcodeOnline(code).catch(() => null);
  if (online) {
    model.draftName = online.name;
    render();
    showToast(`Skenirano online: ${online.name}`);
    return;
  }

  render();
  showToast(`Skeniran barkod: ${code}`);
}

function startVideoBarcodeDetection(video: HTMLVideoElement): void {
  if (!window.BarcodeDetector) return;
  let detector: BarcodeDetectorLike;
  try {
    detector = new window.BarcodeDetector({
      formats: ["ean_13", "ean_8", "upc_a", "upc_e", "code_128", "qr_code"],
    });
  } catch {
    return;
  }

  if (scannerDecodeTimer !== null) {
    window.clearInterval(scannerDecodeTimer);
  }

  scannerDecodeTimer = window.setInterval(async () => {
    if (!model.scannerOpen || video.readyState < 2) return;
    try {
      const results = await detector.detect(video);
      const hit = results.find((r) => r.rawValue && r.rawValue.trim().length > 0);
      if (hit?.rawValue) {
        await applyScannedBarcode(hit.rawValue);
      }
    } catch {
      // Frame decode failed; continue on next interval.
    }
  }, 250);
}

function bindEvents(): void {
  bindRetailSearch();
  const busy = syncTracker.progress?.state === "running";
  for (const id of ["btn-export-bundle", "btn-export-family-bundle", "inp-import-bundle"]) {
    const control = document.getElementById(id) as HTMLButtonElement | HTMLInputElement | null;
    if (control) control.disabled = busy;
  }
  document.querySelectorAll<HTMLButtonElement>("[data-conflict-keep]").forEach(btn => {
    btn.onclick = () => {
      const group = model.conflicts[Number(btn.dataset.conflictGroup)];
      const keepId = btn.dataset.conflictKeep;
      if (group && keepId && group.sync_ids.includes(keepId)) {
        model.conflictReview = {group,keepId}; render();
      }
    };
  });
  const cancelConflict = document.getElementById("cancel-conflict");
  if (cancelConflict) cancelConflict.onclick = () => { model.conflictReview = null; render(); };
  const refreshConflicts = document.getElementById("refresh-conflicts");
  if (refreshConflicts) refreshConflicts.onclick = async () => {
    try { await refreshConflictData(); model.conflictReview = null; render(); }
    catch (e) { showToast(`Dohvat duplikata nije uspio: ${e}`); }
  };
  const confirmConflict = document.getElementById("confirm-conflict");
  if (confirmConflict) confirmConflict.onclick = async () => {
    const review = model.conflictReview;
    if (!review || model.resolvingConflict) return;
    model.resolvingConflict = true; render();
    try {
      await resolveIdentityConflict(review.group,review.keepId);
      model.conflictReview = null;
      model.selectedProduct = null; model.selectedProductPrices = [];
      await refreshConflictData();
      showToast("Odabrani zapis ostaje aktivan. Duplikati su arhivirani.");
    } catch (e) { showToast(`Spremanje nije uspjelo: ${e}`); }
    finally { model.resolvingConflict = false; render(); }
  };
  document.querySelectorAll<HTMLButtonElement>("[data-tab]").forEach((btn) => {
    btn.onclick = () => {
      const nextTab = btn.dataset.tab as Tab;
      model.tab = nextTab;
      if (nextTab === "cijene" && !model.comparisonLoaded && !model.comparisonBusy) {
        void refreshComparisons();
        return;
      }
      render();
    };
  });

  document.querySelectorAll<HTMLButtonElement>("[data-goto]").forEach((btn) => {
    btn.onclick = () => {
      const nextTab = btn.dataset.goto as Tab;
      if (nextTab === "cijene" && model.tab === "planer") {
        const list = currentList();
        model.comparisonSource = list ? `list:${list.id}` : "cart";
        model.tab = nextTab;
        void refreshComparisons();
        return;
      }
      model.tab = nextTab;
      if (nextTab === "cijene" && !model.comparisonLoaded && !model.comparisonBusy) {
        void refreshComparisons();
        return;
      }
      render();
    };
  });

  document.querySelectorAll<HTMLButtonElement>("[data-toggle-retail]").forEach((btn) => {
    btn.onclick = () => {
      model.retailOpen = !model.retailOpen;
      render();
    };
  });

  document.querySelectorAll<HTMLButtonElement>("[data-close-dialog]").forEach((btn) => {
    btn.onclick = () => {
      if (model.pairingBusy) return;
      model.selectedReceipt = null;
      model.pairingOpen = false;
      model.accessMember = null;
      model.removalMember = null;
      render();
    };
  });

  document.querySelectorAll<HTMLButtonElement>("[data-store-plan]").forEach((btn) => {
    btn.onclick = () => {
      model.selectedStoreId = Number(btn.dataset.storePlan);
      model.newListOpen = true;
      model.tab = "planer";
      render();
    };
  });

  const btnCameraScan = document.getElementById("btn-camera-scan");
  if (btnCameraScan) {
    btnCameraScan.onclick = async () => {
      // 1. Pokušaj izvorni Tauri mobilni skener barkoda (Android / iOS plugin)
      try {
        let perm = await checkBarcodePermissions();
        if (perm !== "granted") {
          perm = await requestBarcodePermissions();
        }
        if (perm === "granted") {
          nativeScannerActive = true;
          const scanned = await scanNativeBarcode({
            windowed: false,
            formats: [Format.EAN13, Format.EAN8, Format.UPC_A, Format.UPC_E, Format.Code128],
          });
          nativeScannerActive = false;
          if (scanned?.content) {
            await applyScannedBarcode(scanned.content);
            return;
          }
        }
      } catch {
        nativeScannerActive = false;
      }

      // 2. Web / Desktop tok s kamerom i BarcodeDetector dekodiranjem uživo
      model.scannerOpen = true;
      render();
      try {
        const video = document.getElementById("scanner-video") as HTMLVideoElement | null;
        if (video && navigator.mediaDevices?.getUserMedia) {
          const stream = await navigator.mediaDevices.getUserMedia({
            video: { facingMode: "environment" },
          });
          video.srcObject = stream;
          startVideoBarcodeDetection(video);
        }
      } catch {
        // Kamera nije dostupna na ovom uređaju; korisnik može učitati sliku ili upisati barkod.
      }
    };
  }

  const btnCloseScanner = document.getElementById("btn-close-scanner");
  if (btnCloseScanner) {
    btnCloseScanner.onclick = () => {
      stopScannerCamera();
      model.scannerOpen = false;
      render();
    };
  }

  const btnCaptureFrame = document.getElementById("btn-capture-frame");
  if (btnCaptureFrame) {
    btnCaptureFrame.onclick = () => {
      const video = document.getElementById("scanner-video") as HTMLVideoElement | null;
      if (!video || !video.videoWidth) {
        return showToast("Kamera nije aktivna — odaberite sliku pomoću gumba pokraj.");
      }
      const canvas = document.createElement("canvas");
      canvas.width = 160;
      canvas.height = 160;
      const ctx = canvas.getContext("2d");
      if (ctx) {
        ctx.drawImage(video, 0, 0, 160, 160);
        model.draftPhotoDataUrl = canvas.toDataURL("image/webp", 0.8);
        showToast("Fotografija proizvoda spremljena kao ikona!");
      }
    };
  }

  const inpModalPhoto = document.getElementById("inp-modal-photo") as HTMLInputElement | null;
  if (inpModalPhoto) {
    inpModalPhoto.onchange = async () => {
      const file = inpModalPhoto.files?.[0];
      if (!file) return;
      if (window.BarcodeDetector) {
        try {
          const bitmap = await createImageBitmap(file);
          const detector = new window.BarcodeDetector({
            formats: ["ean_13", "ean_8", "upc_a", "upc_e", "code_128", "qr_code"],
          });
          const hits = await detector.detect(bitmap);
          const code = hits.find((h) => h.rawValue?.trim())?.rawValue?.trim();
          if (code) {
            model.draftModalBarcode = code;
            const modalInp = document.getElementById("inp-modal-barcode") as HTMLInputElement | null;
            if (modalInp) modalInp.value = code;
          }
        } catch {
          // Slika nema čitljiv barkod; nastavi s postavljanjem ikone.
        }
      }
      const reader = new FileReader();
      reader.onload = () => {
        model.draftPhotoDataUrl = typeof reader.result === "string" ? reader.result : null;
        showToast("Slika proizvoda postavljena kao ikona!");
      };
      reader.readAsDataURL(file);
    };
  }

  const inpModalBarcode = document.getElementById("inp-modal-barcode") as HTMLInputElement | null;
  if (inpModalBarcode) {
    inpModalBarcode.oninput = () => {
      model.draftModalBarcode = inpModalBarcode.value;
    };
  }

  const btnConfirmScan = document.getElementById("btn-confirm-scan");
  if (btnConfirmScan) {
    btnConfirmScan.onclick = async () => {
      const code =
        (document.getElementById("inp-modal-barcode") as HTMLInputElement | null)?.value.trim() ||
        model.draftModalBarcode.trim();
      if (!code) {
        stopScannerCamera();
        model.scannerOpen = false;
        render();
        return;
      }
      await applyScannedBarcode(code);
    };
  }

  const storeSelect = document.getElementById("cart-store-select") as HTMLSelectElement | null;
  if (storeSelect) {
    storeSelect.onchange = () => {
      model.selectedStoreId = Number(storeSelect.value);
    };
  }

  const barcodeInp = document.getElementById("inp-barcode") as HTMLInputElement | null;
  const nameInp = document.getElementById("inp-name") as HTMLInputElement | null;
  const qtyInp = document.getElementById("inp-qty") as HTMLInputElement | null;
  const unitInp = document.getElementById("inp-unit") as HTMLSelectElement | null;
  const priceInp = document.getElementById("inp-price") as HTMLInputElement | null;

  if (nameInp) {
    nameInp.oninput = () => {
      model.draftName = nameInp.value;
    };
  }
  if (qtyInp) {
    qtyInp.oninput = () => {
      model.draftQty = qtyInp.value;
    };
  }
  if (unitInp) {
    unitInp.onchange = () => {
      model.draftUnit = unitInp.value as Unit;
    };
  }
  if (priceInp) {
    priceInp.oninput = () => {
      model.draftPrice = priceInp.value;
    };
  }

  if (barcodeInp) {
    barcodeInp.oninput = () => {
      model.draftBarcode = barcodeInp.value;
    };
    barcodeInp.onchange = async () => {
      const code = barcodeInp.value.trim();
      model.draftBarcode = code;
      if (!code) return;
      const found = await getProduct(code);
      if (found) {
        model.draftName = found.name;
        model.draftUnit = found.default_unit;
        if (found.last_price_cents > 0) {
          model.draftPrice = (found.last_price_cents / 100).toFixed(2);
        }
        model.draftPhotoDataUrl = found.image_data_url ?? null;
        if (nameInp) nameInp.value = model.draftName;
        if (unitInp) unitInp.value = model.draftUnit;
        if (priceInp && model.draftPrice) priceInp.value = model.draftPrice;
        showToast(`Prepoznat artikl iz baze: ${found.name}`);
      }
    };
  }

  const btnOnline = document.getElementById("btn-online-lookup");
  if (btnOnline && barcodeInp) {
    btnOnline.onclick = async () => {
      const code = barcodeInp.value.trim();
      model.draftBarcode = code;
      if (!code) return showToast("Unesite barkod za online pretragu.");
      try {
        const info = await lookupBarcodeOnline(code);
        if (info) {
          model.draftName = info.name;
          if (nameInp) nameInp.value = info.name;
          showToast(`Pronađeno online (${info.source}): ${info.name}`);
        } else {
          showToast("Artikl nije pronađen u Open Food Facts bazi.");
        }
      } catch (e) {
        showToast(`Greška online pretrage: ${e}`);
      }
    };
  }

  const btnAddCart = document.getElementById("btn-add-cart");
  if (btnAddCart) {
    btnAddCart.onclick = () => {
      const barcode = (barcodeInp?.value ?? model.draftBarcode).trim();
      const name = (nameInp?.value ?? model.draftName).trim();
      const qty = Number.parseFloat(qtyInp?.value ?? model.draftQty) || 1;
      const unit = ((unitInp?.value as Unit) ?? model.draftUnit) || "kom";
      const priceCents = parseEuroToCents(priceInp?.value ?? model.draftPrice);

      if (!name || priceCents <= 0) {
        return showToast("Unesite naziv artikla i valjanu cijenu.");
      }

      const item: CartItem = {
        id: crypto.randomUUID(),
        barcode,
        name,
        quantity: qty,
        unit,
        unit_price_cents: priceCents,
        total_cents: calcLineCents(qty, unit, priceCents),
        image_data_url: model.draftPhotoDataUrl,
      };
      model.cart.push(item);
      model.draftBarcode = "";
      model.draftName = "";
      model.draftQty = "1";
      model.draftUnit = "kom";
      model.draftPrice = "";
      model.draftPhotoDataUrl = null;
      render();
    };
  }

  document.querySelectorAll<HTMLButtonElement>("[data-remove-cart]").forEach((btn) => {
    btn.onclick = () => {
      const id = btn.dataset.removeCart;
      model.cart = model.cart.filter((i) => i.id !== id);
      render();
    };
  });

  const inpReceiptTotal = document.getElementById("inp-receipt-total") as HTMLInputElement | null;
  if (inpReceiptTotal) {
    inpReceiptTotal.oninput = () => {
      model.receiptTotalInput = inpReceiptTotal.value;
      render();
      const restored = document.getElementById("inp-receipt-total") as HTMLInputElement | null;
      if (restored) {
        restored.focus();
        restored.setSelectionRange(restored.value.length, restored.value.length);
      }
    };
  }

  const btnPrint = document.getElementById("btn-print-receipt");
  if (btnPrint) btnPrint.onclick = () => window.print();

  const btnSaveReceipt = document.getElementById("btn-save-receipt");
  if (btnSaveReceipt) {
    btnSaveReceipt.onclick = async () => {
      if (model.cart.length === 0) return showToast("Košarica je prazna.");
      const store = activeStore();
      const receiptCents = model.receiptTotalInput
        ? parseEuroToCents(model.receiptTotalInput)
        : cartTotalCents();
      try {
        const saved = await saveReceipt(
          store?.id ?? 1,
          store?.name ?? "Trgovina",
          model.cart,
          receiptCents,
        );
        model.receipts.unshift(saved);
        model.products = await listProducts();
        const ver = await verifyReceipt(saved.cart_total_cents, saved.receipt_total_cents);
        model.cart = [];
        model.receiptTotalInput = "";
        render();
        showToast(
          ver.is_matched
            ? "Račun spremljen — uneseni iznosi se podudaraju."
            : "Račun spremljen s razlikom unesenih iznosa.",
        );
      } catch (e) {
        showToast(`Greška spremanja: ${e}`);
      }
    };
  }

  const btnExcel = document.getElementById("btn-export-excel");
  if (btnExcel) {
    btnExcel.onclick = async () => {
      const store = activeStore();
      const receipt: ReceiptSummary = {
        id: Date.now(),
        sync_id: crypto.randomUUID(),
        store_id: store?.id ?? 1,
        store_name: store?.name ?? "Trgovina",
        created_at: new Date().toISOString(),
        items: model.cart,
        cart_total_cents: cartTotalCents(),
        receipt_total_cents: model.receiptTotalInput
          ? parseEuroToCents(model.receiptTotalInput)
          : cartTotalCents(),
        difference_cents: 0,
      };
      try {
        const path = await exportReceiptXlsx(receipt);
        showToast(`Excel spremljen: ${path}`);
      } catch (e) {
        showToast(`Excel izvoz nije uspio: ${e}`);
      }
    };
  }

  const inpStoreName = document.getElementById("inp-store-name") as HTMLInputElement | null;
  const inpStoreLoc = document.getElementById("inp-store-loc") as HTMLInputElement | null;
  if (inpStoreName) {
    inpStoreName.oninput = () => {
      model.draftStoreName = inpStoreName.value;
    };
  }
  if (inpStoreLoc) {
    inpStoreLoc.oninput = () => {
      model.draftStoreLoc = inpStoreLoc.value;
    };
  }

  const btnAddStore = document.getElementById("btn-add-store");
  if (btnAddStore) {
    btnAddStore.onclick = async () => {
      const name = (inpStoreName?.value ?? model.draftStoreName).trim();
      const loc = (inpStoreLoc?.value ?? model.draftStoreLoc).trim();
      if (!name) return showToast("Unesite naziv trgovine.");
      try {
        await addStore(name, loc, "#1768D1");
        await refreshConflictData();
        model.draftStoreName = "";
        model.draftStoreLoc = "";
        render();
        showToast(`Trgovina "${name}" dodana!`);
      } catch (e) {
        showToast(`Greška: ${e}`);
      }
    };
  }

  const inpCatBarcode = document.getElementById("inp-cat-barcode") as HTMLInputElement | null;
  const inpCatName = document.getElementById("inp-cat-name") as HTMLInputElement | null;
  const inpCatPrice = document.getElementById("inp-cat-price") as HTMLInputElement | null;
  if (inpCatBarcode) {
    inpCatBarcode.oninput = () => {
      model.draftCatBarcode = inpCatBarcode.value;
    };
  }
  if (inpCatName) {
    inpCatName.oninput = () => {
      model.draftCatName = inpCatName.value;
    };
  }
  if (inpCatPrice) {
    inpCatPrice.oninput = () => {
      model.draftCatPrice = inpCatPrice.value;
    };
  }

  const photoInput = document.getElementById("inp-cat-photo") as HTMLInputElement | null;
  if (photoInput) {
    photoInput.onchange = () => {
      const file = photoInput.files?.[0];
      if (!file) return;
      const reader = new FileReader();
      reader.onload = () => {
        model.draftPhotoDataUrl = typeof reader.result === "string" ? reader.result : null;
        showToast("Slika proizvoda učitana!");
      };
      reader.readAsDataURL(file);
    };
  }

  const btnSaveProduct = document.getElementById("btn-save-product");
  if (btnSaveProduct) {
    btnSaveProduct.onclick = async () => {
      const barcode = (inpCatBarcode?.value ?? model.draftCatBarcode).trim();
      const name = (inpCatName?.value ?? model.draftCatName).trim();
      const priceCents = parseEuroToCents(inpCatPrice?.value ?? model.draftCatPrice);
      if (!barcode || !name) return showToast("Unesite barkod i naziv artikla.");
      try {
        await upsertProduct({
          barcode,
          name,
          category: "Opće",
          defaultUnit: "kom",
          priceCents,
          storeName: activeStore()?.name ?? "Super Konzum",
          imageDataUrl: model.draftPhotoDataUrl,
        });
        model.draftCatBarcode = "";
        model.draftCatName = "";
        model.draftCatPrice = "";
        model.draftPhotoDataUrl = null;
        model.products = await listProducts(model.productSearch);
        render();
        showToast(`Artikl "${name}" spremljen!`);
      } catch (e) {
        showToast(`Greška: ${e}`);
      }
    };
  }

  const activeListSelect = document.getElementById("active-list-select") as HTMLSelectElement | null;
  if (activeListSelect) {
    activeListSelect.onchange = () => {
      model.selectedListId = Number(activeListSelect.value);
      render();
    };
  }

  const btnNewList = document.getElementById("btn-new-list");
  if (btnNewList) {
    btnNewList.onclick = () => {
      model.newListOpen = !model.newListOpen;
      render();
    };
  }

  const inpNewList = document.getElementById("inp-new-list") as HTMLInputElement | null;
  if (inpNewList) {
    inpNewList.oninput = () => {
      model.draftNewList = inpNewList.value;
    };
  }

  const btnCreateList = document.getElementById("btn-create-list");
  if (btnCreateList) {
    btnCreateList.onclick = async () => {
      const name = (inpNewList?.value ?? model.draftNewList).trim();
      const rawStore = (document.getElementById("sel-list-store") as HTMLSelectElement | null)?.value ?? "";
      const storeId = rawStore ? Number(rawStore) : null;
      if (!name) return showToast("Unesite naziv popisa.");
      const created = await createShoppingList(name, storeId);
      model.draftNewList = "";
      model.newListOpen = false;
      model.selectedListId = created.id;
      model.lists = await listShoppingLists();
      render();
    };
  }

  const inpPlanName = document.getElementById("inp-plan-name") as HTMLInputElement | null;
  const inpPlanQty = document.getElementById("inp-plan-qty") as HTMLInputElement | null;
  const inpPlanUnit = document.getElementById("inp-plan-unit") as HTMLSelectElement | null;
  const inpPlanPrice = document.getElementById("inp-plan-price") as HTMLInputElement | null;
  if (inpPlanName) {
    inpPlanName.oninput = () => {
      model.draftPlanName = inpPlanName.value;
    };
  }
  if (inpPlanQty) {
    inpPlanQty.oninput = () => {
      model.draftPlanQty = inpPlanQty.value;
    };
  }
  if (inpPlanUnit) {
    inpPlanUnit.onchange = () => {
      model.draftPlanUnit = inpPlanUnit.value as Unit;
    };
  }
  if (inpPlanPrice) {
    inpPlanPrice.oninput = () => {
      model.draftPlanPrice = inpPlanPrice.value;
    };
  }

  const btnAddPlanned = document.getElementById("btn-add-planned");
  const activePlanList = currentList();
  if (btnAddPlanned && activePlanList) {
    btnAddPlanned.onclick = async () => {
      const name = (inpPlanName?.value ?? model.draftPlanName).trim();
      const qty = Number.parseFloat(inpPlanQty?.value ?? model.draftPlanQty) || 1;
      const unit = ((inpPlanUnit?.value as Unit) ?? model.draftPlanUnit) || "kom";
      const priceCents = parseEuroToCents(inpPlanPrice?.value ?? model.draftPlanPrice);
      if (!name) return;
      await addPlannedItem({
        listId: activePlanList.id,
        name,
        quantity: qty,
        unit,
        estimatedPriceCents: priceCents,
      });
      model.draftPlanName = "";
      model.draftPlanQty = "1";
      model.draftPlanUnit = "kom";
      model.draftPlanPrice = "";
      model.lists = await listShoppingLists();
      render();
    };
  }

  document.querySelectorAll<HTMLInputElement>("[data-toggle-plan]").forEach((chk) => {
    chk.onchange = async () => {
      await togglePlannedItem(Number(chk.dataset.togglePlan), chk.checked);
      model.lists = await listShoppingLists();
      render();
    };
  });

  const btnTransferCart = document.getElementById("btn-transfer-cart");
  if (btnTransferCart && activePlanList) {
    btnTransferCart.onclick = () => {
      const list = activePlanList;
      for (const item of list.items) {
        if (!item.checked) {
          model.cart.push({
            id: crypto.randomUUID(),
            barcode: item.barcode ?? "",
            name: item.name,
            quantity: item.quantity,
            unit: item.unit,
            unit_price_cents: item.estimated_price_cents,
            total_cents: calcLineCents(item.quantity, item.unit, item.estimated_price_cents),
            image_data_url: null,
          });
        }
      }
      if (list.store_id) {
        model.selectedStoreId = list.store_id;
      }
      model.tab = "kosarica";
      render();
      showToast(`Prebačene stavke s popisa "${list.name}" u košaricu!`);
    };
  }

  const receiptSearchInp = document.getElementById("receipt-search") as HTMLInputElement | null;
  if (receiptSearchInp) {
    receiptSearchInp.oninput = () => {
      model.receiptSearch = receiptSearchInp.value;
      render();
      const restored = document.getElementById("receipt-search") as HTMLInputElement | null;
      if (restored) {
        restored.focus();
        restored.setSelectionRange(restored.value.length, restored.value.length);
      }
    };
  }

  const receiptStoreSel = document.getElementById("receipt-store") as HTMLSelectElement | null;
  if (receiptStoreSel) {
    receiptStoreSel.onchange = () => {
      model.receiptStore = receiptStoreSel.value;
      render();
    };
  }

  document.querySelectorAll<HTMLButtonElement>("[data-receipt-id]").forEach((btn) => {
    btn.onclick = () => {
      const id = Number(btn.dataset.receiptId);
      model.selectedReceipt = model.receipts.find((r) => r.id === id) ?? null;
      render();
    };
  });

  const btnPrintSaved = document.getElementById("btn-print-saved");
  if (btnPrintSaved) {
    btnPrintSaved.onclick = () => window.print();
  }

  const btnExportSaved = document.getElementById("btn-export-saved");
  if (btnExportSaved && model.selectedReceipt) {
    btnExportSaved.onclick = async () => {
      if (!model.selectedReceipt) return;
      try {
        const path = await exportReceiptXlsx(model.selectedReceipt);
        showToast(`Excel spremljen: ${path}`);
      } catch (e) {
        showToast(`Excel izvoz nije uspio: ${e}`);
      }
    };
  }

  const comparisonSourceSel = document.getElementById("comparison-source") as HTMLSelectElement | null;
  if (comparisonSourceSel) {
    comparisonSourceSel.onchange = () => {
      model.comparisonSource = comparisonSourceSel.value;
      void refreshComparisons();
    };
  }

  const btnRefreshPrices = document.getElementById("btn-refresh-prices");
  if (btnRefreshPrices) {
    btnRefreshPrices.onclick = () => {
      void refreshComparisons();
    };
  }

  const inpCatSearch = document.getElementById("inp-cat-search") as HTMLInputElement | null;
  if (inpCatSearch) {
    inpCatSearch.oninput = async () => {
      model.productSearch = inpCatSearch.value;
      model.products = await listProducts(model.productSearch);
      render();
      const restored = document.getElementById("inp-cat-search") as HTMLInputElement | null;
      if (restored) {
        restored.focus();
        restored.setSelectionRange(restored.value.length, restored.value.length);
      }
    };
  }

  document.querySelectorAll<HTMLElement>("[data-inspect-product]").forEach((row) => {
    row.onclick = async () => {
      const barcode = row.dataset.inspectProduct;
      if (!barcode) return;
        try {
          const prices = await getProductStorePrices(barcode);
          model.selectedProduct = model.products.find((p) => p.barcode === barcode) ?? null;
          model.selectedProductPrices = prices;
          render();
        } catch (e) { showToast(`Odaberi zapis u pregledu duplikata: ${e}`); }
    };
  });

  const btnCloseDetail = document.getElementById("btn-close-detail");
  if (btnCloseDetail) {
    btnCloseDetail.onclick = () => {
      model.selectedProduct = null;
      model.selectedProductPrices = [];
      render();
    };
  }

  const btnSavePb = document.getElementById("btn-save-pb");
  if (btnSavePb) {
    btnSavePb.onclick = async () => {
      const url = (document.getElementById("inp-pb-url") as HTMLInputElement).value.trim();
      model.pocketbaseUrl = url;
      await savePocketbaseUrl(url);
      showToast("PocketBase adresa spremljena.");
    };
  }

  const btnSyncPb = document.getElementById("btn-sync-pb");
  if (btnSyncPb) {
    btnSyncPb.onclick = async () => {
      const url = (document.getElementById("inp-pb-url") as HTMLInputElement).value.trim();
      try {
        const rep = await syncPocketbase(url);
        model.stores = await listStores();
        render();
        showToast(`Sinkronizirano! Poslano ${rep.pushed_stores} trgovina, ${rep.pushed_products} artikala.`);
      } catch (e) {
        showToast(`Sinkronizacija nije uspjela: ${e}`);
      }
    };
  }

  const inpNetworkName = document.getElementById("inp-network-name") as HTMLInputElement | null;
  const inpMemberName = document.getElementById("inp-member-name") as HTMLInputElement | null;
  const selMemberRole = document.getElementById("sel-member-role") as HTMLSelectElement | null;

  if (inpNetworkName) {
    inpNetworkName.oninput = () => {
      model.draftNetworkName = inpNetworkName.value;
    };
  }
  if (inpMemberName) {
    inpMemberName.oninput = () => {
      model.draftMemberName = inpMemberName.value;
    };
  }
  if (selMemberRole) {
    selMemberRole.onchange = () => {
      model.pairingRole = selMemberRole.value as PeerRole;
      render();
    };
  }

  const btnOpenPairing = document.getElementById("btn-open-pairing");
  if (btnOpenPairing) {
    btnOpenPairing.onclick = async () => {
      model.pairingOpen = true;
      model.pairingConfirmed = false;
      if (model.draftMemberLists === null) {
        model.draftMemberLists = model.lists.map((l) => l.sync_id).join(",");
      }
      model.pairStatus = await getPairStatus().catch(() => null);
      render();
    };
  }

  document.querySelectorAll<HTMLButtonElement>("[data-pair-mode]").forEach((btn) => {
    btn.onclick = () => {
      const mode = btn.dataset.pairMode;
      if (mode === "lan" || mode === "offline") {
        model.pairMode = mode;
        render();
      }
    };
  });

  const inpDeviceName = document.getElementById("inp-device-name") as HTMLInputElement | null;
  if (inpDeviceName) {
    inpDeviceName.oninput = () => {
      model.draftDeviceName = inpDeviceName.value;
    };
  }

  const inpPairAddr = document.getElementById("inp-pair-addr") as HTMLInputElement | null;
  if (inpPairAddr) {
    inpPairAddr.oninput = () => {
      model.draftPairAddr = inpPairAddr.value;
    };
  }

  const stopPairPoll = () => {
    if (pairPollTimer !== null) {
      window.clearInterval(pairPollTimer);
      pairPollTimer = null;
    }
  };

  const ensurePairPoll = () => {
    if (pairPollTimer !== null) return;
    pairPollTimer = window.setInterval(async () => {
      if (!model.pairingOpen) {
        stopPairPoll();
        return;
      }
      const next = await getPairStatus().catch(() => null);
      if (!next) return;
      const changed =
        next.pending !== model.pairStatus?.pending ||
        next.waiting !== model.pairStatus?.waiting ||
        next.sas !== model.pairStatus?.sas ||
        next.error !== model.pairStatus?.error;
      if (changed) {
        model.pairStatus = next;
        if (next.pending || !next.waiting) stopPairPoll();
        render();
      }
    }, 600);
  };

  const btnStartPairListen = document.getElementById("btn-start-pair-listen");
  if (btnStartPairListen) {
    btnStartPairListen.onclick = async () => {
      model.pairingBusy = true;
      render();
      try {
        model.pairStatus = await startPairListening({
          deviceName: model.draftDeviceName || null,
        });
        ensurePairPoll();
      } catch (e) {
        showToast(`Greška pokretanja uparivanja: ${e}`);
      } finally {
        model.pairingBusy = false;
        render();
      }
    };
  }

  const btnStopPairListen = document.getElementById("btn-stop-pair-listen");
  if (btnStopPairListen) {
    btnStopPairListen.onclick = async () => {
      stopPairPoll();
      model.pairStatus = await stopPairListening().catch(() => null);
      render();
    };
  }

  const btnDiscoverPairPeers = document.getElementById("btn-discover-pair-peers");
  if (btnDiscoverPairPeers) {
    btnDiscoverPairPeers.onclick = async () => {
      model.pairingBusy = true;
      render();
      try {
        model.lanPeersDiscovered = await discoverLanPeers();
        if (!model.lanPeersDiscovered.some((p) => (p.pairPort ?? 0) > 0)) {
          showToast("Nije pronađen nijedan uređaj koji čeka uparivanje na LAN-u.");
        }
      } catch (e) {
        showToast(`Greška LAN pretrage: ${e}`);
      } finally {
        model.pairingBusy = false;
        render();
      }
    };
  }

  const startLiveDial = async (addr: string) => {
    if (!addr.trim()) return showToast("Unesite IP:port drugog uređaja.");
    model.pairingBusy = true;
    render();
    try {
      model.pairStatus = await dialPairPeer({
        addr: addr.trim(),
        deviceName: model.draftDeviceName || null,
      });
      model.pairingConfirmed = false;
    } catch (e) {
      showToast(`Greška spajanja na uređaj: ${e}`);
    } finally {
      model.pairingBusy = false;
      render();
    }
  };

  document.querySelectorAll<HTMLButtonElement>("[data-dial-discovered]").forEach((btn) => {
    btn.onclick = () => {
      const addr = btn.dataset.dialDiscovered;
      if (addr) void startLiveDial(addr);
    };
  });

  const btnDialPair = document.getElementById("btn-dial-pair");
  if (btnDialPair) {
    btnDialPair.onclick = () => {
      void startLiveDial(inpPairAddr?.value ?? model.draftPairAddr);
    };
  }

  const btnConfirmLivePairing = document.getElementById("btn-confirm-live-pairing");
  if (btnConfirmLivePairing) {
    btnConfirmLivePairing.onclick = async () => {
      if (!model.pairingConfirmed || model.pairingBusy) return;
      const role = ((selMemberRole?.value as PeerRole) ?? model.pairingRole) || "family_member";
      const sharedLists =
        role === "own_device"
          ? "*"
          : (model.draftMemberLists ?? "").split(",").filter(Boolean).join(",");
      if (role === "family_member" && !sharedLists) {
        return showToast("Odaberite barem jedan popis za dijeljenje.");
      }
      model.pairingBusy = true;
      render();
      try {
        const outcome = await confirmPairPeer({
          approved: true,
          role,
          sharedLists,
          admit: model.pairStatus?.can_admit ?? true,
        });
        stopPairPoll();
        model.pairStatus = await getPairStatus().catch(() => null);
        model.pairingOpen = false;
        model.pairingConfirmed = false;
        model.draftMemberLists = null;
        model.members = await listNetworkMembers();
        model.lanStatus = await getLanStatus().catch(() => model.lanStatus);
        showToast(outcome.message);
      } catch (e) {
        showToast(`Greška potvrde uparivanja: ${e}`);
      } finally {
        model.pairingBusy = false;
        render();
      }
    };
  }

  const btnRejectLivePairing = document.getElementById("btn-reject-live-pairing");
  if (btnRejectLivePairing) {
    btnRejectLivePairing.onclick = async () => {
      model.pairingBusy = true;
      render();
      try {
        await confirmPairPeer({ approved: false });
      } catch {
        // Ignore error if peer already closed
      } finally {
        stopPairPoll();
        model.pairStatus = await getPairStatus().catch(() => null);
        model.pairingConfirmed = false;
        model.pairingBusy = false;
        render();
        showToast("Uparivanje je odbijeno.");
      }
    };
  }

  const btnLanStart = document.getElementById("btn-lan-start");
  if (btnLanStart) {
    btnLanStart.onclick = async () => {
      model.lanBusy = true;
      render();
      try {
        model.lanStatus = await startLanSync();
        showToast("LAN servis pokrenut.");
      } catch (e) {
        showToast(`Greška pokretanja LAN servisa: ${e}`);
      } finally {
        model.lanBusy = false;
        render();
      }
    };
  }

  const btnLanStop = document.getElementById("btn-lan-stop");
  if (btnLanStop) {
    btnLanStop.onclick = async () => {
      model.lanBusy = true;
      render();
      try {
        model.lanStatus = await stopLanSync();
        showToast("LAN servis zaustavljen.");
      } catch (e) {
        showToast(`Greška zaustavljanja LAN servisa: ${e}`);
      } finally {
        model.lanBusy = false;
        render();
      }
    };
  }

  const btnLanPause = document.getElementById("btn-lan-pause");
  if (btnLanPause) {
    btnLanPause.onclick = async () => {
      const nextPaused = !(model.lanStatus?.paused ?? false);
      try {
        model.lanStatus = await setLanPaused(nextPaused);
        render();
        showToast(nextPaused ? "LAN razmjena pauzirana." : "LAN razmjena nastavljena.");
      } catch (e) {
        showToast(`Greška promjene stanja LAN servisa: ${e}`);
      }
    };
  }

  const btnLanSyncNow = document.getElementById("btn-lan-sync-now");
  if (btnLanSyncNow) {
    btnLanSyncNow.onclick = async () => {
      model.lanBusy = true;
      render();
      try {
        const synced = await triggerLanSync();
        await refreshConflictData();
        model.members = await listNetworkMembers();
        model.lanStatus = await getLanStatus().catch(() => model.lanStatus);
        showToast(`LAN razmjena dovršena (sinkroniziranih partnera: ${synced}).`);
      } catch (e) {
        showToast(`Greška LAN razmjene: ${e}`);
      } finally {
        model.lanBusy = false;
        render();
      }
    };
  }

  document.querySelectorAll<HTMLInputElement>("[data-manual-addr-input]").forEach((inp) => {
    inp.oninput = () => {
      const key = inp.dataset.manualAddrInput;
      if (key) model.draftManualAddrs[key] = inp.value;
    };
  });

  document.querySelectorAll<HTMLButtonElement>("[data-save-manual-addr]").forEach((btn) => {
    btn.onclick = async () => {
      const key = btn.dataset.saveManualAddr;
      if (!key) return;
      const addr = (model.draftManualAddrs[key] ?? "").trim();
      try {
        await setManualPeerAddr(key, addr || null);
        model.lanStatus = await getLanStatus().catch(() => model.lanStatus);
        render();
        showToast(addr ? `Ručna adresa spremljena: ${addr}` : "Ručna adresa uklonjena.");
      } catch (e) {
        showToast(`Greška adrese partnera: ${e}`);
      }
    };
  });

  document.querySelectorAll<HTMLButtonElement>("[data-sync-peer-at]").forEach((btn) => {
    btn.onclick = async () => {
      const key = btn.dataset.syncPeerAt;
      if (!key) return;
      const rec = peerSyncRecord(key);
      const addr = (model.draftManualAddrs[key] ?? rec?.manual_addr ?? rec?.last_addr ?? "").trim();
      if (!addr) return showToast("Unesite IP:port adresu partnera.");
      model.lanBusy = true;
      render();
      try {
        const res = await syncLanPeerAt(key, addr);
        await refreshConflictData();
        model.members = await listNetworkMembers();
        model.lanStatus = await getLanStatus().catch(() => model.lanStatus);
        showToast(`Razmjena dovršena: primljeno ${res.total_applied_versions}, poslano ${res.total_peer_applied_versions}.`);
      } catch (e) {
        model.lanStatus = await getLanStatus().catch(() => model.lanStatus);
        showToast(`Greška razmjene s partnerom: ${e}`);
      } finally {
        model.lanBusy = false;
        render();
      }
    };
  });

  document.querySelectorAll<HTMLInputElement>("[data-pair-list]").forEach((chk) => {
    chk.onchange = () => {
      const syncId = chk.dataset.pairList;
      if (!syncId) return;
      const current = new Set((model.draftMemberLists ?? "").split(",").filter(Boolean));
      if (chk.checked) {
        current.add(syncId);
      } else {
        current.delete(syncId);
      }
      model.draftMemberLists = [...current].join(",");
      render();
    };
  });

  const pairingConfirmedChk = document.getElementById("pairing-confirmed") as HTMLInputElement | null;
  if (pairingConfirmedChk) {
    pairingConfirmedChk.onchange = () => {
      model.pairingConfirmed = pairingConfirmedChk.checked;
      render();
    };
  }

  const btnRejectPairing = document.getElementById("btn-reject-pairing");
  if (btnRejectPairing) {
    btnRejectPairing.onclick = async () => {
      model.pairingConfirmed = false;
      model.pairingOpen = false;
      model.pendingSasCode = await newSasCode();
      render();
      showToast("Uparivanje odbijeno — generiran je novi kod.");
    };
  }

  document.querySelectorAll<HTMLInputElement>("[data-export-list]").forEach((chk) => {
    chk.onchange = () => {
      const syncId = chk.dataset.exportList;
      if (!syncId) return;
      if (chk.checked) {
        if (!model.exportListIds.includes(syncId)) {
          model.exportListIds = [...model.exportListIds, syncId];
        }
      } else {
        model.exportListIds = model.exportListIds.filter((id) => id !== syncId);
      }
      render();
    };
  });

  const btnExportBundle = document.getElementById("btn-export-bundle");
  if (btnExportBundle) {
    btnExportBundle.onclick = async () => {
      await trackPackageExport("own_device");
    };
  }

  const btnExportFamilyBundle = document.getElementById("btn-export-family-bundle");
  if (btnExportFamilyBundle) {
    btnExportFamilyBundle.onclick = async () => {
      const sharedFilter = model.exportListIds.join(",") || null;
      await trackPackageExport("family_member", sharedFilter);
    };
  }

  const btnSaveNetName = document.getElementById("btn-save-net-name");
  if (btnSaveNetName) {
    btnSaveNetName.onclick = async () => {
      const val = inpNetworkName?.value ?? model.draftNetworkName ?? "";
      await setNetworkName(val);
      model.draftNetworkName = null;
      model.ledgerStats = await getLedgerStats();
      render();
      showToast(`Naziv mreže spremljen: ${model.ledgerStats.network_name}`);
    };
  }

  const btnAddMember = document.getElementById("btn-add-member");
  if (btnAddMember) {
    btnAddMember.onclick = async () => {
      if (!model.pairingConfirmed || model.pairingBusy) return;
      const name = (inpMemberName?.value ?? model.draftMemberName).trim();
      const role = ((selMemberRole?.value as PeerRole) ?? model.pairingRole) || "family_member";
      const sharedLists =
        role === "own_device"
          ? "*"
          : (model.draftMemberLists ?? "").split(",").filter(Boolean).join(",");
      if (!name) return showToast("Unesite naziv uređaja ili člana obitelji.");
      if (role === "family_member" && !sharedLists) {
        return showToast("Odaberite barem jedan popis za dijeljenje.");
      }
      model.pairingBusy = true;
      render();
      try {
        await addNetworkMember({
          name,
          role,
          sasCode: model.pendingSasCode,
          sharedLists,
        });
        model.draftMemberName = "";
        model.draftMemberLists = null;
        model.pairingOpen = false;
        model.pairingConfirmed = false;
        model.members = await listNetworkMembers();
        model.pendingSasCode = await newSasCode();
        showToast(
          role === "own_device"
            ? `Uređaj "${name}" dodan u evidenciju.`
            : `Član obitelji "${name}" dodan u evidenciju.`,
        );
      } catch (e) {
        showToast(`Greška dodavanja u mrežu: ${e}`);
      } finally {
        model.pairingBusy = false;
        render();
      }
    };
  }

  document.querySelectorAll<HTMLButtonElement>("[data-member-access]").forEach((btn) => {
    btn.onclick = () => {
      const peerId = btn.dataset.memberAccess;
      model.accessMember = model.members.find((m) => m.peer_id === peerId) ?? null;
      model.removalMember = null;
      render();
    };
  });

  const btnReviewRemoveMember = document.getElementById("btn-review-remove-member");
  if (btnReviewRemoveMember) {
    btnReviewRemoveMember.onclick = () => {
      model.removalMember = model.accessMember;
      render();
    };
  }

  document.querySelectorAll<HTMLButtonElement>("[data-remove-member]").forEach((btn) => {
    btn.onclick = async () => {
      const peerId = btn.dataset.removeMember;
      if (!peerId) return;
      await removeNetworkMember(peerId);
      model.removalMember = null;
      model.accessMember = null;
      model.members = await listNetworkMembers();
      model.conflicts = await identityConflicts();
      render();
      showToast("Član/uređaj uklonjen iz evidencije.");
    };
  });

  const inpImportBundle = document.getElementById("inp-import-bundle") as HTMLInputElement | null;
  if (inpImportBundle) {
    inpImportBundle.onchange = async () => {
      const file = inpImportBundle.files?.[0];
      if (!file) return;
      let text = "";
      let counts: [number,number,number] = [0,0,0];
      await syncTracker.run("Uvoz paketa",[
        {label:"Čitanje datoteke",action:async () => {text=await file.text();}},
        {label:"Provjera i primjena podataka u bazu",commitsData:true,action:async () => {counts=await importSyncBundleJson(text);}},
        {label:"Provjera cjelovitosti računa i osvježavanje prikaza",action:async () => {await refreshConflictData();model.conflictReview=null;}},
      ],() => `Uvezeno: ${counts[0]} trgovina, ${counts[1]} artikala, ${counts[2]} popisa.${model.incompleteReceipts ? ` Računi koji još čekaju potpune podatke: ${model.incompleteReceipts}.` : ""}`);
    };
  }

  const btnCompact = document.getElementById("btn-compact-ledger");
  if (btnCompact) {
    btnCompact.onclick = async () => {
      try {
        const removed = await runLedgerCompact();
        model.ledgerStats = await getLedgerStats();
        render();
        showToast(`Knjiga verzija očišćena (uklonjeno ${removed} zastarjelih međukoraka).`);
      } catch (e) {
        showToast(`Greška čišćenja knjige verzija: ${e}`);
      }
    };
  }
}

async function init() {
  try {
    model.stores = await listStores();
    model.products = await listProducts();
    model.receipts = await listReceipts();
    model.lists = await listShoppingLists();
    model.ledgerStats = await getLedgerStats();
    model.members = await listNetworkMembers();
    model.lanStatus = await getLanStatus().catch(() => null);
    model.pendingSasCode = await newSasCode();
    model.incompleteReceipts = await incompleteReceiptCount();
    model.conflicts = await identityConflicts();
    const pbUrl = await getPocketbaseUrl();
    if (pbUrl) model.pocketbaseUrl = pbUrl;
    void listenLanStatusChanged((st) => {
      model.lanStatus = st;
      render();
    }).catch(() => {});
  } catch (e) {
    console.error("Init error:", e);
  }
  render();
}

void init();
