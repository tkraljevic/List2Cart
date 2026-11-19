import type { ShoppingList } from "../../types";
import { calcLineCents, formatCents } from "../../api";

export function renderListCards(lists: readonly ShoppingList[], selectedId: number | undefined, escape: (value: unknown) => string): string {
  if (!lists.length) return "";
  return `<section class="commerce-lists" aria-label="Vaši popisi">${lists.map(list => {
    const total = list.items.reduce((sum, item) => sum + calcLineCents(item.quantity, item.unit, item.estimated_price_cents),0);
    const checked = list.items.filter(item => item.checked).length;
    return `<button class="card commerce-list-card ${list.id === selectedId ? "is-selected" : ""}" data-open-list="${list.id}" aria-pressed="${list.id === selectedId}">
      <strong>${escape(list.name)}</strong>
      <span class="item-sub">${list.items.length} stavki · Procjena ${escape(formatCents(total))}</span>
      <span class="item-sub">${list.store_name ? escape(list.store_name) + " · " : ""}${checked} kupljeno</span>
      <span class="commerce-list-preview">${list.items.length ? escape(list.items.slice(0,3).map(item => item.name).join(" · ")) : "Prazan popis · Dodajte prvi artikl"}</span>
      <span class="text-button">${list.id === selectedId ? "Otvoren popis" : "Otvori popis →"}</span>
    </button>`;
  }).join("")}</section>`;
}
