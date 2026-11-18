import { expect, test } from "bun:test";
import {
  compareBasket,
  formatDate,
  formatQuantity,
  resolveBasketProduct,
  sharedListNames,
} from "../src/shopping";
import type { Product, StorePrice } from "../src/types";

const sampleProducts: Product[] = [
  {
    barcode: "38501001",
    sync_id: "prod-1",
    name: "Mlijeko 2,8% 1L",
    category: "Mliječni",
    default_unit: "kom",
    last_price_cents: 120,
    last_store_name: "Konzum",
    updated_at: "2026-11-18T08:00:00Z",
    image_data_url: null,
  },
  {
    barcode: "38501002",
    sync_id: "prod-2",
    name: "Jabuka Zlatni delišes",
    category: "Voće",
    default_unit: "kg",
    last_price_cents: 180,
    last_store_name: "Lidl",
    updated_at: "2026-11-18T08:00:00Z",
    image_data_url: null,
  },
  {
    barcode: "38501003",
    sync_id: "prod-3",
    name: "Jogurt",
    category: "Mliječni",
    default_unit: "kom",
    last_price_cents: 90,
    last_store_name: "Konzum",
    updated_at: "2026-11-18T08:00:00Z",
    image_data_url: null,
  },
  {
    barcode: "38501004",
    sync_id: "prod-4",
    name: "Jogurt",
    category: "Mliječni",
    default_unit: "kom",
    last_price_cents: 150,
    last_store_name: "Lidl",
    updated_at: "2026-11-18T08:00:00Z",
    image_data_url: null,
  },
];

test("resolveBasketProduct resolves by barcode or unambiguous name only", () => {
  expect(
    resolveBasketProduct({ barcode: "38501001", name: "Bilo što", quantity: 1, unit: "kom" }, sampleProducts)?.barcode,
  ).toBe("38501001");
  expect(
    resolveBasketProduct({ name: "  mlijeko 2,8% 1l ", quantity: 1, unit: "kom" }, sampleProducts)?.barcode,
  ).toBe("38501001");
  // Ambiguous name ("Jogurt" matches two products) must return null unless barcode is given
  expect(
    resolveBasketProduct({ name: "Jogurt", quantity: 1, unit: "kom" }, sampleProducts),
  ).toBeNull();
});

test("compareBasket separates complete and partial baskets and converts compatible units", () => {
  const prices = new Map<string, StorePrice[]>([
    [
      "38501001",
      [
        { store_name: "Konzum", price_cents: 130, updated_at: "2026-11-17T10:00:00Z", is_cheapest: false },
        { store_name: "Lidl", price_cents: 110, updated_at: "2026-11-15T10:00:00Z", is_cheapest: true },
        { store_name: "Spar", price_cents: 125, updated_at: "2026-11-16T10:00:00Z", is_cheapest: false },
      ],
    ],
    [
      "38501002",
      [
        { store_name: "Konzum", price_cents: 200, updated_at: "2026-11-14T10:00:00Z", is_cheapest: false },
        { store_name: "Lidl", price_cents: 160, updated_at: "2026-11-16T10:00:00Z", is_cheapest: true },
      ],
    ],
  ]);

  const lines = [
    { barcode: "38501001", name: "Mlijeko 2,8% 1L", quantity: 2, unit: "kom" as const },
    { name: "Jabuka Zlatni delišes", quantity: 500, unit: "g" as const },
  ];

  const result = compareBasket(lines, sampleProducts, prices);
  expect(result.map((r) => r.store)).toEqual(["Lidl", "Konzum", "Spar"]);
  // Lidl: 2 * 110 + 0.5 * 160 = 220 + 80 = 300
  expect(result[0]).toEqual({
    store: "Lidl",
    totalCents: 300,
    matched: 2,
    count: 2,
    missing: [],
    oldestPrice: "2026-11-15T10:00:00Z",
  });
  // Konzum: 2 * 130 + 0.5 * 200 = 260 + 100 = 360
  expect(result[1].totalCents).toBe(360);
  // Spar is missing apples
  expect(result[2].missing).toEqual(["Jabuka Zlatni delišes"]);
});

test("sharedListNames maps sync_id and wildcard tokens", () => {
  const lists = [
    { sync_id: "list-uuid-1", name: "Tjedna kupnja" },
    { sync_id: "list-uuid-2", name: "Vikend roštilj" },
  ];
  expect(sharedListNames("*", lists)).toEqual(["Svi obiteljski popisi"]);
  expect(sharedListNames("list-uuid-1, Vikend roštilj", lists)).toEqual([
    "Tjedna kupnja",
    "Vikend roštilj",
  ]);
});

test("formatQuantity and formatDate format Croatian locale values", () => {
  expect(formatQuantity(1.5)).toContain("1");
  expect(formatDate("nevaljan-datum")).toBe("Datum nije zabilježen");
});
