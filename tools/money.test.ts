import { test, expect } from "bun:test";
import { calcLineCents, formatCents } from "../src/api";

test("UI matches Rust integer cent rounding and unit conversion", () => {
  expect(calcLineCents(1.005, "kom", 100)).toBe(101);
  expect(calcLineCents(1.25, "kg", 130)).toBe(163);
  expect(calcLineCents(500, "g", 130)).toBe(65);
  expect(calcLineCents(250, "ml", 200)).toBe(50);
  expect(formatCents(298)).toBe("2,98 €");
  expect(formatCents(-1)).toBe("-0,01 €");
  expect(formatCents(Number.MAX_SAFE_INTEGER)).toBe("90071992547409,91 €");
});

test("UI rejects nonfinite, excessive precision and unsafe amounts", () => {
  for (const q of [NaN, Infinity, -1, 0, 0.0001, 1.0001]) {
    expect(calcLineCents(q, "kg", 100)).toBe(0);
  }
  expect(calcLineCents(2, "kom", Number.MAX_SAFE_INTEGER)).toBe(0);
  expect(calcLineCents(1, "kom", Number.MAX_SAFE_INTEGER + 1)).toBe(0);
});
