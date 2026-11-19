import type { Product } from "../../types";
import type { BranchSearch, ChainSources, HomeBranch, PriceSource, RetailProduct } from "../../api";
import { formatCents } from "../../api";

type Escape = (value: unknown) => string;

type ChainName = { name: string; locative?: string };

/** Chains read from their own websites; the same list and order as `CHAINS` in `retail.rs`. */
const DIRECT_CHAINS: Record<string, ChainName> = {
  konzum: { name: "Konzum", locative: "Konzumu" },
  lidl: { name: "Lidl", locative: "Lidlu" },
  kaufland: { name: "Kaufland", locative: "Kauflandu" },
  spar: { name: "Spar", locative: "Sparu" },
  plodine: { name: "Plodine", locative: "Plodinama" },
  eurospin: { name: "Eurospin", locative: "Eurospinu" },
  studenac: { name: "Studenac", locative: "Studencu" },
};
/**
 * Chains read from the cijene.dev daily archive, by its folder names. A chain the
 * archive adds later still works; it is shown by its folder name until named here.
 * A locative is given only where the form after "u" is certain.
 */
const ARCHIVE_CHAINS: Record<string, ChainName> = {
  tommy: { name: "Tommy", locative: "Tommyju" },
  dm: { name: "dm", locative: "dm-u" },
  ktc: { name: "KTC", locative: "KTC-u" },
  ntl: { name: "NTL", locative: "NTL-u" },
  metro: { name: "Metro" },
  ribola: { name: "Ribola", locative: "Riboli" },
  trgocentar: { name: "Trgocentar", locative: "Trgocentru" },
  zabac: { name: "Žabac", locative: "Žapcu" },
  vrutak: { name: "Vrutak", locative: "Vrutku" },
  roto: { name: "Roto" },
  boso: { name: "Boso" },
  brodokomerc: { name: "Brodokomerc", locative: "Brodokomercu" },
  lorenco: { name: "Lorenco" },
  "trgovina-krk": { name: "Trgovina Krk", locative: "Trgovini Krk" },
  jadranka_trgovina: { name: "Jadranka trgovina", locative: "Jadranka trgovini" },
  gavranovic: { name: "Gavranović" },
  branka: { name: "Branka" },
  djelo_vodice: { name: "Djelo Vodice" },
  bure: { name: "Bure" },
  dukat: { name: "Dukat" },
  stanic: { name: "Diskont Stanić", locative: "Diskontu Stanić" },
  stridon: { name: "Stridon", locative: "Stridonu" },
};
export const CHAINS = Object.keys(DIRECT_CHAINS);
/** Label of the one entry of a chain whose prices are the same in every shop (`retail_archive.rs`). */
const ALL_SHOPS = "Sve poslovnice (iste cijene)";
export const isArchiveChain = (chain: string) => !(chain in DIRECT_CHAINS);
const named = (chain: string): ChainName | undefined => DIRECT_CHAINS[chain] ?? ARCHIVE_CHAINS[chain];
export const chainName = (chain: string) => named(chain)?.name ?? chain.replace(/[_-]+/g, " ");
/** "U Konzumu", "U Plodinama": the store name as it follows "u", when it is known. */
export const chainLocative = (chain: string) => named(chain)?.locative ?? null;
/** The label of the button that searches one chain. */
export const chainSearchLabel = (chain: string) => {
  const locative = chainLocative(chain);
  return locative ? `U ${locative}` : `Samo ${chainName(chain)}`;
};
/** The chain a store of the user's own list belongs to, by its name ("Super Konzum"). */
export function chainForStoreName(storeName: string): string | null {
  // Whole words only, so short names like "dm" or "Bure" do not match inside "Burek".
  const words = (text: string) => ` ${text.split(/[^\p{L}\p{N}]+/u).map(fold).filter(Boolean).join(" ")} `;
  const store = words(storeName);
  const all = [...Object.keys(DIRECT_CHAINS), ...Object.keys(ARCHIVE_CHAINS)];
  return all.find(chain => store.includes(words(chainName(chain)))) ?? null;
}

/** Lowercase without diacritics and spaces, so "lino" finds "LINO LADA" and "Č" finds "c". */
export function fold(text: string): string {
  return text.toLocaleLowerCase("hr").replace(/đ/g, "d").normalize("NFD").replace(/[̀-ͯ]/g, "").replace(/[^\p{L}\p{N}]/gu, "");
}

function tokens(query: string): string[] {
  return query.split(/\s+/).map(fold).filter(Boolean);
}

/** Products from the user's own catalog whose name holds every word of the query. */
export function matchLocalProducts(products: readonly Product[], query: string, limit = 5): Product[] {
  const words = tokens(query);
  if (!words.length || query.trim().length < 2) return [];
  return products.filter(p => {
    const name = fold(p.name);
    return words.every(w => name.includes(w));
  }).slice(0, limit);
}

export interface Offer {
  chain: string;
  product: RetailProduct;
  /** What the shopper pays: the promotion price when there is one. */
  priceCents: number;
  /** Unit price adjusted to the promotion, when the retailer publishes one. */
  unitPriceCents: number | null;
}

export function formatOfferToast(offer: Offer): string {
  return `Dodano: ${offer.product.name} · ${chainName(offer.chain)} ${formatCents(offer.priceCents)}`;
}

export function effectiveOffer(chain: string, product: RetailProduct): Offer {
  const priceCents = product.special_price_cents ?? product.price_cents;
  const unit = product.unit_price_cents ?? null;
  const unitPriceCents = unit === null ? null
    : product.special_price_cents ? Math.round(unit * product.special_price_cents / product.price_cents) : unit;
  return { chain, product, priceCents, unitPriceCents };
}

/**
 * All offers, cheapest first. Package prices of different sizes are not
 * comparable, so offers with a unit price in the most common unit (kg, L or
 * piece) come first by that unit price; the rest follow by package price.
 */
export function rankOffers(results: readonly BranchSearch[]): Offer[] {
  const offers = results.flatMap(r => (r.results?.products ?? []).map(p => effectiveOffer(r.chain, p)));
  const counts = new Map<string, number>();
  for (const o of offers) if (o.unitPriceCents !== null && o.product.unit_basis) counts.set(o.product.unit_basis, (counts.get(o.product.unit_basis) ?? 0) + 1);
  const basis = [...counts.entries()].sort((a, b) => b[1] - a[1])[0]?.[0] ?? null;
  const comparable = (o: Offer) => basis !== null && o.unitPriceCents !== null && o.product.unit_basis === basis;
  return offers.sort((a, b) => {
    const ca = comparable(a), cb = comparable(b);
    if (ca !== cb) return ca ? -1 : 1;
    if (ca && cb && a.unitPriceCents !== b.unitPriceCents) return a.unitPriceCents! - b.unitPriceCents!;
    return a.priceCents - b.priceCents || a.product.name.localeCompare(b.product.name, "hr");
  });
}

/**
 * Branches of each chain in or near a place. A postal code also finds branches
 * with a nearby code (same first three, then first two digits); a place name is
 * matched against the branch label, which holds the town for every chain.
 */
/**
 * Town names written right after a postal code in branch labels ("… 10000 ZAGREB,1000"
 * or "… 10450 Jastrebarsko 1 …"), so a postal code also finds chains whose labels
 * name only the town (Kaufland, Spar).
 */
export function townsForPostalCode(all: readonly ChainSources[], code: string): string[] {
  const towns = new Set<string>();
  for (const chain of all) for (const source of chain.sources) {
    if (source.postal_code !== code) continue;
    const after = source.label.split(code)[1]?.split(/[,;]/)[0] ?? "";
    const words: string[] = [];
    for (const word of after.trim().split(/\s+/)) {
      if (!/^\p{L}[\p{L}.'-]*$/u.test(word)) break;
      words.push(word);
    }
    if (words.length) towns.add(fold(words.join(" ")));
  }
  return [...towns].filter(t => t.length >= 3);
}

export function branchesNear(all: readonly ChainSources[], place: string, limit = 40): Map<string, PriceSource[]> {
  const result = new Map<string, PriceSource[]>();
  const query = place.trim();
  const digits = /^\d{2,5}$/.test(query) ? query : null;
  const words = digits ? [] : tokens(query);
  const towns = digits?.length === 5 ? townsForPostalCode(all, digits) : [];
  for (const chain of all) {
    const ranked: { source: PriceSource; rank: number }[] = [];
    for (const source of chain.sources) {
      let rank = -1;
      if (digits) {
        const code = source.postal_code ?? "";
        if (code === digits) rank = 0;
        else if (code.startsWith(digits.slice(0, 3)) && digits.length >= 3) rank = 1;
        else if (code.startsWith(digits.slice(0, 2))) rank = 2;
        else if (!code && towns.some(t => fold(source.label).includes(t))) rank = 1;
        else if (fold(source.label).includes(digits)) rank = 3;
      } else if (words.length) {
        const label = fold(source.label);
        if (words.every(w => label.includes(w))) rank = 0;
      }
      // Konzum lists its online shop like a branch; it is no shop to walk into.
      // A chain with the same prices in every shop (dm) has one entry and no town;
      // it belongs to every place.
      if (rank < 0 && source.label === ALL_SHOPS) rank = 0;
      if (rank >= 0 && /^internet\b/i.test(source.label.trim())) rank += 10;
      if (rank >= 0) ranked.push({ source, rank });
    }
    ranked.sort((a, b) => a.rank - b.rank || a.source.label.localeCompare(b.source.label, "hr"));
    result.set(chain.chain, ranked.slice(0, limit).map(r => r.source));
  }
  return result;
}

/** A shorter, readable branch name: the label without the date, time and file ending. */
export function branchName(label: string): string {
  return label
    .replace(/\.(csv|xml)$/i, "")
    // A date standing alone, not eight digits inside a longer number (Kaufland labels start
    // with one); Plodine writes the time straight after it, Studenac uses 2026-11-19.
    .replace(/[\s,_-]*(?<!\d)(\d{1,2}\.\d{1,2}\.\d{4}|\d{4}-\d{2}-\d{2}|\d{8}(?:\d{6})?)(?!\d)[\s\S]*$/, "")
    .replace(/[,_]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

export interface QuickFindState {
  query: string;
  busy: boolean;
  error: string;
  results: BranchSearch[] | null;
  searchedQuery: string;
}

export interface QuickFindContext {
  local: Product[];
  home: readonly HomeBranch[];
  /** Chain of the list's own store, when it is one of the four. */
  listChain: string | null;
  escape: Escape;
}

const priceHtml = (o: Offer, escape: Escape) => {
  const unit = o.unitPriceCents !== null && o.product.unit_basis ? `<span class="item-sub">${escape(formatCents(o.unitPriceCents))}/${escape(o.product.unit_basis)}</span>` : "";
  const was = o.product.special_price_cents ? `<s class="item-sub">${escape(formatCents(o.product.price_cents))}</s> ` : "";
  return `<span class="qf-price"><strong>${was}${escape(formatCents(o.priceCents))}</strong>${unit}</span>`;
};

/** Credits cijene.dev and gives the archive's date, which can be a day behind. */
export function archiveNote(results: readonly BranchSearch[], escape: Escape): string {
  const archived = results.filter(r => isArchiveChain(r.chain) && r.results);
  if (!archived.length) return "";
  const dates = [...new Set(archived.map(r => r.results!.source.price_date))].sort();
  const day = (iso: string) => { const [y, m, d] = iso.split("-"); return d && m && y ? `${Number(d)}. ${Number(m)}. ${y}.` : iso; };
  return ` Podaci za ${escape(archived.map(r => chainName(r.chain)).join(", "))}: dnevna arhiva <a href="https://cijene.dev" target="_blank" rel="noreferrer">cijene.dev</a> od ${escape(dates.map(day).join(", "))}`;
}

export function renderQuickFind(state: QuickFindState, ctx: QuickFindContext): string {
  const { escape } = ctx;
  if (state.query.trim().length < 2) return "";
  const homeChains = ctx.home.map(h => h.chain);
  const local = ctx.local.length ? `<p class="qf-head">Tvoji artikli</p>${ctx.local.map(p => `<div class="qf-row"><span class="item-copy"><span class="item-title">${escape(p.name)}</span><span class="item-sub">${p.last_price_cents ? `zadnje: ${escape(p.last_store_name || "nepoznata trgovina")} ${escape(formatCents(p.last_price_cents))}` : "bez zabilježene cijene"}</span></span><button class="btn btn-outline qf-add" type="button" data-qf-local="${escape(p.barcode)}" aria-label="Dodaj ${escape(p.name)} na popis">+</button></div>`).join("")}` : "";
  const buttons = ctx.home.length
    ? `${ctx.listChain && homeChains.includes(ctx.listChain) ? `<button class="btn btn-outline" type="button" data-qf-search="${escape(ctx.listChain)}" ${state.busy ? "disabled" : ""}>${escape(chainSearchLabel(ctx.listChain))}</button>` : ""}<button class="btn btn-primary" type="button" data-qf-search="all" ${state.busy ? "disabled" : ""}>Najjeftinije u mojim trgovinama</button>`
    : `<button class="btn btn-primary" type="button" data-qf-setup>Odaberi svoje poslovnice</button><span class="item-sub">Jednom odaberete poslovnice i tražite u svim cjenicima odjednom.</span>`;
  let results = "";
  if (state.busy) results = `<p class="item-sub" role="status">Tražim u cjenicima…</p>`;
  else if (state.error) results = `<p class="error-inline" role="alert">${escape(state.error)}</p>`;
  else if (state.results && state.searchedQuery === state.query) {
    const offers = rankOffers(state.results).slice(0, 25);
    const failed = state.results.filter(r => r.error).map(r => `${chainName(r.chain)}: ${r.error}`);
    results = (offers.length
      ? offers.map((o, i) => `<div class="qf-row"><span class="item-copy"><span class="item-title">${escape(o.product.name)}</span><span class="item-sub"><strong>${escape(chainName(o.chain))}</strong>${o.product.package ? ` · ${escape(o.product.package)}` : ""}${o.product.special_price_cents ? " · akcija" : ""}</span></span>${priceHtml(o, escape)}<button class="btn btn-outline qf-add" type="button" data-qf-offer="${i}" aria-label="Dodaj ${escape(o.product.name)} na popis">+</button></div>`).join("")
      : `<p class="item-sub">U tvojim poslovnicama nema proizvoda s tim nazivom.</p>`)
      + (failed.length ? `<p class="item-sub" role="status">Nije pretraženo: ${escape(failed.join("; "))}</p>` : "")
      + `<p class="item-sub">Cijene su iz javnih cjenika trgovaca i ne jamče da je proizvod na polici.${archiveNote(state.results, escape)}</p>`;
  }
  return `<div class="quick-find" aria-label="Prijedlozi za ${escape(state.query)}">${local}
    <p class="qf-head">Traži u cjenicima</p><div class="row-wrap qf-actions">${buttons}${ctx.home.length ? `<button class="text-button" type="button" data-qf-setup>Moje poslovnice</button>` : ""}</div>${results}</div>`;
}

export interface BranchSetupState {
  place: string;
  all: ChainSources[] | null;
  busy: boolean;
  error: string;
  /** Chosen source URL per chain. */
  picked: Record<string, string>;
}

export function renderBranchSetup(state: BranchSetupState, home: readonly HomeBranch[], escape: Escape): string {
  const near = state.all && state.place.trim() ? branchesNear(state.all, state.place) : null;
  const field = (chain: string) => {
    const options = near?.get(chain) ?? [];
    const loadError = state.all?.find(c => c.chain === chain)?.error;
    const body = loadError ? `<span class="item-sub">Popis poslovnica nije učitan: ${escape(loadError)}</span>`
      : options.length ? `<select data-branch-pick="${escape(chain)}" aria-label="Poslovnica ${escape(chainName(chain))}"><option value="">Ne koristim ${escape(chainName(chain))}</option>${options.map(s => `<option value="${escape(s.url)}" ${state.picked[chain] === s.url ? "selected" : ""}>${escape(branchName(s.label))}</option>`).join("")}</select>`
      : `<span class="item-sub">Nema poslovnice za to mjesto.</span>`;
    return `<label class="field">${escape(chainName(chain))}${body}</label>`;
  };
  // Other chains are shown only where they have a branch near the place.
  const others = near && state.all ? state.all.map(c => c.chain).filter(chain => isArchiveChain(chain)
    && ((near.get(chain) ?? []).length > 0 || state.all!.some(c => c.chain === chain && c.error))) : [];
  const chains = near
    ? `<div class="row-wrap branch-picks">${CHAINS.map(field).join("")}</div>`
      + (others.length ? `<p class="qf-head">Ostali lanci u tom mjestu</p><p class="item-sub">Podaci iz dnevne arhive <a href="https://cijene.dev" target="_blank" rel="noreferrer">cijene.dev</a>, obično od jučer ili od danas poslijepodne. Pri prvoj pretrazi preuzima se cijeli cjenik lanca, kod velikih i do 12 MB.</p><div class="row-wrap branch-picks">${others.map(field).join("")}</div>` : "")
    : "";
  const saved = home.length ? `<p class="qf-head">Spremljene poslovnice</p>${home.map(h => `<div class="qf-row"><span class="item-copy"><span class="item-title">${escape(chainName(h.chain))}</span><span class="item-sub">${escape(branchName(h.label))}</span></span><button class="icon-button btn-danger-ghost" type="button" data-branch-remove="${escape(h.chain)}" aria-label="Ukloni poslovnicu ${escape(chainName(h.chain))}">×</button></div>`).join("")}` : "";
  return `<section class="card no-print" id="branch-setup" aria-label="Moje poslovnice"><div class="row space-between"><h2>Moje poslovnice</h2><button class="icon-button" type="button" data-branch-close aria-label="Zatvori odabir poslovnica">×</button></div>
    <p class="item-sub">Upiši grad ili poštanski broj. Za svaki lanac predložit ćemo poslovnicu u tom mjestu; spremaju se jednom, a cjenik se svaki dan nađe sam.</p>
    <form id="branch-place-form" class="row-wrap"><input id="branch-place" style="flex:1 1 220px;" aria-label="Grad ili poštanski broj" placeholder="Npr. Zagreb ili 10000" value="${escape(state.place)}" ${state.busy ? "disabled" : ""} /><button class="btn btn-outline" type="submit" ${state.busy ? "disabled" : ""}>${state.busy ? "Učitavam poslovnice…" : "Pronađi poslovnice"}</button></form>
    ${state.error ? `<p class="error-inline" role="alert">${escape(state.error)}</p>` : ""}
    ${chains ? `${chains}<button class="btn btn-primary" type="button" data-branch-save ${state.busy || !Object.values(state.picked).some(Boolean) ? "disabled" : ""}>Spremi poslovnice</button>` : ""}
    ${saved}</section>`;
}

/**
 * First branch near the place for each chain read directly, so saving takes one
 * click. A chain from the cijene.dev archive is picked only if it is already saved:
 * each one costs a download of its whole price list.
 */
export function suggestPicks(all: readonly ChainSources[], place: string, home: readonly HomeBranch[]): Record<string, string> {
  const picks: Record<string, string> = {};
  for (const [chain, sources] of branchesNear(all, place)) {
    const saved = home.find(h => h.chain === chain);
    const current = saved && sources.find(s => s.branch_key === saved.branch_key);
    const first = current ?? (isArchiveChain(chain) && !saved ? undefined : sources[0]);
    if (first) picks[chain] = first.url;
  }
  return picks;
}
