import type { AboutViewState, ViewHelpers } from "../contracts";

export function renderAbout(state: AboutViewState, helpers: ViewHelpers): string {
  const esc = helpers.escapeHtml;
  const appName = state.name.trim() || "List2Cart";
  const versionText = state.version.trim() ? `Verzija ${state.version.trim()}` : "Verzija nije dostupna";
  const licenseText = state.license.trim() || "EUPL-1.2";

  return `<section class="fs-page" aria-labelledby="fs-about-heading">
    <header class="fs-header">
      <div class="fs-header-copy">
        <h1 class="fs-title" id="fs-about-heading">O aplikaciji</h1>
        <p class="fs-subtitle">Privatna kupnja, na vašim uređajima.</p>
      </div>
    </header>

    <div class="fs-about-hero">
      <img class="fs-about-icon" src="${esc(state.iconUrl)}" alt="${esc(appName)}" width="72" height="72" />
      <div class="fs-about-brand">
        <div class="fs-about-name">${esc(appName)}</div>
        <div class="fs-about-version">${esc(versionText)}</div>
      </div>
    </div>

    <section class="fs-card">
      <h2 class="fs-card-title">Od popisa do provjerenog računa</h2>
      <p class="fs-card-sub">Planirajte kupnju, usporedite cijene i provjerite koliko ste potrošili.</p>
    </section>

    <section class="fs-card fs-card--success">
      <h2 class="fs-card-title">Vaši podaci ostaju kod vas</h2>
      <p class="fs-card-sub">Bez obveznog računa ili cloud poslužitelja. Razmjenjujete samo odabrane podatke s uređajima koje sami povežete.</p>
    </section>

    <section class="fs-card">
      <h2 class="fs-card-title">Otvoreni kod · ${esc(licenseText)}</h2>
      <p class="fs-card-sub">Licenca aplikacije: ${esc(licenseText)}. Komponente imaju vlastite licence.</p>
    </section>

    <button class="btn btn-primary fs-btn-full" type="button" data-goto="sync">Natrag na obitelj</button>
  </section>`;
}
