import { calcLineCents, formatCents } from "../../api";
import type { CartItem, Unit } from "../../types";

export interface CartEditValues { quantity: string; unit: Unit; price: string }
export type CartEditResult = { ok: true; item: CartItem } | { ok: false; error: string };

export function editCartItem(item: CartItem, values: CartEditValues): CartEditResult {
  const rawQuantity = values.quantity.trim().replace(",", ".");
  const rawPrice = values.price.trim().replace(",", ".");
  if (!/^\d+(\.\d{1,3})?$/.test(rawQuantity) || Number(rawQuantity) <= 0 || Number(rawQuantity) > 1e9) {
    return {ok:false, error:"Unesite količinu veću od nule, s najviše tri decimale."};
  }
  if (!/^\d+(\.\d{1,2})?$/.test(rawPrice)) {
    return {ok:false, error:"Unesite cijenu u eurima, s najviše dvije decimale."};
  }
  const quantity = Number(rawQuantity);
  const price = Math.round(Number(rawPrice) * 100);
  if (!["kom", "kg", "g", "l", "ml"].includes(values.unit) || !Number.isSafeInteger(price) || price < 0) {
    return {ok:false, error:"Provjerite jedinicu i cijenu."};
  }
  const total = calcLineCents(quantity, values.unit, price);
  // A positive price that rounds to zero is valid; unsafe multiplication is not.
  const denominator = values.unit === "g" || values.unit === "ml" ? 1000000n : 1000n;
  const exact = (BigInt(Math.round(quantity * 1000)) * BigInt(price) + denominator / 2n) / denominator;
  if (exact > BigInt(Number.MAX_SAFE_INTEGER)) return {ok:false, error:"Iznos je prevelik."};
  return {ok:true, item:{...item, quantity, unit:values.unit, unit_price_cents:price, total_cents:total}};
}

export function renderCartEditor(item: CartItem, escape: (value: unknown) => string): string {
  const unitOptions = (["kom", "kg", "g", "l", "ml"] as Unit[]).map(unit => `<option value="${unit}" ${unit === item.unit ? "selected" : ""}>${unit === "l" ? "L" : unit}</option>`).join("");
  return `<div class="modal-backdrop"><section class="modal-card commerce-editor" role="dialog" aria-modal="true" aria-labelledby="cart-edit-title">
    <div class="row space-between"><h2 id="cart-edit-title">Količina i cijena</h2><button class="icon-button" data-cancel-cart-edit aria-label="Zatvori uređivanje">×</button></div>
    <p class="item-title">${escape(item.name)}</p>
    <form id="cart-edit-form">
      <label class="field">Količina<input id="cart-edit-quantity" inputmode="decimal" value="${escape(item.quantity)}" required></label>
      <label class="field">Jedinica<select id="cart-edit-unit">${unitOptions}</select></label>
      <label class="field">Cijena po komadu / kg / L (€)<input id="cart-edit-price" inputmode="decimal" value="${escape((item.unit_price_cents / 100).toFixed(2).replace(".", ","))}" required></label>
      <div class="row space-between"><span>Ukupno za stavku</span><strong id="cart-edit-total">${escape(formatCents(item.total_cents))}</strong></div>
      <p id="cart-edit-error" class="error-inline" role="status"></p>
      <div class="dialog-actions"><button class="btn btn-outline" type="button" data-cancel-cart-edit>Odustani</button><button class="btn btn-primary" type="submit">Spremi izmjene</button></div>
    </form></section></div>`;
}
