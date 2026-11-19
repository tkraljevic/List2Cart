import {test, expect} from "bun:test";
import {editCartItem} from "../src/ui/commerce/cart-editor";
import type {CartItem} from "../src/types";
const item: CartItem = {id:"one",barcode:"123",name:"Mlijeko",quantity:1,unit:"kom",unit_price_cents:129,total_cents:129};
test("editing quantities uses integer cent rounding and preserves item identity", () => {
  const result = editCartItem(item,{quantity:"250",unit:"ml",price:"1,29"});
  expect(result.ok).toBe(true);
  if(result.ok) expect(result.item).toEqual({...item,quantity:250,unit:"ml",total_cents:32});
  expect(item.total_cents).toBe(129);
});
test("editing rejects invalid precision, quantities and unsafe totals", () => {
  for(const values of [
    {quantity:"0",unit:"kom",price:"1.00"},
    {quantity:"-1",unit:"kom",price:"1.00"},
    {quantity:"1.0001",unit:"kg",price:"1.00"},
    {quantity:"1",unit:"kom",price:"1.001"},
    {quantity:"1000000000",unit:"kom",price:"99999999999.99"},
  ] as const) expect(editCartItem(item,values).ok).toBe(false);
  expect(editCartItem(item,{quantity:"2",unit:"kom",price:"0"}).ok).toBe(true);
});
