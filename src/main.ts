import {
  addNetworkMember,
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

interface AppModel {
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
}

const model: AppModel = {
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
};

function showToast(msg: string) {
  model.toast = msg;
  render();
  setTimeout(() => {
    if (model.toast === msg) {
      model.toast = null;
      render();
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
            <button class="nav-btn ${model.tab === n.id ? "active" : ""}" data-tab="${n.id}">
              ${n.icon}
              <span>${n.label}</span>
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
      <div class="item-sub">${store ? store.name : "Odaberi trgovinu"} — Aktivna košarica</div>
      <div class="big-stat">${formatCents(total)} u košarici</div>
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
          ${lastReceipt ? `${lastReceipt.store_name}: ${formatCents(lastReceipt.cart_total_cents)}` : "Još nema spremljenih računa."}
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
            .map((s) => `<option value="${s.id}" ${s.id === model.selectedStoreId ? "selected" : ""}>${s.name}</option>`)
            .join("")}
        </select>
        <button class="btn btn-primary" id="btn-camera-scan">📷 Skeniraj barkod kamerom</button>
        <button class="btn btn-outline" id="btn-online-lookup">🌐 Dohvati naziv/sliku online</button>
      </div>

      <div class="row-wrap">
        <input id="inp-barcode" placeholder="Barkod (EAN-13)" style="width:150px;" />
        <input id="inp-name" placeholder="Naziv artikla" style="flex:1; min-width:160px;" />
        <input id="inp-qty" type="number" step="0.001" value="1" style="width:85px;" />
        <select id="inp-unit" style="width:80px;">
          <option value="kom">kom</option>
          <option value="kg">kg</option>
          <option value="g">g</option>
          <option value="l">L</option>
          <option value="ml">ml</option>
        </select>
        <input id="inp-price" placeholder="Cijena (€)" style="width:105px;" />
        <button class="btn btn-primary" id="btn-add-cart">+ Dodaj</button>
      </div>
    </div>

    <div class="list-container">
      ${
        model.cart.length === 0
          ? `<div class="card item-sub">Košarica je trenutno prazna. Skenirajte barkod ili unesite artikl iznad.</div>`
          : model.cart
              .map(
                (item) => `
          <div class="list-row">
            <div class="list-row-left">
              <div class="item-thumb">
                ${item.image_data_url ? `<img src="${item.image_data_url}" alt="" />` : item.name.slice(0, 1).toUpperCase()}
              </div>
              <div>
                <div class="item-title">${item.name}</div>
                <div class="item-sub">${item.quantity} ${item.unit} × ${formatCents(item.unit_price_cents)} ${item.barcode ? `• ${item.barcode}` : ""}</div>
              </div>
            </div>
            <div class="row">
              <div class="item-price">${formatCents(item.total_cents)}</div>
              <button class="btn btn-danger-ghost no-print" data-remove-cart="${item.id}">✕</button>
            </div>
          </div>`,
              )
              .join("")
      }
    </div>

    <div class="sticky-bar">
      <div>
        <div style="font-size:18px;font-weight:700;">Ukupno u košarici: ${formatCents(total)}</div>
        <div class="row" style="margin-top:6px;">
          <span class="item-sub">Iznos na računu:</span>
          <input id="inp-receipt-total" value="${model.receiptTotalInput}" placeholder="0,00 €" style="width:110px;" />
          ${
            hasReceipt
              ? diff === 0
                ? `<span class="pill success">✓ Iznos odgovara računu! (0,00 €)</span>`
                : `<span class="pill warning">Razlika: ${formatCents(diff)}</span>`
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
        <input id="inp-new-list" placeholder="Naziv novog popisa (npr. Tjedna nabavka)..." style="flex:1;" />
        <select id="sel-list-store">
          ${model.stores.map((s) => `<option value="${s.id}">${s.name}</option>`).join("")}
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
            <div class="item-title" style="font-size:17px;">${activeList.name}</div>
            <div class="item-sub">${activeList.store_name ?? "Općenito"} • Procjena: ${formatCents(activeList.total_estimated_cents)}</div>
          </div>
          <button class="btn btn-subtle" id="btn-transfer-cart">Prebaci u košaricu →</button>
        </div>

        <div class="row-wrap" style="margin-bottom:14px;">
          <input id="inp-plan-name" placeholder="Artikl za popis..." style="flex:1;" />
          <input id="inp-plan-qty" type="number" step="0.1" value="1" style="width:80px;" />
          <select id="inp-plan-unit" style="width:75px;">
            <option value="kom">kom</option>
            <option value="kg">kg</option>
            <option value="g">g</option>
            <option value="l">L</option>
            <option value="ml">ml</option>
          </select>
          <input id="inp-plan-price" placeholder="Procjena (€)" style="width:110px;" />
          <button class="btn btn-primary" id="btn-add-planned">+ Dodaj</button>
        </div>

        <div class="list-container">
          ${activeList.items
            .map(
              (item) => `
            <div class="list-row">
              <label class="row" style="cursor:pointer;">
                <input type="checkbox" data-toggle-plan="${item.id}" ${item.checked ? "checked" : ""} />
                <span style="${item.checked ? "text-decoration:line-through;color:var(--text-subtle);" : "font-weight:600;"}">
                  ${item.name} (${item.quantity} ${item.unit})
                </span>
              </label>
              <div class="item-price">${formatCents(calcLineCents(item.quantity, item.unit, item.estimated_price_cents))}</div>
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
  return `
    <div class="page-header">
      <h1>Artikli koje ste već skenirali.</h1>
      <p>Pretražite bazu barkodova, dodajte fotografiju proizvoda i pratite zadnje cijene.</p>
    </div>

    <div class="card">
      <div class="row-wrap" style="margin-bottom: 12px;">
        <input id="inp-cat-barcode" placeholder="Barkod" style="width:140px;" />
        <input id="inp-cat-name" placeholder="Naziv artikla" style="flex:1;" />
        <input id="inp-cat-price" placeholder="Zadnja cijena (€)" style="width:120px;" />
        <label class="btn btn-outline" style="cursor:pointer;">
          📷 Uslikaj / Slika
          <input id="inp-cat-photo" type="file" accept="image/*" capture="environment" style="display:none;" />
        </label>
        <button class="btn btn-primary" id="btn-save-product">Spremi artikl →</button>
      </div>
      <input id="inp-cat-search" value="${model.productSearch}" placeholder="Pretraži po nazivu ili barkodu..." style="width:100%;" />
    </div>

    ${
      model.selectedProduct
        ? `
      <div class="card">
        <div class="row space-between" style="margin-bottom:10px;">
          <div class="row">
            <div class="item-thumb">
              ${
                model.selectedProduct.image_data_url
                  ? `<img src="${model.selectedProduct.image_data_url}" alt="" />`
                  : model.selectedProduct.name.slice(0, 1).toUpperCase()
              }
            </div>
            <div>
              <span class="pill">DETALJ ARTIKLA I USPOREDBA CIJENA PO TRGOVINAMA</span>
              <div class="item-title" style="font-size:17px;margin-top:4px;">${model.selectedProduct.name} (${model.selectedProduct.barcode})</div>
            </div>
          </div>
          <button class="btn btn-outline" id="btn-close-detail">Zatvori detalj</button>
        </div>
        <div class="list-container">
          ${
            model.selectedProductPrices.length === 0
              ? `<div class="item-sub">Za ovaj artikl zabilježena je zadnja cijena: ${formatCents(model.selectedProduct.last_price_cents)} (${model.selectedProduct.last_store_name || "Trgovina"}).</div>`
              : model.selectedProductPrices
                  .map(
                    (sp) => `
              <div class="list-row">
                <div>
                  <div class="item-title">${sp.store_name}</div>
                  <div class="item-sub">Ažurirano: ${sp.updated_at.slice(0, 10)}</div>
                </div>
                <div class="row">
                  ${sp.is_cheapest ? `<span class="pill success">★ NAJJEFTINIJE</span>` : ""}
                  <div class="item-price">${formatCents(sp.price_cents)}</div>
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
              .map(
                (p) => `
          <div class="list-row" style="cursor:pointer;" data-inspect-product="${p.barcode}">
            <div class="list-row-left">
              <div class="item-thumb">
                ${p.image_data_url ? `<img src="${p.image_data_url}" alt="" />` : p.name.slice(0, 1).toUpperCase()}
              </div>
              <div>
                <div class="item-title">${p.name}</div>
                <div class="item-sub">${p.category} • Barkod: ${p.barcode} • ${p.last_store_name || "Nepoznato"}</div>
              </div>
            </div>
            <div class="row">
              <span class="btn btn-subtle" style="padding:5px 10px;font-size:11px;">Usporedi cijene →</span>
              <div class="item-price">${formatCents(p.last_price_cents)}</div>
            </div>
          </div>`,
              )
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
        <input id="inp-store-name" placeholder="Naziv trgovine (npr. Plodine, DM, Pekara)..." style="flex:1;" />
        <input id="inp-store-loc" placeholder="Lokacija (opcionalno)" style="width:200px;" />
        <button class="btn btn-primary" id="btn-add-store">+ Dodaj trgovinu →</button>
      </div>
    </div>

    <div class="list-container">
      ${model.stores
        .map(
          (s) => `
        <div class="list-row">
          <div class="list-row-left">
            <div class="item-thumb" style="background:${s.color}18;color:${s.color};">
              ${s.name.slice(0, 1).toUpperCase()}
            </div>
            <div>
              <div class="item-title">${s.name}</div>
              <div class="item-sub">${s.location || "Hrvatska"} • ${s.item_count} artikala u bazi</div>
            </div>
          </div>
          <button class="btn btn-subtle" data-store-plan="${s.id}">Kreiraj popis →</button>
        </div>`,
        )
        .join("")}
    </div>
  `;
}

function renderSync(): string {
  const st = model.ledgerStats;
  const sizeKb = st ? (st.db_size_bytes / 1024).toFixed(1).replace(".", ",") : "0,0";
  const ownDevices = model.members.filter((m) => m.role === "own_device");
  const familyMembers = model.members.filter((m) => m.role === "family_member");
  const listNames = model.lists.map((l) => l.name).join(", ") || "Sve obiteljske liste";

  return `
    <div class="page-header">
      <h1>Sinkronizacija: Moji uređaji i Obiteljska mreža.</h1>
      <p>S vlastitim uređajima dijelite SVE (račune, katalog, slike i liste), a s članovima obitelji samo odabrane liste za kupnju.</p>
    </div>

    <div class="card">
      <div class="row space-between" style="margin-bottom:10px;">
        <div>
          <span class="pill">AKTIVNA MREŽA • GOCOP KLJUČ MREŽE</span>
          <div class="item-title" style="font-size:17px;margin-top:6px;">
            Mreža: ${st?.network_name ?? "Obiteljska mreža"} (Ovaj čvor: ${st?.node_id ?? "—"})
          </div>
        </div>
        <div class="row">
          <input id="inp-network-name" value="${st?.network_name ?? "Obiteljska mreža"}" placeholder="Naziv mreže (npr. Obitelj Kraljević)" style="width:210px;" />
          <button class="btn btn-outline" id="btn-save-net-name">Spremi naziv mreže</button>
        </div>
      </div>

      <div class="highlight-box" style="margin-top:10px;">
        <div class="row space-between">
          <div>
            <div class="item-title">Dodaj uređaj ili člana obitelji u mrežu (goCOP 6-znamenkasti SAS kod)</div>
            <div class="item-sub">Odaberite ulogu — vlastiti uređaj dobiva sve kanale (<code>osobno + obitelj</code>), a član obitelji samo kanal <code>obitelj</code> (liste za kupnju).</div>
          </div>
          <div class="pill" style="font-size:15px;padding:6px 12px;">SAS kod: ${model.pendingSasCode}</div>
        </div>
        <div class="row-wrap" style="margin-top:8px;">
          <input id="inp-member-name" placeholder="Naziv (npr. Moj Kućni PC ili Ana — iPhone 15)" style="flex:1;min-width:220px;" />
          <select id="sel-member-role" style="width:265px;">
            <option value="own_device" ${model.pairingRole === "own_device" ? "selected" : ""}>💻 Moj drugi uređaj (Dijeli SVE)</option>
            <option value="family_member" ${model.pairingRole === "family_member" ? "selected" : ""}>👨‍👩‍👧 Član obitelji (Dijeli SAMO liste)</option>
          </select>
          <input id="inp-member-lists" value="${listNames}" placeholder="Koje liste dijeliti (za obitelj)" style="width:220px;" />
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
              <div class="item-title">💻 Ovaj uređaj (${st?.node_id ?? "lokalni čvor"})</div>
              <div class="item-sub" style="color:var(--success);">✓ Osnivač mreže • Puni pristup svim kanalima</div>
            </div>
          </div>
          ${ownDevices
            .map(
              (m) => `
            <div class="list-row">
              <div>
                <div class="item-title">💻 ${m.name}</div>
                <div class="item-sub">SAS: ${m.sas_code} • Kanal: ${m.channels} (SVE)</div>
              </div>
              <button class="btn btn-danger-ghost" data-remove-member="${m.peer_id}">✕</button>
            </div>`,
            )
            .join("")}
        </div>
      </div>

      <div class="card" style="margin-bottom:0;">
        <div class="item-title">2. Članovi obitelji — Dijeli SAMO liste</div>
        <div class="item-sub" style="margin:4px 0 12px;">Kanal: <code>obitelj</code> (Zajedničke liste za kupnju; osobni računi ostaju privatni)</div>
        <div class="list-container">
          ${
            familyMembers.length === 0
              ? `<div class="list-row"><div class="item-sub">Još nema dodanih članova obitelji. Dodajte člana iznad pomoću SAS koda.</div></div>`
              : familyMembers
                  .map(
                    (m) => `
              <div class="list-row">
                <div>
                  <div class="item-title">👨‍👩‍👧 ${m.name}</div>
                  <div class="item-sub">SAS: ${m.sas_code} • Liste: ${m.shared_lists}</div>
                </div>
                <button class="btn btn-danger-ghost" data-remove-member="${m.peer_id}">✕</button>
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
        Čvor uređaja: <strong>${st?.node_id ?? "—"}</strong> • Aktivnih zapisa: <strong>${st?.unique_records ?? 0}</strong> • Ukupno verzija u knjizi: <strong>${st?.total_versions ?? 0}</strong> • Arhivirano (Tombstones): <strong>${st?.tombstones ?? 0}</strong> • Poznati čvorovi (Frontier): <strong>${st?.frontier_nodes ?? 1}</strong> • Veličina SQLite baze: <strong>${sizeKb} KB</strong>
      </div>
      <div class="item-sub">
        <strong>Pravilo automatskog čišćenja i oslobađanja prostora:</strong> Pri pokretanju i nakon svake sinkronizacije automatski se brišu zastarjeli međukoraci stariji od 30 dana ili iznad zadnje 3 izmjene po artiklu (uz čuvanje <code>MAX(version_id)</code> i <code>Frontier</code> granice), te se odmah pokreće <code>VACUUM; PRAGMA optimize;</code> kako bi SQLite fizički oslobodio diskovni prostor.
      </div>
    </div>

    <div class="card">
      <div class="item-title" style="margin-bottom:6px;">Izravna P2P sinkronizacija i ručni prijenos (.list2cart)</div>
      <div class="item-sub" style="margin-bottom:12px;">
        Podaci se nikada ne spremaju na vanjske poslužitelje. Sama sinkronizacija ovisi o vama i dostupnosti vaših uređaja — kada uređaji nisu istovremeno uključeni, promjene možete u bilo kojem trenutku prenijeti i preko <code>.list2cart</code> paketa:
      </div>
      <div class="row-wrap">
        <button class="btn btn-primary" id="btn-export-bundle">📤 Izvezi SVE (za moj drugi uređaj) →</button>
        <button class="btn btn-outline" id="btn-export-family-bundle">👨‍👩‍👧 Izvezi SAMO liste (za člana obitelji) →</button>
        <label class="btn btn-outline" style="cursor:pointer;">
          📥 Uvezi i spoji paket
          <input id="inp-import-bundle" type="file" accept=".json,.list2cart" style="display:none;" />
        </label>
      </div>
    </div>
  `;
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
            <input id="inp-modal-barcode" placeholder="Ili upišite / potvrdite barkod (npr. 3850102...)" style="flex:1;" />
            <button class="btn btn-primary" id="btn-confirm-scan">Primijeni →</button>
          </div>
        </div>
      </div>`
        : ""
    }
    ${model.toast ? `<div class="toast">${model.toast}</div>` : ""}
  `;

  bindEvents();
}

function bindEvents(): void {
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
      model.scannerOpen = true;
      render();
      try {
        const video = document.getElementById("scanner-video") as HTMLVideoElement | null;
        if (video && navigator.mediaDevices?.getUserMedia) {
          const stream = await navigator.mediaDevices.getUserMedia({
            video: { facingMode: "environment" },
          });
          video.srcObject = stream;
        }
      } catch {
        // Kamera nije dostupna na ovom uređaju; korisnik može učitati sliku ili upisati barkod.
      }
    };
  }

  const stopScannerCamera = () => {
    const video = document.getElementById("scanner-video") as HTMLVideoElement | null;
    const stream = video?.srcObject as MediaStream | null;
    stream?.getTracks().forEach((t) => t.stop());
  };

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
    inpModalPhoto.onchange = () => {
      const file = inpModalPhoto.files?.[0];
      if (!file) return;
      const reader = new FileReader();
      reader.onload = () => {
        model.draftPhotoDataUrl = typeof reader.result === "string" ? reader.result : null;
        showToast("Slika proizvoda postavljena kao ikona!");
      };
      reader.readAsDataURL(file);
    };
  }

  const btnConfirmScan = document.getElementById("btn-confirm-scan");
  if (btnConfirmScan) {
    btnConfirmScan.onclick = async () => {
      const code = (document.getElementById("inp-modal-barcode") as HTMLInputElement | null)?.value.trim() ?? "";
      stopScannerCamera();
      model.scannerOpen = false;
      render();
      if (code) {
        const bInp = document.getElementById("inp-barcode") as HTMLInputElement | null;
        if (bInp) bInp.value = code;
        const found = await getProduct(code);
        if (found) {
          const nameInp = document.getElementById("inp-name") as HTMLInputElement | null;
          const priceInp = document.getElementById("inp-price") as HTMLInputElement | null;
          if (nameInp) nameInp.value = found.name;
          if (priceInp && found.last_price_cents > 0) {
            priceInp.value = (found.last_price_cents / 100).toFixed(2);
          }
          if (found.image_data_url && !model.draftPhotoDataUrl) {
            model.draftPhotoDataUrl = found.image_data_url;
          }
          showToast(`Skenirano iz kataloga: ${found.name}`);
        } else {
          const online = await lookupBarcodeOnline(code).catch(() => null);
          if (online) {
            const nameInp = document.getElementById("inp-name") as HTMLInputElement | null;
            if (nameInp) nameInp.value = online.name;
            showToast(`Skenirano online: ${online.name}`);
          }
        }
      }
    };
  }

  const storeSelect = document.getElementById("cart-store-select") as HTMLSelectElement | null;
  if (storeSelect) {
    storeSelect.onchange = () => {
      model.selectedStoreId = Number(storeSelect.value);
    };
  }

  const barcodeInp = document.getElementById("inp-barcode") as HTMLInputElement | null;
  if (barcodeInp) {
    barcodeInp.onchange = async () => {
      const found = await getProduct(barcodeInp.value);
      if (found) {
        const nameInp = document.getElementById("inp-name") as HTMLInputElement | null;
        const priceInp = document.getElementById("inp-price") as HTMLInputElement | null;
        if (nameInp) nameInp.value = found.name;
        if (priceInp && found.last_price_cents > 0) {
          priceInp.value = (found.last_price_cents / 100).toFixed(2);
        }
        model.draftPhotoDataUrl = found.image_data_url ?? null;
        showToast(`Prepoznat artikl iz baze: ${found.name}`);
      }
    };
  }

  const btnOnline = document.getElementById("btn-online-lookup");
  if (btnOnline && barcodeInp) {
    btnOnline.onclick = async () => {
      const code = barcodeInp.value.trim();
      if (!code) return showToast("Unesite barkod za online pretragu.");
      try {
        const info = await lookupBarcodeOnline(code);
        if (info) {
          const nameInp = document.getElementById("inp-name") as HTMLInputElement | null;
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
      const barcode = (document.getElementById("inp-barcode") as HTMLInputElement).value.trim();
      const name = (document.getElementById("inp-name") as HTMLInputElement).value.trim();
      const qty = Number.parseFloat((document.getElementById("inp-qty") as HTMLInputElement).value) || 1;
      const unit = (document.getElementById("inp-unit") as HTMLSelectElement).value as Unit;
      const priceCents = parseEuroToCents((document.getElementById("inp-price") as HTMLInputElement).value);

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
        showToast(ver.is_matched ? "Račun spremljen (100% točan)!" : "Račun spremljen s razlikom.");
        model.cart = [];
        model.receiptTotalInput = "";
        render();
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

  const btnAddStore = document.getElementById("btn-add-store");
  if (btnAddStore) {
    btnAddStore.onclick = async () => {
      const name = (document.getElementById("inp-store-name") as HTMLInputElement).value.trim();
      const loc = (document.getElementById("inp-store-loc") as HTMLInputElement).value.trim();
      if (!name) return showToast("Unesite naziv trgovine.");
      try {
        await addStore(name, loc, "#1768D1");
        model.stores = await listStores();
        showToast(`Trgovina "${name}" dodana!`);
        render();
      } catch (e) {
        showToast(`Greška: ${e}`);
      }
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
      const barcode = (document.getElementById("inp-cat-barcode") as HTMLInputElement).value.trim();
      const name = (document.getElementById("inp-cat-name") as HTMLInputElement).value.trim();
      const priceCents = parseEuroToCents((document.getElementById("inp-cat-price") as HTMLInputElement).value);
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
        model.draftPhotoDataUrl = null;
        model.products = await listProducts(model.productSearch);
        showToast(`Artikl "${name}" spremljen!`);
        render();
      } catch (e) {
        showToast(`Greška: ${e}`);
      }
    };
  }

  const btnCreateList = document.getElementById("btn-create-list");
  if (btnCreateList) {
    btnCreateList.onclick = async () => {
      const name = (document.getElementById("inp-new-list") as HTMLInputElement).value.trim();
      const storeId = Number((document.getElementById("sel-list-store") as HTMLSelectElement).value);
      if (!name) return showToast("Unesite naziv popisa.");
      await createShoppingList(name, storeId);
      model.lists = await listShoppingLists();
      render();
    };
  }

  const btnAddPlanned = document.getElementById("btn-add-planned");
  if (btnAddPlanned && model.lists[0]) {
    btnAddPlanned.onclick = async () => {
      const name = (document.getElementById("inp-plan-name") as HTMLInputElement).value.trim();
      const qty = Number.parseFloat((document.getElementById("inp-plan-qty") as HTMLInputElement).value) || 1;
      const unit = (document.getElementById("inp-plan-unit") as HTMLSelectElement).value as Unit;
      const priceCents = parseEuroToCents((document.getElementById("inp-plan-price") as HTMLInputElement).value);
      if (!name) return;
      await addPlannedItem({
        listId: model.lists[0].id,
        name,
        quantity: qty,
        unit,
        estimatedPriceCents: priceCents,
      });
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
      showToast(`Prebačene stavke s popisa "${list.name}" u košaricu!`);
      render();
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
      model.selectedProduct = model.products.find((p) => p.barcode === barcode) ?? null;
      model.selectedProductPrices = await getProductStorePrices(barcode);
      render();
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
        showToast(`Sinkronizirano! Poslano ${rep.pushed_stores} trgovina, ${rep.pushed_products} artikala.`);
        render();
      } catch (e) {
        showToast(`Sinkronizacija nije uspjela: ${e}`);
      }
    };
  }

  const btnExportBundle = document.getElementById("btn-export-bundle");
  if (btnExportBundle) {
    btnExportBundle.onclick = async () => {
      try {
        const jsonStr = await exportSyncBundleJson("own_device");
        const blob = new Blob([jsonStr], { type: "application/json" });
        const url = URL.createObjectURL(blob);
        const a = document.createElement("a");
        a.href = url;
        a.download = `List2Cart_SVE_${new Date().toISOString().slice(0, 10)}.list2cart.json`;
        a.click();
        URL.revokeObjectURL(url);
        showToast("Puni paket za vaš drugi uređaj preuzet!");
      } catch (e) {
        showToast(`Greška izvoza paketa: ${e}`);
      }
    };
  }

  const btnExportFamilyBundle = document.getElementById("btn-export-family-bundle");
  if (btnExportFamilyBundle) {
    btnExportFamilyBundle.onclick = async () => {
      try {
        const jsonStr = await exportSyncBundleJson("family_member");
        const blob = new Blob([jsonStr], { type: "application/json" });
        const url = URL.createObjectURL(blob);
        const a = document.createElement("a");
        a.href = url;
        a.download = `List2Cart_OBITELJ_LISTE_${new Date().toISOString().slice(0, 10)}.list2cart.json`;
        a.click();
        URL.revokeObjectURL(url);
        showToast("Obiteljski paket (samo liste za kupnju) preuzet!");
      } catch (e) {
        showToast(`Greška izvoza obiteljskog paketa: ${e}`);
      }
    };
  }

  const btnSaveNetName = document.getElementById("btn-save-net-name");
  if (btnSaveNetName) {
    btnSaveNetName.onclick = async () => {
      const inp = document.getElementById("inp-network-name") as HTMLInputElement | null;
      if (!inp) return;
      await setNetworkName(inp.value);
      model.ledgerStats = await getLedgerStats();
      showToast(`Naziv mreže spremljen: ${model.ledgerStats.network_name}`);
      render();
    };
  }

  const btnAddMember = document.getElementById("btn-add-member");
  if (btnAddMember) {
    btnAddMember.onclick = async () => {
      const name = (document.getElementById("inp-member-name") as HTMLInputElement).value.trim();
      const role = (document.getElementById("sel-member-role") as HTMLSelectElement).value as PeerRole;
      const sharedLists = (document.getElementById("inp-member-lists") as HTMLInputElement).value.trim();
      if (!name) return showToast("Unesite naziv uređaja ili člana obitelji.");
      try {
        await addNetworkMember({
          name,
          role,
          sasCode: model.pendingSasCode,
          sharedLists,
        });
        model.members = await listNetworkMembers();
        model.pendingSasCode = await newSasCode();
        showToast(
          role === "own_device"
            ? `Uređaj "${name}" dodan (dijeli SVE)!`
            : `Član obitelji "${name}" dodan (dijeli samo liste za kupnju)!`,
        );
        render();
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
      showToast("Član/uređaj uklonjen iz mreže.");
      render();
    };
  });

  const inpImportBundle = document.getElementById("inp-import-bundle") as HTMLInputElement | null;
  if (inpImportBundle) {
    inpImportBundle.onchange = async () => {
      const file = inpImportBundle.files?.[0];
      if (!file) return;
      try {
        const text = await file.text();
        const [s, p, l] = await importSyncBundleJson(text);
        model.stores = await listStores();
        model.products = await listProducts();
        model.lists = await listShoppingLists();
        model.ledgerStats = await getLedgerStats();
        showToast(`Spojeno: ${s} trgovina, ${p} artikala, ${l} popisa!`);
        render();
      } catch (e) {
        showToast(`Greška uvoza paketa: ${e}`);
      }
    };
  }

  const btnCompact = document.getElementById("btn-compact-ledger");
  if (btnCompact) {
    btnCompact.onclick = async () => {
      try {
        const removed = await runLedgerCompact();
        model.ledgerStats = await getLedgerStats();
        showToast(`Knjiga verzija očišćena (uklonjeno ${removed} zastarjelih međukoraka).`);
        render();
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
    const pbUrl = await getPocketbaseUrl();
    if (pbUrl) model.pocketbaseUrl = pbUrl;
  } catch (e) {
    console.error("Init error:", e);
  }
  render();
}

void init();
