const paths = {
  pregled: '<rect x="3" y="3" width="7" height="7" rx="1.5"/><rect x="14" y="3" width="7" height="7" rx="1.5"/><rect x="3" y="14" width="7" height="7" rx="1.5"/><rect x="14" y="14" width="7" height="7" rx="1.5"/>',
  planer: '<path d="M7 3h10v18H7zM10 7h4m-4 4h4m-4 4h4"/>',
  kosarica: '<path d="M2 3h3l3 12h11l3-9H6M9 20h.01M18 20h.01"/><circle cx="9" cy="20" r="1"/><circle cx="18" cy="20" r="1"/>',
  racuni: '<path d="M6 3h12v18l-3-2-3 2-3-2-3 2V3zm3 4h6m-6 4h6m-6 4h3"/>',
  katalog: '<path d="M3 4h8l10 10-7 7L3 10V4z"/><circle cx="7" cy="8" r="1"/>',
  cijene: '<path d="M4 20h17M7 16V9m5 7V4m5 12v-6"/>',
  trgovine: '<path d="M3 9l2-6h14l2 6M4 10v11h16V10M9 21v-7h6v7M2 9c0 3 4 3 5 0 1 3 4 3 5 0 1 3 4 3 5 0 1 3 5 3 5 0H2z"/>',
  sync: '<circle cx="9" cy="7" r="3"/><path d="M2 21v-3a7 7 0 0 1 14 0v3m1-17a3 3 0 0 1 0 6m1 4a6 6 0 0 1 4 6v1"/>',
  scan: '<path d="M3 8V3h5m8 0h5v5m0 8v5h-5M8 21H3v-5m4-9v10m3-10v10m4-10v10m3-10v10"/>',
  back: '<path d="m14 5-7 7 7 7M7 12h14"/>',
  close: '<path d="m6 6 12 12M6 18 18 6"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
};

export function icon(name: keyof typeof paths): string {
  return `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${paths[name]}</svg>`;
}
