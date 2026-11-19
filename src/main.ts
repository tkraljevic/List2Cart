import {
  Format,
  cancel as cancelNativeBarcodeScan,
  checkPermissions as checkBarcodePermissions,
  requestPermissions as requestBarcodePermissions,
  scan as scanNativeBarcode,
} from "@tauri-apps/plugin-barcode-scanner";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  addNetworkMember,
  retailSources,
  retailSearch,
  retailAllSources,
  retailHomeBranches,
  setRetailHomeBranches,
  removeRetailHomeBranch,
  retailSearchHome,
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
  setListShared,
  deleteShoppingList,
  dialPairPeer,
  discoverLanPeers,
  exportReceiptXlsx,
  exportListXlsx,
  exportReceiptsXlsx,
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
  removePlannedItem,
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
import type { PriceSource, RetailResults, RetailProduct, CsvInspection, CsvMapping, HomeBranch } from "./api";
import { SyncTracker } from "./sync-progress";
import { compareBasket, formatDate, formatQuantity, resolveBasketProduct, sharedListNames } from "./shopping";
import type { BasketComparison } from "./shopping";
import { icon } from "./icons";
import { renderNavigation, renderProductTabs } from "./ui/shell";
import { editCartItem, renderCartEditor } from "./ui/commerce/cart-editor";
import { renderListCards } from "./ui/commerce/lists";
import { createListWithRefresh, listCreationError } from "./ui/commerce/list-creation";
import { CHAINS, chainForStoreName, chainName, formatOfferToast, matchLocalProducts, rankOffers, renderBranchSetup, renderQuickFind, suggestPicks } from "./ui/commerce/quick-find";
import type { BranchSetupState, QuickFindState } from "./ui/commerce/quick-find";
import { renderFamily, renderMemberDialog as renderMemberView } from "./ui/family";
import { renderAbout } from "./ui/about";
import { renderPairingDialog as renderPairingView } from "./ui/pairing";
import { renderSyncProgress as renderProgressView, renderSyncOverview, renderIdentityConflicts as renderConflictView } from "./ui/sync";
import { appMetadata } from "./ui/app-metadata";
import type { FamilyViewState, ViewHelpers } from "./ui/contracts";
import "./ui/family-sync.css";

const syncTracker = new SyncTracker(() => render());
const retail = {sources:[] as PriceSource[],results:null as RetailResults|null,query:"",branch:"",busy:false,error:"",storeId:0};
const localPriceCsv = {bytes:[] as number[],inspection:null as CsvInspection|null,filename:"",products:null as RetailProduct[]|null,storeId:0};
/** Suggestions under the list's add field: own products and today's price lists. */
const quickFind: QuickFindState = { query: "", busy: false, error: "", results: null, searchedQuery: "" };
let quickFindSeq = 0;
let homeBranches: HomeBranch[] = [];
const branchSetup: BranchSetupState & { open: boolean } = { open: false, place: "", all: null, busy: false, error: "", picked: {} };
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

type Tab = "pregled" | "kosarica" | "planer" | "racuni" | "katalog" | "cijene" | "trgovine" | "sync" | "napredno" | "about";

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
  pairingAdmit: boolean;
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
  newListBusy: boolean;
  newListError: string;
  draftNewListStoreId: number | null;
  /** The new list is shared with every member of the network. */
  draftNewListShared: boolean;
  /** This device can share lists: it holds the network key or a delegation. */
  canShareLists: boolean;
  /** Sharing or unsharing the open list is in progress. */
  shareBusy: boolean;
  /** The list whose deletion waits for a second click. */
  confirmDeleteListId: number | null;
  selectedReceipt: ReceiptSummary | null;
  receiptSearch: string;
  receiptStore: string;
  pairingOpen: boolean;
  pairingConfirmed: boolean;
  pairingBusy: boolean;
  pairingAction: "confirming" | "cancelling" | null;
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
  tab: "planer",
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
  pairingAdmit: true,
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
  newListBusy: false,
  newListError: "",
  draftNewListStoreId: null,
  draftNewListShared: false,
  canShareLists: false,
  shareBusy: false,
  confirmDeleteListId: null,
  selectedReceipt: null,
  receiptSearch: "",
  receiptStore: "",
  pairingOpen: false,
  pairingConfirmed: false,
  pairingBusy: false,
  pairingAction: null,
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

let pairingOperation = 0;
let syncDetailsOpen = false;
let editingCartItemId: string | null = null;

let scannerDecodeTimer: number | null = null;
let pairPollTimer: number | null = null;
let nativeScannerActive = false;

/** Cancel native work before dismissing the dialog; ignore stale IPC completions. */
async function cancelPairing(): Promise<void> {
  if (model.pairingAction === "cancelling") return;
  if (model.pairingBusy && model.pairingAction !== "confirming") return;
  const wasConfirming = model.pairingAction === "confirming";
  ++pairingOperation;
  if (pairPollTimer !== null) {
    window.clearInterval(pairPollTimer);
    pairPollTimer = null;
  }
  model.pairingBusy = true;
  model.pairingAction = "cancelling";
  render();
  try {
    if (model.pairMode === "lan" && (wasConfirming || model.pairStatus?.waiting || model.pairStatus?.pending)) {
      model.pairStatus = await stopPairListening();
    }
    model.pairingConfirmed = false;
    model.pairingOpen = false;
    // Cancellation cannot retract a confirmation already sent to the peer.
    // Refresh actual membership instead of assuming that no pairing took place.
    try {
      model.members = await listNetworkMembers();
      model.ledgerStats = await getLedgerStats();
    } catch (error) {
      model.toast = `Čekanje je prekinuto, ali pregled uređaja nije osvježen: ${error}`;
    }
    model.pairingAction = null;
    model.pairingBusy = false;
  } catch (error) {
    model.pairingAction = wasConfirming ? "confirming" : null;
    model.pairingBusy = wasConfirming;
    model.toast = `Prekid povezivanja nije potvrđen. Pokušajte ponovno: ${error}`;
  }
  render();
}

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

function renderSidebar(): string {
  return renderNavigation(model.tab);
}

function pageHeader(title: string, description: string, action = ""): string {
  const secondary = ["pregled", "napredno"].includes(model.tab);
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
  return `${pageHeader("Kupnja", "Dodajte artikle i na blagajni usporedite unesene iznose.", storeSelect)}
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
            <button class="text-button cart-edit-trigger" data-edit-cart="${escapeHtml(item.id)}" aria-label="Uredi količinu i cijenu za ${escapeHtml(item.name)}">${escapeHtml(formatCents(item.total_cents))}<span>Uredi</span></button>
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

/** The latest pairing status knows best; before it loads, the value read at start. */
function canShareLists(): boolean {
  return model.pairStatus?.can_admit ?? model.canShareLists;
}

function quickFindHtml(): string {
  const list = currentList();
  return renderQuickFind(quickFind, {
    local: matchLocalProducts(model.products, quickFind.query),
    home: homeBranches,
    listChain: list?.store_name ? retailerChain(list.store_name) : null,
    escape: escapeHtml,
  });
}

/** Redraws only the suggestions, so typing in the field keeps its focus and caret. */
function refreshQuickFind(): void {
  const slot = document.getElementById("quick-find-slot");
  if (!slot) return;
  slot.innerHTML = quickFindHtml();
  bindQuickFind();
}

function resetQuickFind(): void {
  quickFindSeq++;
  Object.assign(quickFind, { query: "", busy: false, error: "", results: null, searchedQuery: "" });
}

async function runQuickSearch(chains: string[] | null): Promise<void> {
  const query = quickFind.query.trim();
  if (query.length < 2 || quickFind.busy) return;
  const seq = ++quickFindSeq;
  Object.assign(quickFind, { busy: true, error: "", results: null });
  refreshQuickFind();
  try {
    const results = await retailSearchHome(query, chains);
    if (seq !== quickFindSeq) return;
    Object.assign(quickFind, { results, searchedQuery: quickFind.query });
  } catch (error) {
    if (seq !== quickFindSeq) return;
    quickFind.error = `Pretraga nije uspjela: ${error}`;
  } finally {
    if (seq === quickFindSeq) {
      quickFind.busy = false;
      refreshQuickFind();
    }
  }
}

async function addSuggestedItem(item: { barcode: string | null; name: string; unit: Unit; priceCents: number }, toast: string): Promise<void> {
  const list = currentList();
  if (!list) return;
  try {
    await addPlannedItem({ listId: list.id, barcode: item.barcode, name: item.name, quantity: 1, unit: item.unit, estimatedPriceCents: item.priceCents });
  } catch (error) {
    showToast(`Artikl nije dodan: ${error}`);
    return;
  }
  resetQuickFind();
  model.draftPlanName = "";
  model.lists = await listShoppingLists();
  render();
  document.getElementById("inp-plan-name")?.focus();
  showToast(toast);
}

function bindQuickFind(): void {
  document.querySelectorAll<HTMLButtonElement>("[data-qf-search]").forEach(button => {
    button.onclick = () => void runQuickSearch(button.dataset.qfSearch === "all" ? null : [button.dataset.qfSearch!]);
  });
  document.querySelectorAll<HTMLButtonElement>("[data-qf-setup]").forEach(button => {
    button.onclick = () => { branchSetup.open = true; render(); document.getElementById("branch-place")?.focus(); };
  });
  document.querySelectorAll<HTMLButtonElement>("[data-qf-local]").forEach(button => {
    button.onclick = () => {
      const product = model.products.find(p => p.barcode === button.dataset.qfLocal);
      if (!product) return;
      const where = product.last_store_name ? ` · ${product.last_store_name} ${formatCents(product.last_price_cents)}` : "";
      void addSuggestedItem({ barcode: product.barcode, name: product.name, unit: product.default_unit, priceCents: product.last_price_cents }, `Dodano: ${product.name}${where}`);
    };
  });
  document.querySelectorAll<HTMLButtonElement>("[data-qf-offer]").forEach(button => {
    button.onclick = () => {
      const offer = quickFind.results ? rankOffers(quickFind.results)[Number(button.dataset.qfOffer)] : undefined;
      if (!offer) return;
      void addSuggestedItem({ barcode: offer.product.barcode, name: offer.product.name, unit: "kom", priceCents: offer.priceCents }, formatOfferToast(offer));
    };
  });
}

/** Redraws the page and keeps the branch picker in view; a full render starts at the top. */
function renderSetup(): void {
  render();
  document.getElementById("branch-setup")?.scrollIntoView({ block: "nearest" });
}

function bindBranchSetup(): void {
  const close = document.querySelector<HTMLButtonElement>("[data-branch-close]");
  if (close) close.onclick = () => { branchSetup.open = false; render(); };
  const form = document.getElementById("branch-place-form") as HTMLFormElement | null;
  const place = document.getElementById("branch-place") as HTMLInputElement | null;
  if (place) place.oninput = () => { branchSetup.place = place.value; };
  if (form) form.onsubmit = async event => {
    event.preventDefault();
    if (branchSetup.busy) return;
    branchSetup.place = place?.value ?? branchSetup.place;
    if (!branchSetup.place.trim()) { branchSetup.error = "Upiši grad ili poštanski broj."; renderSetup(); return; }
    branchSetup.error = "";
    if (!branchSetup.all || branchSetup.all.some(c => c.error)) {
      branchSetup.busy = true;
      renderSetup();
      try { branchSetup.all = await retailAllSources(); }
      catch (error) { branchSetup.error = `Poslovnice nisu učitane: ${error}`; }
      finally { branchSetup.busy = false; }
    }
    if (branchSetup.all) {
      branchSetup.picked = suggestPicks(branchSetup.all, branchSetup.place, homeBranches);
      if (!Object.keys(branchSetup.picked).length) branchSetup.error = "Za to mjesto nije nađena nijedna poslovnica. Probaj drugi grad ili poštanski broj.";
    }
    renderSetup();
  };
  document.querySelectorAll<HTMLSelectElement>("[data-branch-pick]").forEach(select => {
    select.onchange = () => { branchSetup.picked[select.dataset.branchPick!] = select.value; renderSetup(); };
  });
  const save = document.querySelector<HTMLButtonElement>("[data-branch-save]");
  if (save) save.onclick = async () => {
    const urls = Object.values(branchSetup.picked).filter(Boolean);
    if (!urls.length || branchSetup.busy) return;
    try {
      homeBranches = await setRetailHomeBranches(urls);
      Object.assign(branchSetup, { open: false, error: "", picked: {} });
      render();
      showToast(`Spremljeno: ${homeBranches.map(h => chainName(h.chain)).join(", ")}.`);
      document.getElementById("inp-plan-name")?.focus();
    } catch (error) {
      branchSetup.error = `Poslovnice nisu spremljene: ${error}`;
      renderSetup();
    }
  };
  document.querySelectorAll<HTMLButtonElement>("[data-branch-remove]").forEach(button => {
    button.onclick = async () => {
      try { homeBranches = await removeRetailHomeBranch(button.dataset.branchRemove!); }
      catch (error) { showToast(`Poslovnica nije uklonjena: ${error}`); return; }
      renderSetup();
    };
  });
}

function renderPlaner(): string {
  const list = currentList();
  const items = list?.items ?? [];
  const checked = items.filter(i => i.checked).length;
  const remaining = items.filter(i => !i.checked);
  const lineTotal = (item: typeof items[number]) => calcLineCents(item.quantity, item.unit, item.estimated_price_cents);
  const total = items.reduce((sum, i) => sum + lineTotal(i), 0);
  const remainingTotal = remaining.reduce((sum, i) => sum + lineTotal(i), 0);
  return `${pageHeader("Popisi", "Pripremite kupnju i označite kupljene artikle.", `<div class="header-actions">${model.lists.length ? `<select id="active-list-select" aria-label="Aktivni popis">${model.lists.map(l => `<option value="${l.id}" ${l.id === list?.id ? "selected" : ""}>${escapeHtml(l.name)}</option>`).join("")}</select>` : ""}<button class="btn btn-outline" id="btn-new-list" aria-expanded="${model.newListOpen}" ${model.newListBusy ? "disabled" : ""}>${icon("plus")} Novi popis</button></div>`)}
    ${renderListCards(model.lists, list?.id, escapeHtml)}
    ${model.newListOpen ? `<section class="card no-print" aria-label="Novi popis" aria-busy="${model.newListBusy}"><h2>Novi popis za kupnju</h2><form id="new-list-form" class="row-wrap">
      <label class="field grow">Naziv popisa<input id="inp-new-list" value="${escapeHtml(model.draftNewList)}" placeholder="Npr. Tjedna nabavka" required ${model.newListBusy ? "disabled" : ""} /></label>
      <label class="field">Trgovina<select id="sel-list-store" ${model.newListBusy ? "disabled" : ""}><option value="" ${model.draftNewListStoreId === null ? "selected" : ""}>Sve trgovine</option>${model.stores.map(s => `<option value="${s.id}" ${s.id === model.draftNewListStoreId ? "selected" : ""}>${escapeHtml(s.name)}</option>`).join("")}</select></label>
      ${canShareLists() ? `<label class="field shared-toggle"><span><input type="checkbox" id="chk-list-shared" ${model.draftNewListShared ? "checked" : ""} ${model.newListBusy ? "disabled" : ""} /> Zajednički popis</span><span class="item-sub">Vide ga i uređuju svi članovi obitelji i tvoji uređaji.</span></label>` : ""}
      <button class="btn btn-primary" id="btn-create-list" type="submit" ${model.newListBusy ? "disabled" : ""}>${model.newListBusy ? "Spremanje…" : "Kreiraj popis"}</button></form>
      ${model.newListError ? `<p class="error-inline" role="alert">${escapeHtml(model.newListError)}</p>` : ""}</section>` : ""}
    ${list ? `<section class="card plan-add no-print"><div class="row"><input id="inp-plan-name" aria-label="Artikl za popis" value="${escapeHtml(model.draftPlanName)}" placeholder="Dodaj artikl na popis… (npr. linolada)" autocomplete="off" /><button class="btn btn-primary" id="btn-add-planned">Dodaj</button></div>
      <div id="quick-find-slot" aria-live="polite">${quickFindHtml()}</div>
      <details class="plan-details" ${model.draftPlanPrice || model.draftPlanQty !== "1" || model.draftPlanUnit !== "kom" ? "open" : ""}><summary>Količina i procjena cijene</summary><div class="row-wrap">
        <label class="field grow">Količina<input id="inp-plan-qty" type="number" min="0.001" step="0.001" value="${escapeHtml(model.draftPlanQty)}" /></label>
        <label class="field grow">Jedinica<select id="inp-plan-unit">${unitOptions(model.draftPlanUnit)}</select></label>
        <label class="field grow">Cijena (€)<input id="inp-plan-price" inputmode="decimal" value="${escapeHtml(model.draftPlanPrice)}" placeholder="0,00" /></label>
      </div></details></section>
      ${branchSetup.open ? renderBranchSetup(branchSetup, homeBranches, escapeHtml) : ""}
      <section class="card plan-progress"><div class="row space-between"><span class="item-sub">${checked} od ${items.length} kupljeno</span><strong class="item-price">${escapeHtml(formatCents(total))}</strong></div><div class="progress-track" role="progressbar" aria-label="Kupljeni artikli" aria-valuemin="0" aria-valuemax="${Math.max(items.length, 1)}" aria-valuenow="${checked}"><div class="progress-fill" style="width:${items.length ? checked / items.length * 100 : 0}%"></div></div></section>
      <div class="plan-layout"><section class="card plan-items" aria-label="Stavke popisa ${escapeHtml(list.name)}">
        ${items.length ? items.map(item => `<div class="plan-row ${item.checked ? "is-checked" : ""}"><label class="row grow"><input type="checkbox" data-toggle-plan="${item.id}" ${item.checked ? "checked" : ""} /><span class="item-copy"><span class="item-title">${escapeHtml(item.name)}</span><span class="item-sub">${formatQuantity(item.quantity)} ${item.unit === "l" ? "L" : item.unit}</span></span></label><span class="item-price">${escapeHtml(formatCents(lineTotal(item)))}</span><button class="icon-button btn-danger-ghost no-print" type="button" data-remove-plan="${escapeHtml(item.sync_id)}" aria-label="Ukloni ${escapeHtml(item.name)}">${icon("close")}</button></div>`).join("") : '<div class="empty-state"><h2>Što trebate kupiti?</h2><p>Dodajte prvi artikl na ovaj popis.</p></div>'}
      </section><section class="card plan-summary"><div class="row space-between"><span class="item-sub">Preostalo · ${remaining.length} artikala</span><strong class="checkout-total">${escapeHtml(formatCents(remainingTotal))}</strong></div>
        <button class="btn btn-primary" id="btn-transfer-cart" ${remaining.length ? "" : "disabled"}>Kreni u kupnju</button><p class="item-sub">Preostali artikli dodaju se u košaricu s unesenim procjenama cijena.</p><button class="text-button" data-goto="cijene">Usporedi cijene popisa →</button><button class="text-button no-print" data-toggle-retail>Pronađi artikl u cjeniku</button>
        <button class="text-button no-print" id="btn-export-list" ${items.length ? "" : "disabled"}>Izvezi u Excel</button>
        ${canShareLists() ? `<button class="text-button no-print" id="btn-toggle-shared" ${model.shareBusy ? "disabled" : ""}>${list.shared ? "Prestani dijeliti" : "Podijeli s obitelji"}</button><p class="item-sub">${list.shared ? "Popis vide i uređuju svi članovi obitelji. Ako prestaneš dijeliti, vraća se među tvoje privatne popise sa svim artiklima." : "Popis će vidjeti i uređivati svi članovi obitelji."}</p>` : ""}
        ${!list.shared || canShareLists() ? (model.confirmDeleteListId === list.id
          ? `<div class="delete-confirm" id="delete-list-confirm" role="alert"><p>Sigurno obrisati „${escapeHtml(list.name)}”?${list.shared ? " Nestat će i svim članovima obitelji." : ""}</p><div class="row-wrap"><button class="btn btn-danger" id="btn-delete-list-confirm">Obriši popis</button><button class="btn btn-outline" id="btn-delete-list-cancel">Odustani</button></div></div>`
          : `<button class="text-button btn-danger-ghost no-print" id="btn-delete-list">Obriši popis</button>`) : ""}
      </section></div>` : `<section class="card empty-state">${icon("planer")}<h2>Vaš prvi popis za kupnju</h2><p>Odaberite Novi popis i zapišite što trebate.</p></section>`}`;
}

/** The receipts the Računi view shows for its search and store filter. */
function visibleReceipts(): ReceiptSummary[] {
  const query = model.receiptSearch.trim().toLocaleLowerCase("hr");
  return model.receipts.filter(r => (!model.receiptStore || r.store_name === model.receiptStore)
    && `${r.store_name} ${r.items.map(i => i.name).join(" ")} ${formatDate(r.created_at)}`.toLocaleLowerCase("hr").includes(query));
}

/** What the export holds, in words, for the top of the spreadsheet. */
function receiptSelectionText(): string {
  const parts = [model.receiptStore || "Sve trgovine"];
  if (model.receiptSearch.trim()) parts.push(`pretraga „${model.receiptSearch.trim()}”`);
  return parts.join(", ");
}

function renderReceipts(): string {
  const query = model.receiptSearch.trim().toLocaleLowerCase("hr");
  const receipts = visibleReceipts();
  const total = receipts.reduce((sum, r) => sum + r.receipt_total_cents, 0);
  return `${pageHeader("Računi", "Povijest kupnje i spremljene stavke na jednom mjestu.", `<button class="btn btn-outline no-print" id="btn-export-receipts" ${receipts.length ? "" : "disabled"}>Izvezi u Excel</button>`)}
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
  return model.lanStatus?.peers.find(peer => peer.peer_public_key === peerId);
}

const viewHelpers: ViewHelpers = { escapeHtml, formatDate, sharedListNames };

function familyState(): FamilyViewState {
  return { ...model, progress: syncTracker.progress };
}

function renderSync(): string {
  if (syncDetailsOpen) return renderSyncOverview(familyState(), viewHelpers);
  return renderFamily(familyState(), viewHelpers)
    + '<div class="family-footer"><button class="btn btn-outline" data-show-sync-status>Tijek sinkronizacije</button><button class="text-button" data-goto="pregled">Pregled potrošnje</button></div>';
}

function renderPairingDialog(): string {
  return renderPairingView(model, viewHelpers);
}

function renderMemberDialog(): string {
  return renderMemberView(familyState(), viewHelpers);
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
  return renderConflictView(model, viewHelpers, conflictLabel);
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
  return chainForStoreName(name);
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
    if(!chain){retail.error=`Podržani su ${CHAINS.map(chainName).join(", ")}. Odaberi jednu od tih trgovina.`;render();return;}
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
  return renderProgressView(syncTracker.progress, viewHelpers);
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
    case "about":
      content = renderAbout(appMetadata, viewHelpers);
      break;
    case "napredno":
      content = renderAdvanced();
      break;
  }

  root.innerHTML = `
    <div class="app-shell">
      ${renderSidebar()}
      <main class="main-content">
          ${renderProductTabs(model.tab)}
          ${model.retailOpen && ["kosarica", "planer", "katalog", "cijene"].includes(model.tab) ? renderRetailSearch() : ""}
          <div role="status" aria-live="polite" aria-atomic="true">${model.tab === "sync" && syncDetailsOpen ? "" : renderSyncProgress()}</div>
          ${model.incompleteReceipts ? `<div class="card" role="status">Računi koji čekaju dovršetak prijenosa: ${model.incompleteReceipts}. Prikazat će se kada stignu sve potvrđene stavke i zbrojevi budu usklađeni.</div>` : ""}
          ${(model.conflicts.length || model.conflictReview) && model.tab !== "napredno" ? renderIdentityConflicts() : ""}
        ${content}
      </main>
    </div>
    ${editingCartItemId && model.cart.find(item => item.id === editingCartItemId) ? renderCartEditor(model.cart.find(item => item.id === editingCartItemId)!, escapeHtml) : ""}
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
  document.querySelectorAll<HTMLButtonElement>("[data-show-sync-status]").forEach(button => {
    button.onclick = () => { syncDetailsOpen = true; model.tab = "sync"; render(); };
  });
  document.querySelectorAll<HTMLButtonElement>("[data-open-list]").forEach(button => {
    button.onclick = () => {
      const id = Number(button.dataset.openList);
      if (!model.lists.some(list => list.id === id)) return;
      model.selectedListId = id;
      render();
    };
  });
  document.querySelectorAll<HTMLButtonElement>("[data-edit-cart]").forEach(button => {
    button.onclick = () => {
      editingCartItemId = button.dataset.editCart ?? null;
      render();
      document.getElementById("cart-edit-quantity")?.focus();
    };
  });
  document.querySelectorAll<HTMLButtonElement>("[data-cancel-cart-edit]").forEach(button => {
    button.onclick = () => { editingCartItemId = null; render(); };
  });
  const editor = document.getElementById("cart-edit-form") as HTMLFormElement | null;
  if (editor) {
    const readEdit = () => {
      const item = model.cart.find(item => item.id === editingCartItemId);
      return item ? editCartItem(item, {
        quantity: (document.getElementById("cart-edit-quantity") as HTMLInputElement).value,
        unit: (document.getElementById("cart-edit-unit") as HTMLSelectElement).value as Unit,
        price: (document.getElementById("cart-edit-price") as HTMLInputElement).value,
      }) : null;
    };
    editor.oninput = () => {
      const result = readEdit();
      const total = document.getElementById("cart-edit-total");
      if (total) total.textContent = result?.ok ? formatCents(result.item.total_cents) : "—";
    };
    editor.onsubmit = event => {
      event.preventDefault();
      const result = readEdit();
      if (!result) return;
      if (!result.ok) {
        const error = document.getElementById("cart-edit-error");
        if (error) error.textContent = result.error;
        return;
      }
      model.cart = model.cart.map(item => item.id === editingCartItemId ? result.item : item);
      editingCartItemId = null;
      render();
    };
  }
  document.querySelectorAll<HTMLButtonElement>("[data-fs-open-pairing]").forEach(button => {
    button.onclick = async () => {
      model.pairingAdmit = false;
      model.pairMode = "lan";
      model.pairingOpen = true;
      model.pairingConfirmed = false;
      model.pairStatus = await getPairStatus().catch(() => null);
      render();
    };
  });
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
      if (nextTab === "sync") syncDetailsOpen = false;
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
      if (nextTab === "sync") syncDetailsOpen = false;
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
    btn.onclick = async () => {
      if (model.pairingOpen) {
        await cancelPairing();
        return;
      }
      model.selectedReceipt = null;
      model.accessMember = null;
      model.removalMember = null;
      render();
    };
  });

  document.querySelectorAll<HTMLButtonElement>("[data-store-plan]").forEach((btn) => {
    btn.onclick = () => {
      model.selectedStoreId = Number(btn.dataset.storePlan);
      model.draftNewListStoreId = model.selectedStoreId;
      model.newListError = "";
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
      if (model.newListBusy) return;
      model.newListOpen = !model.newListOpen;
      model.newListError = "";
      render();
      if (model.newListOpen) document.getElementById("inp-new-list")?.focus();
    };
  }

  const inpNewList = document.getElementById("inp-new-list") as HTMLInputElement | null;
  if (inpNewList) {
    inpNewList.oninput = () => {
      model.draftNewList = inpNewList.value;
    };
  }

  const newListStore = document.getElementById("sel-list-store") as HTMLSelectElement | null;
  if (newListStore) newListStore.onchange = () => {
    model.draftNewListStoreId = newListStore.value ? Number(newListStore.value) : null;
  };
  const exportTarget = currentList();
  const btnExportList = document.getElementById("btn-export-list") as HTMLButtonElement | null;
  if (btnExportList && exportTarget) btnExportList.onclick = async () => {
    btnExportList.disabled = true;
    try {
      showToast(`Excel spremljen: ${await exportListXlsx(exportTarget.id)}`);
    } catch (error) {
      showToast(`Excel izvoz nije uspio: ${error}`);
    } finally {
      btnExportList.disabled = false;
    }
  };
  const deleteTarget = currentList();
  const btnDeleteList = document.getElementById("btn-delete-list");
  if (btnDeleteList && deleteTarget) btnDeleteList.onclick = () => {
    model.confirmDeleteListId = deleteTarget.id;
    render();
    // A full render starts at the top; keep the question where it was asked.
    document.getElementById("delete-list-confirm")?.scrollIntoView({ block: "nearest" });
  };
  const btnDeleteCancel = document.getElementById("btn-delete-list-cancel");
  if (btnDeleteCancel) btnDeleteCancel.onclick = () => {
    model.confirmDeleteListId = null;
    render();
  };
  const btnDeleteConfirm = document.getElementById("btn-delete-list-confirm") as HTMLButtonElement | null;
  if (btnDeleteConfirm && deleteTarget) btnDeleteConfirm.onclick = async () => {
    btnDeleteConfirm.disabled = true;
    try {
      await deleteShoppingList(deleteTarget.id);
      model.confirmDeleteListId = null;
      model.selectedListId = null;
      model.lists = await listShoppingLists();
      render();
      showToast(`Popis „${deleteTarget.name}” je obrisan.`);
    } catch (error) {
      model.confirmDeleteListId = null;
      render();
      showToast(`Popis nije obrisan: ${listCreationError(error).replace(/^Popis nije spremljen: /, "")}`);
    }
  };
  const btnToggleShared = document.getElementById("btn-toggle-shared");
  const sharedTarget = currentList();
  if (btnToggleShared && sharedTarget) {
    btnToggleShared.onclick = async () => {
      if (model.shareBusy) return;
      const share = !sharedTarget.shared;
      model.shareBusy = true;
      render();
      try {
        const result = await setListShared(sharedTarget.id, share);
        model.selectedListId = result.id;
        model.lists = await listShoppingLists();
        showToast(share
          ? `Popis „${result.name}” sad vide svi članovi obitelji.`
          : `Popis „${result.name}” više nije zajednički i vraćen je među tvoje popise.`);
      } catch (error) {
        showToast(`${share ? "Popis nije podijeljen" : "Dijeljenje nije prekinuto"}: ${listCreationError(error).replace(/^Popis nije spremljen: /, "")}`);
      } finally {
        model.shareBusy = false;
        render();
      }
    };
  }
  const newListShared = document.getElementById("chk-list-shared") as HTMLInputElement | null;
  if (newListShared) newListShared.onchange = () => {
    model.draftNewListShared = newListShared.checked;
  };
  const newListForm = document.getElementById("new-list-form") as HTMLFormElement | null;
  if (newListForm) {
    newListForm.onsubmit = async event => {
      event.preventDefault();
      if (model.newListBusy) return;
      const name = (inpNewList?.value ?? model.draftNewList).trim();
      const storeId = model.draftNewListStoreId;
      if (!name) return showToast("Unesite naziv popisa.");
      model.draftNewList = name;
      let refreshNote = "";
      model.newListBusy = true;
      model.newListError = "";
      render();
      try {
        const result = await createListWithRefresh(name, storeId, {
          create: createShoppingList, refresh: listShoppingLists,
        }, canShareLists() && model.draftNewListShared);
        model.selectedListId = result.created.id;
        model.lists = result.lists ?? [result.created, ...model.lists.filter(list => list.id !== result.created.id)];
        model.draftNewList = "";
        model.draftNewListShared = false;
        model.newListOpen = false;
        refreshNote = result.refreshError ? "Popis je spremljen. Ostale popise trenutačno nije moguće osvježiti." : "";
      } catch (error) {
        model.newListError = listCreationError(error);
      } finally {
        model.newListBusy = false;
        render();
      }
      // Through showToast, so the note also goes away again.
      if (refreshNote) showToast(refreshNote);
    };
  }

  const inpPlanName = document.getElementById("inp-plan-name") as HTMLInputElement | null;
  const inpPlanQty = document.getElementById("inp-plan-qty") as HTMLInputElement | null;
  const inpPlanUnit = document.getElementById("inp-plan-unit") as HTMLSelectElement | null;
  const inpPlanPrice = document.getElementById("inp-plan-price") as HTMLInputElement | null;
  if (inpPlanName) {
    inpPlanName.oninput = () => {
      model.draftPlanName = inpPlanName.value;
      if (quickFind.query !== inpPlanName.value) {
        quickFindSeq++;
        Object.assign(quickFind, { query: inpPlanName.value, busy: false, error: "" });
        refreshQuickFind();
      }
    };
    inpPlanName.onkeydown = event => {
      if (event.key === "Enter" && !event.isComposing) {
        event.preventDefault();
        document.getElementById("btn-add-planned")?.click();
      } else if (event.key === "Escape" && quickFind.query) {
        resetQuickFind();
        refreshQuickFind();
      }
    };
  }
  bindQuickFind();
  bindBranchSetup();
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
      resetQuickFind();
      model.lists = await listShoppingLists();
      render();
      document.getElementById("inp-plan-name")?.focus();
    };
  }

  document.querySelectorAll<HTMLInputElement>("[data-toggle-plan]").forEach((chk) => {
    chk.onchange = async () => {
      await togglePlannedItem(Number(chk.dataset.togglePlan), chk.checked);
      model.lists = await listShoppingLists();
      render();
    };
  });

  document.querySelectorAll<HTMLButtonElement>("[data-remove-plan]").forEach((btn) => {
    btn.onclick = async () => {
      const syncId = btn.dataset.removePlan;
      if (!syncId) return;
      await removePlannedItem(syncId);
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

  const btnExportReceipts = document.getElementById("btn-export-receipts") as HTMLButtonElement | null;
  if (btnExportReceipts) btnExportReceipts.onclick = async () => {
    const receipts = visibleReceipts();
    if (!receipts.length) return;
    btnExportReceipts.disabled = true;
    try {
      const path = await exportReceiptsXlsx(receipts.map(r => r.id), receiptSelectionText());
      showToast(`Excel s ${receipts.length} ${receipts.length === 1 ? "računom" : "računa"} spremljen: ${path}`);
    } catch (error) {
      showToast(`Excel izvoz nije uspio: ${error}`);
    } finally {
      btnExportReceipts.disabled = false;
    }
  };
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
  const selPairDirection = document.getElementById("sel-pair-direction") as HTMLSelectElement | null;
  if (selPairDirection) {
    selPairDirection.onchange = () => {
      model.pairingAdmit = selPairDirection.value === "admit";
      render();
    };
  }

  const btnOpenPairing = document.getElementById("btn-open-pairing");
  if (btnOpenPairing) {
    btnOpenPairing.onclick = async () => {
      model.pairingAdmit = true;
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
      const operation = pairingOperation;
      const next = await getPairStatus().catch(() => null);
      if (!next || operation !== pairingOperation || !model.pairingOpen || model.pairingAction === "cancelling") return;
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
      model.pairingAdmit = true;
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
    model.pairingAdmit = model.members.length > 0;
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
      const canAdmit = model.pairStatus?.can_admit ?? false;
      const admit = canAdmit && model.pairingAdmit;
      const role = ((selMemberRole?.value as PeerRole) ?? model.pairingRole) || "family_member";
      const sharedLists =
        !admit || role === "own_device"
          ? "*"
          : (model.draftMemberLists ?? "").split(",").filter(Boolean).join(",");
      if (admit && role === "family_member" && !sharedLists) {
        return showToast("Odaberite barem jedan popis za dijeljenje.");
      }
      const operation = ++pairingOperation;
      model.pairingBusy = true;
      model.pairingAction = "confirming";
      render();
      try {
        const outcome = await confirmPairPeer({
          approved: true,
          role,
          sharedLists,
          admit,
        });
        if (operation !== pairingOperation) return;
        stopPairPoll();
        const [status, members, lanStatus, ledgerStats] = await Promise.all([
          getPairStatus().catch(() => null),
          listNetworkMembers(),
          getLanStatus().catch(() => model.lanStatus),
          getLedgerStats().catch(() => model.ledgerStats),
        ]);
        if (operation !== pairingOperation) return;
        model.pairStatus = status;
        model.pairingOpen = false;
        model.pairingConfirmed = false;
        model.draftMemberLists = null;
        model.members = members;
        model.lanStatus = lanStatus;
        model.ledgerStats = ledgerStats;
        if (model.ledgerStats?.network_name) {
          model.draftNetworkName = model.ledgerStats.network_name;
        }
        showToast(outcome.message);
      } catch (e) {
        if (operation === pairingOperation) showToast(`Greška potvrde uparivanja: ${e}`);
      } finally {
        if (operation === pairingOperation) {
          model.pairingBusy = false;
          model.pairingAction = null;
          render();
        }
      }
    };
  }

  const btnRejectLivePairing = document.getElementById("btn-reject-live-pairing");
  if (btnRejectLivePairing) {
    btnRejectLivePairing.onclick = () => { void cancelPairing(); };
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

/**
 * A link to another website opens in the system browser; the app window has no
 * browser of its own, so such a link would otherwise do nothing.
 */
document.addEventListener("click", event => {
  const link = (event.target as Element | null)?.closest?.('a[href^="https://"]') as HTMLAnchorElement | null;
  if (!link || !("__TAURI_INTERNALS__" in window)) return;
  event.preventDefault();
  void openUrl(link.href).catch(error => showToast(`Poveznica se nije otvorila: ${error}`));
});

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
    homeBranches = await retailHomeBranches().catch(() => []);
    model.canShareLists = await getPairStatus().then(status => status?.can_admit ?? false).catch(() => false);
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
