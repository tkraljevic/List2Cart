import type { PairingViewState, ViewHelpers } from "../contracts";

function formatSasCode(raw: string | null | undefined): string {
  if (!raw) return "—";
  const digits = raw.replace(/\s+/g, "");
  if (/^\d{6}$/.test(digits)) {
    return `${digits.slice(0, 3)} ${digits.slice(3)}`;
  }
  return raw;
}

export function renderPairingDialog(state: PairingViewState, helpers: ViewHelpers): string {
  if (!state.pairingOpen) return "";

  const esc = helpers.escapeHtml;
  const cancelling = state.pairingAction === "cancelling";
  const confirming = state.pairingAction === "confirming";
  const canCancel = !cancelling && (!state.pairingBusy || confirming);
  const busyTitle = cancelling ? "Prekidanje povezivanja…" : "Obrada povezivanja…";
  const family = state.pairingRole === "family_member";
  const selected = (state.draftMemberLists ?? "").split(",").filter(Boolean);
  const ps = state.pairStatus;
  const isLan = state.pairMode === "lan";
  const canAdmit = ps?.can_admit ?? false;
  const admit = canAdmit && state.pairingAdmit;
  const networkName = state.ledgerStats?.network_name?.trim() || "Obiteljska mreža";

  if (isLan) {
    const discoverable = state.lanPeersDiscovered.filter((p) => (p.pairPort ?? 0) > 0);

    return `<div class="modal-backdrop">
      <section class="modal-card fs-modal-card" role="dialog" aria-modal="true" aria-labelledby="pairing-title">
        <div class="fs-modal-header">
          <div>
            <h2 class="fs-modal-title" id="pairing-title">${ps?.pending ? (state.pairingBusy ? busyTitle : "Usporedite kod") : "Poveži uređaj ili člana"}</h2>
            <p class="fs-modal-sub">${ps?.pending
              ? `${esc(ps.peer_name || "Nepoznati uređaj")} · ${esc(networkName)}`
              : `Mreža: ${esc(networkName)} · Pristup se daje tek nakon obostrane potvrde koda.`}</p>
          </div>
          <button class="icon-button" type="button" data-close-dialog aria-label="Odustani od povezivanja" ${canCancel ? "" : "disabled"}>×</button>
        </div>

        ${!ps?.pending
          ? `<div class="fs-mode-tabs">
               <button class="btn btn-primary fs-btn-full" type="button" data-pair-mode="lan">Izravno na mreži</button>
               <button class="btn btn-outline fs-btn-full" type="button" data-pair-mode="offline">Ručna evidencija</button>
             </div>`
          : ""}

        ${ps?.error
          ? `<div class="fs-banner fs-banner--warning" role="alert">
               <div class="fs-banner-title">Povezivanje je zaustavljeno · Podaci nisu podijeljeni</div>
               <p class="fs-banner-text">${esc(ps.error)}</p>
             </div>`
          : ""}

        ${ps?.pending
          ? `
            <div class="fs-sas-box">
              <div class="fs-sas-code" aria-label="Sigurnosni kod za usporedbu">${esc(formatSasCode(ps.sas))}</div>
              <div class="fs-sas-hint">Kod mora biti jednak na oba uređaja.</div>
            </div>

            <div class="fs-section-box">
              <div class="fs-member-name">${esc(ps.peer_name || "Nepoznati uređaj")}</div>
              <div class="fs-member-meta">Adresa: ${esc(ps.peer_host ?? "—")}</div>
              <div class="fs-member-meta">Ključ: <code>${esc((ps.peer_public_key ?? "").slice(0, 18))}…</code></div>
            </div>

            ${canAdmit
              ? `<label class="field">Smjer povezivanja
                   <select id="sel-pair-direction" ${state.pairingBusy ? "disabled" : ""}>
                     <option value="admit" ${admit ? "selected" : ""}>Pozovi u moju mrežu (${esc(networkName)})</option>
                     <option value="join" ${admit ? "" : "selected"}>Pridruži se mreži partnera</option>
                   </select>
                 </label>`
              : '<p class="fs-card-sub">Ovaj uređaj pridružuje se mreži partnera i preuzet će njegov ključ mreže nakon obostrane potvrde.</p>'}

            ${admit
              ? `<label class="field">Pristup podacima
                   <select id="sel-member-role" ${state.pairingBusy ? "disabled" : ""}>
                     <option value="family_member" ${family ? "selected" : ""}>Član obitelji · samo odabrani popisi</option>
                     <option value="own_device" ${family ? "" : "selected"}>Moj drugi uređaj · svi moji osobni podaci</option>
                   </select>
                 </label>
                 ${family
                   ? `<fieldset class="list-choices">
                        <legend>Popisi za dijeljenje</legend>
                        ${state.lists.length
                          ? state.lists
                              .map(
                                (l) =>
                                  `<label class="check-option"><input type="checkbox" data-pair-list="${esc(l.sync_id)}" ${selected.includes(l.sync_id) ? "checked" : ""} ${state.pairingBusy ? "disabled" : ""} /><span>${esc(l.name)}</span></label>`,
                              )
                              .join("")
                          : '<p class="fs-card-sub">Kreirajte popis prije dodavanja člana obitelji.</p>'}
                      </fieldset>
                      <p class="fs-footnote">Osobni računi i ostali popisi ostaju privatni.</p>`
                   : '<div class="fs-section-box"><p class="fs-card-sub">Računi, katalog i svi moji popisi. Odaberite samo za uređaj koji osobno koristite.</p></div>'}`
              : '<p class="fs-footnote">Partner na drugom uređaju treba odabrati "Pozovi u moju mrežu" i potvrditi isti kod.</p>'}

            ${state.pairingBusy
              ? `<div class="fs-banner fs-banner--info" role="status" aria-live="polite">
                   <div class="fs-banner-title">${busyTitle}</div>
                   <p class="fs-banner-text">${cancelling ? "Prekida se aktivno čekanje i veza s drugim uređajem." : "Povezivanje još nije dovršeno. Čeka se ishod obostrane provjere koda. Prekid zaustavlja čekanje na ovom uređaju, ali ne povlači već poslanu potvrdu."}</p>
                 </div>`
              : `<label class="fs-confirm-box ${state.pairingConfirmed ? "fs-confirm-box--checked" : ""}">
                   <input id="pairing-confirmed" type="checkbox" ${state.pairingConfirmed ? "checked" : ""} ${state.pairingBusy ? "disabled" : ""} />
                   <span>Provjerio/la sam: kod je jednak na oba uređaja.</span>
                 </label>`}

            <div class="fs-dialog-actions">
              <button class="btn btn-primary fs-btn-full" type="button" id="btn-confirm-live-pairing" ${!state.pairingConfirmed || state.pairingBusy || (admit && family && !selected.length) ? "disabled" : ""}>${state.pairingBusy
                ? busyTitle
                : state.pairingConfirmed
                  ? "Potvrdi povezivanje"
                  : "Prvo usporedite i označite kod"}</button>
              <button class="btn btn-outline fs-btn-full" type="button" id="btn-reject-live-pairing" ${canCancel ? "" : "disabled"}>${cancelling ? "Prekidanje…" : "Kodovi se razlikuju / Odustani"}</button>
            </div>
          `
          : `
            <label class="field">Naziv ovog uređaja
              <input id="inp-device-name" value="${esc(state.draftDeviceName)}" placeholder="Npr. Tomislav · MacBook" ${state.pairingBusy ? "disabled" : ""} />
            </label>

            <div class="fs-section-box">
              <div class="fs-member-name">1. Pozovi u moju mrežu</div>
              <p class="fs-card-sub">Pokrenite čekanje na ovom uređaju, a zatim se povežite s drugog uređaja.</p>
              ${ps?.waiting
                ? `<div class="fs-banner fs-banner--info" role="status">
                     <div class="fs-banner-title">Čeka se povezivanje na portu ${esc(ps.listen_port ?? "?")}</div>
                     <p class="fs-banner-text">Na drugom uređaju otvorite "Pridruži se drugoj mreži" i pronađite ovaj uređaj.</p>
                   </div>
                   <button class="btn btn-outline fs-btn-full" type="button" id="btn-stop-pair-listen" ${state.pairingBusy ? "disabled" : ""}>Zaustavi čekanje</button>`
                : `<button class="btn btn-primary fs-btn-full" type="button" id="btn-start-pair-listen" ${state.pairingBusy ? "disabled" : ""}>Čekaj povezivanje na ovom uređaju</button>`}
            </div>

            <div class="fs-section-box">
              <div class="fs-member-name">2. Pridruži se mreži partnera</div>
              <p class="fs-card-sub">Pronađite uređaj koji vas poziva na lokalnoj mreži ili upišite njegovu adresu.</p>
              <button class="btn btn-outline fs-btn-full" type="button" id="btn-discover-pair-peers" ${state.pairingBusy ? "disabled" : ""}>Pronađi uređaj na mreži</button>
              ${discoverable.length
                ? discoverable
                    .map(
                      (p) => `<div class="fs-peer-found-row">
                        <div class="fs-member-copy">
                          <div class="fs-member-name">${esc(p.name)}</div>
                          <div class="fs-member-meta">${esc(p.addr)}:${esc(p.pairPort ?? "")}</div>
                        </div>
                        <button class="btn btn-primary" type="button" data-dial-discovered="${esc(`${p.addr}:${p.pairPort}`)}" ${state.pairingBusy ? "disabled" : ""}>Poveži se</button>
                      </div>`,
                    )
                    .join("")
                : ""}
              <div class="fs-inline-form">
                <input class="fs-input" id="inp-pair-addr" value="${esc(state.draftPairAddr)}" placeholder="192.168.1.25:45123" ${state.pairingBusy ? "disabled" : ""} />
                <button class="btn btn-primary" type="button" id="btn-dial-pair" ${state.pairingBusy ? "disabled" : ""}>Nastavi s adresom</button>
              </div>
            </div>

            <p class="fs-footnote">Pristup se ne daje bez obostrane potvrde koda. Prije promjene mreže provjerite s kime ćete razmjenjivati podatke.</p>
          `}
      </section>
    </div>`;
  }

  return `<div class="modal-backdrop">
    <section class="modal-card fs-modal-card" role="dialog" aria-modal="true" aria-labelledby="pairing-title">
      <div class="fs-modal-header">
        <div>
          <h2 class="fs-modal-title" id="pairing-title">Ručna evidencija uređaja</h2>
          <p class="fs-modal-sub">Za razmjenu potpisanih paketa datotekom kad uređaji nisu na istoj mreži.</p>
        </div>
        <button class="icon-button" type="button" data-close-dialog aria-label="Odustani od dodavanja" ${state.pairingBusy ? "disabled" : ""}>×</button>
      </div>

      <div class="fs-mode-tabs">
        <button class="btn btn-outline fs-btn-full" type="button" data-pair-mode="lan">Izravno na mreži</button>
        <button class="btn btn-primary fs-btn-full" type="button" data-pair-mode="offline">Ručna evidencija</button>
      </div>

      <div class="fs-sas-box">
        <div class="fs-sas-code" aria-label="Kod za usporedbu">${esc(formatSasCode(state.pendingSasCode))}</div>
        <div class="fs-sas-hint">Usporedite kod s drugim uređajem prije dodavanja u evidenciju.</div>
      </div>

      <label class="field">Naziv uređaja ili osobe
        <input id="inp-member-name" value="${esc(state.draftMemberName)}" placeholder="Npr. Ana · iPhone" ${state.pairingBusy ? "disabled" : ""} />
      </label>

      <label class="field">Pristup podacima
        <select id="sel-member-role" ${state.pairingBusy ? "disabled" : ""}>
          <option value="family_member" ${family ? "selected" : ""}>Član obitelji · odabrani popisi</option>
          <option value="own_device" ${family ? "" : "selected"}>Moj drugi uređaj · svi moji podaci</option>
        </select>
      </label>

      ${family
        ? `<fieldset class="list-choices">
             <legend>Popisi za dijeljenje</legend>
             ${state.lists.length
               ? state.lists
                   .map(
                     (l) =>
                       `<label class="check-option"><input type="checkbox" data-pair-list="${esc(l.sync_id)}" ${selected.includes(l.sync_id) ? "checked" : ""} ${state.pairingBusy ? "disabled" : ""} /><span>${esc(l.name)}</span></label>`,
                   )
                   .join("")
               : '<p class="fs-card-sub">Kreirajte popis prije dodavanja člana obitelji.</p>'}
           </fieldset>
           <p class="fs-footnote">Osobni računi nisu dio obiteljskog paketa.</p>`
        : '<div class="fs-section-box"><p class="fs-card-sub">Ovaj uređaj moći će primati i osobne račune, katalog i sve popise.</p></div>'}

      <label class="fs-confirm-box ${state.pairingConfirmed ? "fs-confirm-box--checked" : ""}">
        <input id="pairing-confirmed" type="checkbox" ${state.pairingConfirmed ? "checked" : ""} ${state.pairingBusy ? "disabled" : ""} />
        <span>Provjerio/la sam: kod je jednak na oba uređaja.</span>
      </label>

      <div class="fs-dialog-actions">
        <button class="btn btn-primary fs-btn-full" type="button" id="btn-add-member" ${!state.pairingConfirmed || state.pairingBusy || (family && !selected.length) ? "disabled" : ""}>${state.pairingBusy ? "Spremanje…" : "Potvrdi i dodaj"}</button>
        <button class="btn btn-outline fs-btn-full" type="button" id="btn-reject-pairing" ${state.pairingBusy ? "disabled" : ""}>Kodovi se razlikuju / Odustani</button>
      </div>
    </section>
  </div>`;
}
