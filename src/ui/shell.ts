import { icon } from "../icons";
import appIconUrl from "../assets/app-icon.png";
import type { AppTab } from "./contracts";

const navigation = [
  { id: "planer", label: "Popisi" },
  { id: "kosarica", label: "Kupnja" },
  { id: "racuni", label: "Računi" },
  { id: "katalog", label: "Proizvodi" },
  { id: "sync", label: "Obitelj" },
] as const;

export function renderNavigation(tab: AppTab): string {
  const active = ["cijene", "trgovine"].includes(tab) ? "katalog"
    : ["napredno", "about"].includes(tab) ? "sync" : tab;
  const buttons = navigation.map(({id, label}) => `<button class="nav-btn ${active === id ? "active" : ""}" data-tab="${id}" ${active === id ? 'aria-current="page"' : ""}>${icon(id)}<span>${label}</span></button>`).join("");
  return `<aside class="sidebar no-print">
    <div class="brand-box"><img class="brand-logo" src="${appIconUrl}" alt="" width="38" height="38"><div><div class="brand-title">List2Cart</div><div class="brand-tag">Privatna kupnja</div></div></div>
    <nav class="nav-list" aria-label="Glavna navigacija">${buttons}</nav>
    <button class="back-link" data-tab="pregled">${icon("pregled")} Pregled potrošnje</button>
    <div class="sidebar-footer"><strong>Spremljeno lokalno</strong><p>Razmjena samo s odabranim uređajima.</p></div>
  </aside><nav class="mobile-nav no-print" aria-label="Mobilna navigacija">${buttons}</nav>`;
}

export function renderProductTabs(tab: AppTab): string {
  if (!["katalog", "cijene", "trgovine"].includes(tab)) return "";
  return `<nav class="product-tabs no-print" aria-label="Proizvodi">${([
    ["katalog", "Katalog"], ["cijene", "Cijene"], ["trgovine", "Trgovine"],
  ] as const).map(([id, label]) => `<button class="btn ${tab === id ? "btn-primary" : "btn-subtle"}" data-goto="${id}" ${tab === id ? 'aria-current="page"' : ""}>${label}</button>`).join("")}</nav>`;
}
