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
  createShoppingList,
  exportReceiptXlsx,
  exportSyncBundleJson,
  formatCents,
  getLedgerStats,
  getPocketbaseUrl,
  getProduct,
  getProductStorePrices,
  importSyncBundleJson,
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
  setNetworkName,
  syncPocketbase,
  togglePlannedItem,
  upsertProduct,
  verifyReceipt,
} from "./api";
import type { IdentityConflict } from "./api";
import type { PriceSource, RetailResults, RetailProduct, CsvInspection, CsvMapping } from "./api";
import { SyncTracker } from "./sync-progress";

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

type Tab = "pregled" | "kosarica" | "planer" | "katalog" | "trgovine" | "sync";

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
}

const model: AppModel = {
  incompleteReceipts: 0,
  conflicts: [],
  conflictReview: null,
  resolvingConflict: false,
  tab: "pregled",
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
};

let scannerDecodeTimer: number | null = null;
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

function renderSidebar(): string {
  const navItems: { id: Tab; label: string; icon: string }[] = [
    {
      id: "pregled",
      label: "Pregled",
      icon: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="3" y="3" width="7" height="9" rx="1"/><rect x="14" y="3" width="7" height="5" rx="1"/><rect x="14" y="12" width="7" height="9" rx="1"/><rect x="3" y="16" width="7" height="5" rx="1"/></svg>`,
    },
    {
      id: "kosarica",
      label: "Košarica",
      icon: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><circle cx="8" cy="21" r="1"/><circle cx="19" cy="21" r="1"/><path d="M2.05 2.05h2l2.66 12.42a2 2 0 0 0 2 1.58h9.78a2 2 0 0 0 1.95-1.57l1.65-7.43H5.12"/></svg>`,
    },
    {
      id: "planer",
      label: "Planer i popisi",
      icon: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect width="8" height="4" x="8" y="2" rx="1"/><path d="M16 4h2a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h2"/><path d="M12 11h4"/><path d="M12 16h4"/><path d="M8 11h.01"/><path d="M8 16h.01"/></svg>`,
    },
    {
      id: "katalog",
      label: "Katalog artikala",
      icon: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M12.586 2.586A2 2 0 0 0 11.172 2H4a2 2 0 0 0-2 2v7.172a2 2 0 0 0 .586 1.414l8.704 8.704a2.426 2.426 0 0 0 3.42 0l6.58-6.58a2.426 2.426 0 0 0 0-3.42z"/><circle cx="7.5" cy="7.5" r=".5" fill="currentColor"/></svg>`,
    },
    {
      id: "trgovine",
      label: "Trgovine i cijene",
      icon: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M15 21v-5a1 1 0 0 0-1-1h-4a1 1 0 0 0-1 1v5"/><path d="M17.774 10.31a1.12 1.12 0 0 0-1.549 0 2.5 2.5 0 0 1-3.451 0 1.12 1.12 0 0 0-1.548 0 2.5 2.5 0 0 1-3.452 0 1.12 1.12 0 0 0-1.549 0 2.5 2.5 0 0 1-3.77-3.248l2.889-4.184A2 2 0 0 1 7 2h10a2 2 0 0 1 1.653.873l2.895 4.192a2.5 2.5 0 0 1-3.774 3.244"/><path d="M4 10.95V19a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-8.05"/></svg>`,
    },
    {
      id: "sync",
      label: "Sinkronizacija",
      icon: `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M12 13v8l-4-4"/><path d="m12 21 4-4"/><path d="M4.393 15.269A7 7 0 1 1 15.71 8h1.79a4.5 4.5 0 0 1 2.436 8.284"/></svg>`,
    },
  ];

  return `
    <aside class="sidebar">
      <div>
        <div class="brand-box">
          ${APP_ICON_SVG}
          <div>
            <div class="brand-title">List2Cart</div>
            <div class="brand-tag">Pametna kupnja. Točan račun.</div>
          </div>
        </div>
        <nav class="nav-list">
          ${navItems
            .map(
              (n) => `
            <button class="nav-btn ${model.tab === n.id ? "active" : ""}" data-tab="${escapeHtml(n.id)}">
              ${n.icon}
              <span>${escapeHtml(n.label)}</span>
            </button>`,
            )
            .join("")}
        </nav>
      </div>
      <div class="sidebar-footer">
        <p>Uvijek točno u cent. Provjerite svaku stavku prije plaćanja na blagajni.</p>
        <div class="status-line">
          <span class="status-dot"></span>
          <span>List2Cart 0.0.1-alfa • Spreman</span>
        </div>
      </div>
    </aside>
  `;
}

function renderPregled(): string {
  const total = cartTotalCents();
  const store = activeStore();
  const lastReceipt = model.receipts[0];
  return `
    <div class="page-header">
      <h1>Vaša kupnja, pod potpunom kontrolom.</h1>
      <p>Brzi pregled košarice, planova i usklađivanja s računom na blagajni.</p>
    </div>

    <div class="card">
      <div class="item-sub">${store ? escapeHtml(store.name) : "Odaberi trgovinu"} — Aktivna košarica</div>
      <div class="big-stat">${escapeHtml(formatCents(total))} u košarici</div>
      <div class="item-sub">${model.cart.length} skeniranih artikala spremno za kontrolu</div>
      <div class="progress-track">
        <div class="progress-fill" style="width: ${model.cart.length > 0 ? "100%" : "12%"}"></div>
      </div>
      <div class="highlight-box">
        <span class="pill">SPREMNO ZA BLAGAJNU</span>
        <div style="font-size:17px;font-weight:700;">Kontrolirajte svaki račun u cent</div>
        <div class="item-sub">Skenirajte barkod, upišite količinu u komadima, gramima ili kilogramima i usporedite zbroj.</div>
        <div class="row" style="margin-top:6px;">
          <button class="btn btn-primary" data-goto="kosarica">Otvori košaricu i skener →</button>
        </div>
      </div>
    </div>

    <div class="grid-2">
      <div class="card">
        <div class="item-title">Katalog i barkodovi →</div>
        <div class="item-sub" style="margin:6px 0 12px;">${model.products.length} zapamćenih proizvoda sa slikama i cijenama.</div>
        <button class="btn btn-outline" data-goto="katalog">Pregledaj katalog</button>
      </div>
      <div class="card">
        <div class="item-title">Zadnji spremljeni račun →</div>
        <div class="item-sub" style="margin:6px 0 12px;">
          ${
            lastReceipt
              ? `${escapeHtml(lastReceipt.store_name)}: ${escapeHtml(formatCents(lastReceipt.cart_total_cents))}`
              : "Još nema spremljenih računa."
          }
        </div>
        <button class="btn btn-outline" data-goto="planer">Planiraj sljedeću kupnju</button>
      </div>
    </div>
  `;
}

function renderKosarica(): string {
  const total = cartTotalCents();
  const receiptCents = parseEuroToCents(model.receiptTotalInput);
  const diff = total - receiptCents;
  const hasReceipt = model.receiptTotalInput.trim().length > 0;

  return `
    <div class="page-header">
      <h1>Skenirajte artikle i provjerite račun.</h1>
      <p>Dodajte proizvode u komadima, gramima ili kilogramima i usporedite s blagajnom.</p>
    </div>

    <div class="card no-print">
      <div class="row-wrap" style="margin-bottom: 10px;">
        <select id="cart-store-select">
          ${model.stores
            .map(
              (s) =>
                `<option value="${escapeHtml(s.id)}" ${s.id === model.selectedStoreId ? "selected" : ""}>${escapeHtml(s.name)}</option>`,
            )
            .join("")}
        </select>
        <button class="btn btn-primary" id="btn-camera-scan">📷 Skeniraj barkod kamerom</button>
        <button class="btn btn-outline" id="btn-online-lookup">🌐 Dohvati naziv/sliku online</button>
      </div>

      <div class="row-wrap">
        <input id="inp-barcode" value="${escapeHtml(model.draftBarcode)}" placeholder="Barkod (EAN-13)" style="width:150px;" />
        <input id="inp-name" value="${escapeHtml(model.draftName)}" placeholder="Naziv artikla" style="flex:1; min-width:160px;" />
        <input id="inp-qty" type="number" step="0.001" value="${escapeHtml(model.draftQty)}" style="width:85px;" />
        <select id="inp-unit" style="width:80px;">
          <option value="kom" ${model.draftUnit === "kom" ? "selected" : ""}>kom</option>
          <option value="kg" ${model.draftUnit === "kg" ? "selected" : ""}>kg</option>
          <option value="g" ${model.draftUnit === "g" ? "selected" : ""}>g</option>
          <option value="l" ${model.draftUnit === "l" ? "selected" : ""}>L</option>
          <option value="ml" ${model.draftUnit === "ml" ? "selected" : ""}>ml</option>
        </select>
        <input id="inp-price" value="${escapeHtml(model.draftPrice)}" placeholder="Cijena (€)" style="width:105px;" />
        <button class="btn btn-primary" id="btn-add-cart">+ Dodaj</button>
      </div>
    </div>

    <div class="list-container">
      ${
        model.cart.length === 0
          ? `<div class="card item-sub">Košarica je trenutno prazna. Skenirajte barkod ili unesite artikl iznad.</div>`
          : model.cart
              .map((item) => {
                const safeImg = sanitizeImageUrl(item.image_data_url);
                return `
          <div class="list-row">
            <div class="list-row-left">
              <div class="item-thumb">
                ${safeImg ? `<img src="${escapeHtml(safeImg)}" alt="" />` : escapeHtml(item.name.slice(0, 1).toUpperCase())}
              </div>
              <div>
                <div class="item-title">${escapeHtml(item.name)}</div>
                <div class="item-sub">${escapeHtml(item.quantity)} ${escapeHtml(item.unit)} × ${escapeHtml(formatCents(item.unit_price_cents))} ${item.barcode ? `• ${escapeHtml(item.barcode)}` : ""}</div>
              </div>
            </div>
            <div class="row">
              <div class="item-price">${escapeHtml(formatCents(item.total_cents))}</div>
              <button class="btn btn-danger-ghost no-print" data-remove-cart="${escapeHtml(item.id)}">✕</button>
            </div>
          </div>`;
              })
              .join("")
      }
    </div>

    <div class="sticky-bar">
      <div>
        <div style="font-size:18px;font-weight:700;">Ukupno u košarici: ${escapeHtml(formatCents(total))}</div>
        <div class="row" style="margin-top:6px;">
          <span class="item-sub">Iznos na računu:</span>
          <input id="inp-receipt-total" value="${escapeHtml(model.receiptTotalInput)}" placeholder="0,00 €" style="width:110px;" />
          ${
            hasReceipt
              ? diff === 0
                ? `<span class="pill success">✓ Iznos odgovara računu! (0,00 €)</span>`
                : `<span class="pill warning">Razlika: ${escapeHtml(formatCents(diff))}</span>`
              : ""
          }
        </div>
      </div>
      <div class="row no-print">
        <button class="btn btn-outline" id="btn-print-receipt">🖨️ Ispis</button>
        <button class="btn btn-outline" id="btn-export-excel">📊 Excel (.xlsx)</button>
        <button class="btn btn-primary" id="btn-save-receipt">Spremi račun →</button>
      </div>
    </div>
  `;
}

function renderPlaner(): string {
  const activeList = model.lists[0];
  return `
    <div class="page-header">
      <h1>Planirajte kupnju i štedite vrijeme.</h1>
      <p>Kreirajte popise za trgovine i križajte stavke dok kupujete.</p>
    </div>

    <div class="card">
      <div class="row-wrap">
        <input id="inp-new-list" value="${escapeHtml(model.draftNewList)}" placeholder="Naziv novog popisa (npr. Tjedna nabavka)..." style="flex:1;" />
        <select id="sel-list-store">
          ${model.stores
            .map(
              (s) =>
                `<option value="${escapeHtml(s.id)}" ${s.id === model.selectedStoreId ? "selected" : ""}>${escapeHtml(s.name)}</option>`,
            )
            .join("")}
        </select>
        <button class="btn btn-primary" id="btn-create-list">+ Novi popis →</button>
      </div>
    </div>

    ${
      activeList
        ? `
      <div class="card">
        <div class="row space-between" style="margin-bottom:12px;">
          <div>
            <div class="item-title" style="font-size:17px;">${escapeHtml(activeList.name)}</div>
            <div class="item-sub">${escapeHtml(activeList.store_name ?? "Općenito")} • Procjena: ${escapeHtml(formatCents(activeList.total_estimated_cents))}</div>
          </div>
          <button class="btn btn-subtle" id="btn-transfer-cart">Prebaci u košaricu →</button>
        </div>

        <div class="row-wrap" style="margin-bottom:14px;">
          <input id="inp-plan-name" value="${escapeHtml(model.draftPlanName)}" placeholder="Artikl za popis..." style="flex:1;" />
          <input id="inp-plan-qty" type="number" step="0.1" value="${escapeHtml(model.draftPlanQty)}" style="width:80px;" />
          <select id="inp-plan-unit" style="width:75px;">
            <option value="kom" ${model.draftPlanUnit === "kom" ? "selected" : ""}>kom</option>
            <option value="kg" ${model.draftPlanUnit === "kg" ? "selected" : ""}>kg</option>
            <option value="g" ${model.draftPlanUnit === "g" ? "selected" : ""}>g</option>
            <option value="l" ${model.draftPlanUnit === "l" ? "selected" : ""}>L</option>
            <option value="ml" ${model.draftPlanUnit === "ml" ? "selected" : ""}>ml</option>
          </select>
          <input id="inp-plan-price" value="${escapeHtml(model.draftPlanPrice)}" placeholder="Procjena (€)" style="width:110px;" />
          <button class="btn btn-primary" id="btn-add-planned">+ Dodaj</button>
        </div>

        <div class="list-container">
          ${activeList.items
            .map(
              (item) => `
            <div class="list-row">
              <label class="row" style="cursor:pointer;">
                <input type="checkbox" data-toggle-plan="${escapeHtml(item.id)}" ${item.checked ? "checked" : ""} />
                <span style="${item.checked ? "text-decoration:line-through;color:var(--text-subtle);" : "font-weight:600;"}">
                  ${escapeHtml(item.name)} (${escapeHtml(item.quantity)} ${escapeHtml(item.unit)})
                </span>
              </label>
              <div class="item-price">${escapeHtml(formatCents(calcLineCents(item.quantity, item.unit, item.estimated_price_cents)))}</div>
            </div>`,
            )
            .join("")}
        </div>
      </div>`
        : `<div class="card item-sub">Kreirajte svoj prvi popis za kupnju iznad.</div>`
    }
  `;
}

function renderKatalog(): string {
  const selectedImg = sanitizeImageUrl(model.selectedProduct?.image_data_url);
  return `
    <div class="page-header">
      <h1>Artikli koje ste već skenirali.</h1>
      <p>Pretražite bazu barkodova, dodajte fotografiju proizvoda i pratite zadnje cijene.</p>
    </div>

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
    <div class="page-header">
      <h1>Vaše trgovine i omiljeni dućani.</h1>
      <p>Dodajte vlastite trgovine za praćenje cijena i kreiranje popisa po dućanu.</p>
    </div>

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

function renderSync(): string {
  const st = model.ledgerStats;
  const sizeKb = st ? (st.db_size_bytes / 1024).toFixed(1).replace(".", ",") : "0,0";
  const ownDevices = model.members.filter((m) => m.role === "own_device");
  const familyMembers = model.members.filter((m) => m.role === "family_member");
  const defaultListNames = model.lists.map((l) => l.name).join(", ") || "Sve obiteljske liste";
  const memberListsVal = model.draftMemberLists ?? defaultListNames;
  const netNameVal = model.draftNetworkName ?? st?.network_name ?? "Obiteljska mreža";

  return `
    <div class="page-header">
      <h1>Sinkronizacija: Moji uređaji i Obiteljska mreža.</h1>
      <p>S vlastitim uređajima dijelite SVE (račune, katalog, slike i liste), a s članovima obitelji samo odabrane liste za kupnju.</p>
    </div>

    <div class="card">
      <div class="row space-between" style="margin-bottom:10px;">
        <div>
          <span class="pill">AKTIVNA MREŽA • EVIDENCIJA ČLANOVA I PRAVA PRISTUPA</span>
          <div class="item-title" style="font-size:17px;margin-top:6px;">
            Mreža: ${escapeHtml(st?.network_name ?? "Obiteljska mreža")} (Ovaj čvor: ${escapeHtml(st?.node_id ?? "—")})
          </div>
        </div>
        <div class="row">
          <input id="inp-network-name" value="${escapeHtml(netNameVal)}" placeholder="Naziv mreže (npr. Obitelj Kraljević)" style="width:210px;" />
          <button class="btn btn-outline" id="btn-save-net-name">Spremi naziv mreže</button>
        </div>
      </div>

      <div class="highlight-box" style="margin-top:10px;">
        <div class="row space-between">
          <div>
            <div class="item-title">Registriraj uređaj ili člana obitelji (SAS kod za provjeru paketa)</div>
            <div class="item-sub">Odaberite ulogu — vlastiti uređaj dobiva sve kanale (<code>osobno + obitelj</code>, uključujući račune), a član obitelji samo kanal <code>obitelj</code> i odabrane popise (<code>shared_lists</code>).</div>
          </div>
          <div class="pill" style="font-size:15px;padding:6px 12px;">SAS kod: ${escapeHtml(model.pendingSasCode)}</div>
        </div>
        <div class="row-wrap" style="margin-top:8px;">
          <input id="inp-member-name" value="${escapeHtml(model.draftMemberName)}" placeholder="Naziv (npr. Moj Kućni PC ili Ana — iPhone 15)" style="flex:1;min-width:220px;" />
          <select id="sel-member-role" style="width:265px;">
            <option value="own_device" ${model.pairingRole === "own_device" ? "selected" : ""}>💻 Moj drugi uređaj (Dijeli SVE)</option>
            <option value="family_member" ${model.pairingRole === "family_member" ? "selected" : ""}>👨‍👩‍👧 Član obitelji (Dijeli SAMO liste)</option>
          </select>
          <input id="inp-member-lists" value="${escapeHtml(memberListsVal)}" placeholder="Koje liste dijeliti (za obitelj)" style="width:220px;" />
          <button class="btn btn-primary" id="btn-add-member">✓ Potvrdi SAS i dodaj →</button>
        </div>
      </div>
    </div>

    <div class="grid-2" style="margin-bottom:16px;">
      <div class="card" style="margin-bottom:0;">
        <div class="item-title">1. Moji osobni uređaji — Dijeli SVE</div>
        <div class="item-sub" style="margin:4px 0 12px;">Kanal: <code>osobno + obitelj</code> (Računi, katalog, cijene, slike i sve liste)</div>
        <div class="list-container">
          <div class="list-row">
            <div>
              <div class="item-title">💻 Ovaj uređaj (${escapeHtml(st?.node_id ?? "lokalni čvor")})</div>
              <div class="item-sub" style="color:var(--success);">✓ Osnivač mreže • Puni pristup svim kanalima</div>
            </div>
          </div>
          ${ownDevices
            .map(
              (m) => `
            <div class="list-row">
              <div>
                <div class="item-title">💻 ${escapeHtml(m.name)}</div>
                <div class="item-sub">SAS: ${escapeHtml(m.sas_code)} • Kanal: ${escapeHtml(m.channels)} (SVE)</div>
              </div>
              <button class="btn btn-danger-ghost" data-remove-member="${escapeHtml(m.peer_id)}">✕</button>
            </div>`,
            )
            .join("")}
        </div>
      </div>

      <div class="card" style="margin-bottom:0;">
        <div class="item-title">2. Članovi obitelji — Dijeli SAMO liste</div>
        <div class="item-sub" style="margin:4px 0 12px;">Kanal: <code>obitelj</code> (Samo odabrane liste za kupnju; osobni računi ostaju privatni)</div>
        <div class="list-container">
          ${
            familyMembers.length === 0
              ? `<div class="list-row"><div class="item-sub">Još nema dodanih članova obitelji. Dodajte člana iznad pomoću SAS koda.</div></div>`
              : familyMembers
                  .map(
                    (m) => `
              <div class="list-row">
                <div>
                  <div class="item-title">👨‍👩‍👧 ${escapeHtml(m.name)}</div>
                  <div class="item-sub">SAS: ${escapeHtml(m.sas_code)} • Liste: ${escapeHtml(m.shared_lists)}</div>
                </div>
                <button class="btn btn-danger-ghost" data-remove-member="${escapeHtml(m.peer_id)}">✕</button>
              </div>`,
                  )
                  .join("")
          }
        </div>
      </div>
    </div>

    <div class="card">
      <div class="row space-between" style="margin-bottom:8px;">
        <div>
          <span class="pill">GOCOP ENGINE • RECORD_VERSIONS + SQLITE VACUUM</span>
          <div class="item-title" style="margin-top:6px;">Knjiga verzija i automatsko čišćenje (Auto-Compaction & VACUUM)</div>
        </div>
        <button class="btn btn-subtle" id="btn-compact-ledger">🧹 Očisti stare verzije i izvrši VACUUM</button>
      </div>
      <div class="item-sub" style="margin-bottom:12px;">
        Čvor uređaja: <strong>${escapeHtml(st?.node_id ?? "—")}</strong> • Aktivnih zapisa: <strong>${escapeHtml(st?.unique_records ?? 0)}</strong> • Ukupno verzija u knjizi: <strong>${escapeHtml(st?.total_versions ?? 0)}</strong> • Arhivirano (Tombstones): <strong>${escapeHtml(st?.tombstones ?? 0)}</strong> • Poznati čvorovi (Frontier): <strong>${escapeHtml(st?.frontier_nodes ?? 1)}</strong> • Veličina SQLite baze: <strong>${escapeHtml(sizeKb)} KB</strong>
      </div>
      <div class="item-sub">
          <strong>Automatsko održavanje:</strong> Pri pokretanju i zatim svakog sata provjerava se treba li održavanje. Najviše jednom dnevno uklanjaju se suvišne međuv­erzije starije od 30 dana ili iznad zadnje 3 verzije. Posljednje verzije, oznake arhiviranja, dokazi ovlasti i zapisi koji čekaju primjenu ostaju sačuvani; zaštićeni zapisi mogu zadržati više verzija. Nakon sažimanja VACUUM oslobađa neiskorišten prostor. Arhivirani podaci ne brišu se trajno samo zbog starosti.
      </div>
    </div>

    <div class="card">
      <div class="item-title" style="margin-bottom:6px;">Razmjena paketa (.list2cart) s provjerom uloge i SAS koda</div>
      <div class="item-sub" style="margin-bottom:12px;">
        Podaci se čuvaju lokalno na vašem uređaju. U izdanju 0.0.1-alfa sinkronizacija se obavlja izvozom i uvozom potpisanih <code>.list2cart</code> paketa (uz provjeru SAS koda i filtriranje odabranih obiteljskih popisa <code>shared_lists</code>):
      </div>
      <div class="row-wrap">
        <button class="btn btn-primary" id="btn-export-bundle">📤 Izvezi SVE (za moj drugi uređaj) →</button>
        <button class="btn btn-outline" id="btn-export-family-bundle">👨‍👩‍👧 Izvezi SAMO odabrane liste (za obitelj) →</button>
        <label class="btn btn-outline" style="cursor:pointer;">
          📥 Uvezi i spoji paket
          <input id="inp-import-bundle" type="file" accept=".json,.list2cart" style="display:none;" />
        </label>
      </div>
    </div>
  `;
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
      const list=model.lists[0];
      if(!list || (list.store_id!==null && list.store_id!==model.selectedStoreId)) {
        showToast("Najprije kreiraj popis za odabranu trgovinu u Planeru.");return;
      }
      model.draftPlanName=product.name;model.draftPlanPrice=(product.price_cents/100).toFixed(2);model.draftPlanQty="1";model.draftPlanUnit="kom";
      retail.results=null;model.tab="planer";render();
      document.getElementById("inp-plan-name")?.focus();
      showToast(`Pripremljeno za popis ${list.name}. Provjeri jedinicu i količinu pa dodaj.`);return;
    }
    model.draftName=product.name;model.draftBarcode=product.barcode??"";model.draftPrice=(product.price_cents/100).toFixed(2);model.draftQty="1";model.draftUnit="kom";model.draftPhotoDataUrl=null;
    retail.results=null;model.tab="kosarica";render();document.getElementById("inp-name")?.focus();showToast("Unos je popunjen. Provjeri jedinicu, količinu i cijenu prije dodavanja.");
  };});
}

function renderSyncProgress(): string {
  const p = syncTracker.progress;
  if (!p) return model.tab === "sync" ? `<section class="card"><h2>Status razmjene</h2><p>Nema aktivnog prijenosa. Odaberi uvoz ili izvoz paketa.</p></section>` : "";
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
    case "katalog":
      content = renderKatalog();
      break;
    case "trgovine":
      content = renderTrgovine();
      break;
    case "sync":
      content = renderSync();
      break;
  }

  root.innerHTML = `
    <div class="app-shell">
      ${renderSidebar()}
      <main class="main-content">
          ${["kosarica","planer","katalog"].includes(model.tab) ? renderRetailSearch() : ""}
          <div role="status" aria-live="polite" aria-atomic="true">${renderSyncProgress()}</div>
          ${model.incompleteReceipts ? `<div class="card" role="status">Računi koji čekaju dovršetak prijenosa: ${model.incompleteReceipts}. Prikazat će se kada stignu sve potvrđene stavke i zbrojevi budu usklađeni.</div>` : ""}
          ${model.conflicts.length || model.conflictReview || model.tab === "sync" ? renderIdentityConflicts() : ""}
        ${content}
      </main>
    </div>
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
      model.tab = btn.dataset.tab as Tab;
      render();
    };
  });

  document.querySelectorAll<HTMLButtonElement>("[data-goto]").forEach((btn) => {
    btn.onclick = () => {
      model.tab = btn.dataset.goto as Tab;
      render();
    };
  });

  document.querySelectorAll<HTMLButtonElement>("[data-store-plan]").forEach((btn) => {
    btn.onclick = () => {
      model.selectedStoreId = Number(btn.dataset.storePlan);
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
        showToast(ver.is_matched ? "Račun spremljen (100% točan)!" : "Račun spremljen s razlikom.");
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
      const storeId = Number((document.getElementById("sel-list-store") as HTMLSelectElement).value);
      if (!name) return showToast("Unesite naziv popisa.");
      await createShoppingList(name, storeId);
      model.draftNewList = "";
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
  if (btnAddPlanned && model.lists[0]) {
    btnAddPlanned.onclick = async () => {
      const name = (inpPlanName?.value ?? model.draftPlanName).trim();
      const qty = Number.parseFloat(inpPlanQty?.value ?? model.draftPlanQty) || 1;
      const unit = ((inpPlanUnit?.value as Unit) ?? model.draftPlanUnit) || "kom";
      const priceCents = parseEuroToCents(inpPlanPrice?.value ?? model.draftPlanPrice);
      if (!name) return;
      await addPlannedItem({
        listId: model.lists[0].id,
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
  if (btnTransferCart && model.lists[0]) {
    btnTransferCart.onclick = () => {
      const list = model.lists[0];
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
      model.tab = "kosarica";
      render();
      showToast(`Prebačene stavke s popisa "${list.name}" u košaricu!`);
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
  const inpMemberLists = document.getElementById("inp-member-lists") as HTMLInputElement | null;

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
    };
  }
  if (inpMemberLists) {
    inpMemberLists.oninput = () => {
      model.draftMemberLists = inpMemberLists.value;
    };
  }

  const btnExportBundle = document.getElementById("btn-export-bundle");
  if (btnExportBundle) {
    btnExportBundle.onclick = async () => {
      await trackPackageExport("own_device");
    };
  }

  const btnExportFamilyBundle = document.getElementById("btn-export-family-bundle");
  if (btnExportFamilyBundle) {
    btnExportFamilyBundle.onclick = async () => {
        const sharedFilter = (inpMemberLists?.value ?? model.draftMemberLists ?? "").trim() || null;
        await trackPackageExport("family_member",sharedFilter);
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
      const name = (inpMemberName?.value ?? model.draftMemberName).trim();
      const role = ((selMemberRole?.value as PeerRole) ?? model.pairingRole) || "family_member";
      const sharedLists = (inpMemberLists?.value ?? model.draftMemberLists ?? "").trim();
      if (!name) return showToast("Unesite naziv uređaja ili člana obitelji.");
      try {
        await addNetworkMember({
          name,
          role,
          sasCode: model.pendingSasCode,
          sharedLists,
        });
        model.draftMemberName = "";
        model.members = await listNetworkMembers();
        model.pendingSasCode = await newSasCode();
        render();
        showToast(
          role === "own_device"
            ? `Uređaj "${name}" dodan (dijeli SVE)!`
            : `Član obitelji "${name}" dodan (dijeli odabrane liste)!`,
        );
      } catch (e) {
        showToast(`Greška dodavanja u mrežu: ${e}`);
      }
    };
  }

  document.querySelectorAll<HTMLButtonElement>("[data-remove-member]").forEach((btn) => {
    btn.onclick = async () => {
      const peerId = btn.dataset.removeMember;
      if (!peerId) return;
      await removeNetworkMember(peerId);
      model.members = await listNetworkMembers();
      model.conflicts = await identityConflicts();
      render();
      showToast("Član/uređaj uklonjen iz mreže.");
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
    model.pendingSasCode = await newSasCode();
    model.incompleteReceipts = await incompleteReceiptCount();
    model.conflicts = await identityConflicts();
    const pbUrl = await getPocketbaseUrl();
    if (pbUrl) model.pocketbaseUrl = pbUrl;
  } catch (e) {
    console.error("Init error:", e);
  }
  render();
}

void init();
