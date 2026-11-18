import { calcLineCents } from "./api";
import type { Product, StorePrice, Unit } from "./types";

export interface BasketLine {
  barcode?: string | null;
  name: string;
  quantity: number;
  unit: Unit;
}

export interface BasketComparison {
  store: string;
  totalCents: number;
  matched: number;
  count: number;
  missing: string[];
  oldestPrice: string;
}

const normalized = (value: string) => value.trim().toLocaleLowerCase("hr");

export function resolveBasketProduct(line: BasketLine, products: Product[]): Product | null {
  const matches = line.barcode?.trim()
    ? products.filter(p => p.barcode === line.barcode)
    : products.filter(p => normalized(p.name) === normalized(line.name));
  // A name shared by several products must never silently choose a package.
  return matches.length === 1 ? matches[0] : null;
}

export function compareBasket(
  lines: BasketLine[], products: Product[], prices: Map<string, StorePrice[]>,
): BasketComparison[] {
  const rows = lines.map(line => ({ line, product: resolveBasketProduct(line, products) }));
  const stores = new Set<string>();
  for (const { product } of rows) {
    if (product) for (const price of prices.get(product.barcode) ?? []) stores.add(price.store_name);
  }
  return [...stores].map(store => {
    let totalCents = 0;
    let oldestPrice = "";
    const missing: string[] = [];
    for (const { line, product } of rows) {
      const price = product && (prices.get(product.barcode) ?? []).find(p => p.store_name === store);
      // Prices of different units or packages are not interchangeable.
      const compatible = product && (product.default_unit === line.unit
        || (["kg", "g"].includes(product.default_unit) && ["kg", "g"].includes(line.unit))
        || (["l", "ml"].includes(product.default_unit) && ["l", "ml"].includes(line.unit)));
      const cents = price && compatible ? calcLineCents(line.quantity, line.unit, price.price_cents) : 0;
      if (!price || !compatible || price.price_cents <= 0 || cents <= 0) {
        missing.push(line.name);
        continue;
      }
      totalCents += cents;
      if (!oldestPrice || price.updated_at < oldestPrice) oldestPrice = price.updated_at;
    }
    return { store, totalCents, matched: lines.length - missing.length, count: lines.length, missing, oldestPrice };
  }).sort((a, b) => Number(a.missing.length > 0) - Number(b.missing.length > 0) || a.totalCents - b.totalCents);
}

export function formatQuantity(quantity: number): string {
  return new Intl.NumberFormat("hr-HR", { maximumFractionDigits: 3 }).format(quantity);
}

export function formatDate(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? "Datum nije zabilježen" : date.toLocaleDateString("hr-HR");
}

export function sharedListNames(filter: string, lists: { sync_id: string; name: string }[]): string[] {
  const tokens = filter.split(/[,;\n]/).map(normalized).filter(Boolean);
  if (tokens.some(t => ["*", "sve", "sve obiteljske liste", "all"].includes(t))) return ["Svi obiteljski popisi"];
  return tokens.map(token => lists.find(l => normalized(l.sync_id) === token || normalized(l.name) === token)?.name ?? token);
}
