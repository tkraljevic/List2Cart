import type { PeerSyncRecord } from "../../api";
import type { NetworkMember } from "../../types";
import type { FamilyViewState, ViewHelpers } from "../contracts";

function findPeerRecord(state: FamilyViewState, peerId: string): PeerSyncRecord | undefined {
  return state.lanStatus?.peers.find((p) => p.peer_public_key === peerId);
}

function formatPeerConfirmation(rec: PeerSyncRecord | undefined, helpers: ViewHelpers): {
  badgeClass: string;
  badgeText: string;
  detailLine: string;
} {
  if (!rec) {
    return {
      badgeClass: "fs-badge--neutral",
      badgeText: "Čeka prvu razmjenu",
      detailLine: "Još nije potvrđena razmjena s ovim uređajem.",
    };
  }
  if (rec.last_error) {
    return {
      badgeClass: "fs-badge--error",
      badgeText: "Veza nije uspjela",
      detailLine: rec.last_sync_at
        ? `Zadnja potvrđena razmjena: ${helpers.formatDate(rec.last_sync_at)} · primljeno ${rec.last_applied}, poslano ${rec.last_sent}`
        : "Još nema potvrđene razmjene s ovim uređajem.",
    };
  }
  if (rec.last_sync_at) {
    const pathLabel =
      rec.last_path === "lan"
        ? "LAN"
        : rec.last_path === "manual"
          ? "Ručna adresa"
          : "Izravna veza";
    return {
      badgeClass: "fs-badge--success",
      badgeText: "Potvrđeno",
      detailLine: `${pathLabel} · zadnja potvrda ${helpers.formatDate(rec.last_sync_at)} · primljeno ${rec.last_applied}, poslano ${rec.last_sent}`,
    };
  }
  return {
    badgeClass: "fs-badge--neutral",
    badgeText: "Čeka prvu razmjenu",
    detailLine: "Lokalno zabilježen uređaj; primitak još nije potvrđen.",
  };
}

function renderPeerControls(
  member: NetworkMember,
  state: FamilyViewState,
  helpers: ViewHelpers,
): string {
  const esc = helpers.escapeHtml;
  const rec = findPeerRecord(state, member.peer_id);
  const manualVal =
    state.draftManualAddrs[member.peer_id] ?? rec?.manual_addr ?? rec?.last_addr ?? "";

  return `<details class="fs-peer-details peer-sync-details">
    <summary class="fs-peer-summary">Veza i ručna adresa${rec?.last_addr ? ` · ${esc(rec.last_addr)}` : ""}</summary>
    <div class="fs-peer-controls peer-sync-controls">
      <div class="fs-member-meta">Zadnja poznata adresa: ${esc(rec?.last_addr ?? "—")}</div>
      <div class="fs-inline-form">
        <input class="fs-input inp-manual-addr" data-manual-addr-input="${esc(member.peer_id)}" value="${esc(manualVal)}" placeholder="IP:port (npr. 192.168.1.25:4411)" />
        <button class="btn btn-outline" type="button" data-save-manual-addr="${esc(member.peer_id)}" ${state.lanBusy ? "disabled" : ""}>Spremi adresu</button>
        <button class="btn btn-primary" type="button" data-sync-peer-at="${esc(member.peer_id)}" ${state.lanBusy ? "disabled" : ""}>Razmijeni sada</button>
      </div>
    </div>
  </details>`;
}

function renderMemberRow(
  member: NetworkMember,
  state: FamilyViewState,
  helpers: ViewHelpers,
): string {
  const esc = helpers.escapeHtml;
  const rec = findPeerRecord(state, member.peer_id);
  const status = formatPeerConfirmation(rec, helpers);
  const sharedNames =
    member.role === "family_member"
      ? helpers.sharedListNames(member.shared_lists, [...state.lists])
      : [];

  return `<div class="fs-member-item">
    <div class="fs-member-top">
      <div class="fs-member-copy">
        <div class="fs-member-name">${esc(member.name)}</div>
        <div class="fs-member-meta">${esc(status.detailLine)}</div>
        ${rec?.last_error ? `<div class="fs-member-error">Pogreška: ${esc(rec.last_error)}</div>` : ""}
        ${member.role === "family_member"
          ? `<div class="fs-pill-list">${sharedNames.length ? sharedNames.map((n) => `<span class="fs-pill">${esc(n)}</span>`).join("") : '<span class="fs-member-meta">Nema odabranih popisa</span>'}</div>`
          : `<div class="fs-member-meta">Dodan ${esc(helpers.formatDate(member.added_at))} · svi osobni podaci</div>`}
      </div>
      <div class="fs-actions-row">
        <span class="fs-badge ${status.badgeClass}">${esc(status.badgeText)}</span>
        <button class="btn btn-outline" type="button" data-member-access="${esc(member.peer_id)}">Pregled pristupa</button>
      </div>
    </div>
    ${renderPeerControls(member, state, helpers)}
  </div>`;
}

function renderStatusBanner(state: FamilyViewState, helpers: ViewHelpers): string {
  const esc = helpers.escapeHtml;
  const progress = state.progress;
  const lan = state.lanStatus;

  if (progress?.state === "running") {
    return `<section class="fs-banner fs-banner--info" role="status" aria-live="polite">
      <div class="fs-banner-title">Razmjena je u tijeku</div>
      <p class="fs-banner-text">${esc(progress.title)} · lokalno spremanje ostaje zaštićeno dok razmjena traje.</p>
    </section>`;
  }

  if (progress?.state === "error" || lan?.last_error) {
    const errText = progress?.state === "error" ? progress.detail : lan?.last_error ?? "";
    return `<section class="fs-banner fs-banner--warning" role="status">
      <div class="fs-banner-title">Spremljeno na ovom uređaju · zadnja razmjena nije dovršena</div>
      <p class="fs-banner-text">${esc(errText || "Provjerite je li drugi uređaj uključen i na istoj mreži.")}</p>
    </section>`;
  }

  if (lan?.running && lan.paused) {
    return `<section class="fs-banner fs-banner--warning" role="status">
      <div class="fs-banner-title">Spremljeno na ovom uređaju · Automatsko pokretanje razmjene je pauzirano</div>
      <p class="fs-banner-text">Ručna razmjena, dolazne veze i prijenosi koji već traju ostaju mogući. Za prekid LAN razmjene odaberite „Zaustavi LAN”.</p>
    </section>`;
  }

  if (lan?.running) {
    const detail =
      lan.active_conversations > 0
        ? `Aktivna izravna razmjena (${lan.active_conversations}).`
        : lan.last_round_at
          ? `LAN razmjena aktivna · zadnji krug ${helpers.formatDate(lan.last_round_at)}. Potvrda primitka prikazuje se zasebno po uređaju.`
          : "LAN razmjena aktivna · lokalno spremanje nije potvrda udaljenog primitka dok uređaj ne potvrdi razmjenu.";
    return `<section class="fs-banner fs-banner--success" role="status">
      <div class="fs-banner-title">Spremljeno na ovom uređaju</div>
      <p class="fs-banner-text">${esc(detail)}</p>
    </section>`;
  }

  return `<section class="fs-banner fs-banner--success" role="status">
    <div class="fs-banner-title">Spremljeno na ovom uređaju</div>
    <p class="fs-banner-text">Lokalno spremanje nije potvrda primitka na drugim uređajima. Pokrenite LAN razmjenu ili razmijenite datotekom.</p>
  </section>`;
}

export function renderFamily(state: FamilyViewState, helpers: ViewHelpers): string {
  const esc = helpers.escapeHtml;
  const networkName = state.ledgerStats?.network_name?.trim() || "Obiteljska mreža";
  const ownDevices = state.members.filter((m) => m.role === "own_device");
  const familyMembers = state.members.filter((m) => m.role === "family_member");
  const lan = state.lanStatus;

  return `<section class="fs-page" aria-labelledby="fs-family-heading">
    <header class="fs-header">
      <div class="fs-header-copy">
        <h1 class="fs-title" id="fs-family-heading">Obitelj i uređaji</h1>
        <p class="fs-subtitle">Mreža: ${esc(networkName)} · Vi birate s kim dijelite popise.</p>
      </div>
      <button class="fs-info-btn" type="button" data-goto="about" aria-label="O aplikaciji" title="O aplikaciji">i</button>
    </header>

    ${renderStatusBanner(state, helpers)}

    <div class="fs-actions-grid">
      <button class="btn btn-primary fs-btn-full" type="button" id="btn-open-pairing" data-fs-pair-intent="invite">Pozovi člana ili uređaj</button>
      <button class="btn btn-outline fs-btn-full" type="button" data-fs-open-pairing="join">Pridruži se drugoj mreži</button>
    </div>

    <div class="fs-actions-row">
      ${lan?.running
        ? `<button class="btn btn-outline" type="button" id="btn-lan-sync-now" ${state.lanBusy || lan.paused ? "disabled" : ""}>${state.lanBusy ? "Razmjena…" : "Pokušaj ponovno / Sinkroniziraj"}</button>
           <button class="btn btn-outline" type="button" id="btn-lan-pause" ${state.lanBusy ? "disabled" : ""}>${lan.paused ? "Nastavi automatsku razmjenu" : "Pauziraj automatsku razmjenu"}</button>
           <button class="btn btn-subtle" type="button" id="btn-lan-stop" ${state.lanBusy ? "disabled" : ""}>Zaustavi LAN</button>`
        : `<button class="btn btn-outline" type="button" id="btn-lan-start" ${state.lanBusy ? "disabled" : ""}>Pokreni LAN razmjenu</button>`}
    </div>

    <section class="fs-card" aria-labelledby="fs-own-devices-heading">
      <div class="fs-card-header">
        <div>
          <h2 class="fs-card-title" id="fs-own-devices-heading">Moji uređaji · moji računi, katalog i popisi</h2>
          <p class="fs-card-sub">Pristup svim osobnim podacima i popisima na vašim uređajima.</p>
        </div>
      </div>
      <div class="fs-member-list">
        <div class="fs-member-item">
          <div class="fs-member-top">
            <div class="fs-member-copy">
              <div class="fs-member-name">Ovaj uređaj</div>
              <div class="fs-member-meta">Lokalno spremljeno${lan?.exchange_addr ? ` · sluša na ${esc(lan.exchange_addr)}` : ""}</div>
            </div>
            <span class="fs-badge fs-badge--neutral">Ovaj uređaj</span>
          </div>
        </div>
        ${ownDevices.map((m) => renderMemberRow(m, state, helpers)).join("")}
      </div>
    </section>

    <section class="fs-card" aria-labelledby="fs-family-members-heading">
      <div class="fs-card-header">
        <div>
          <h2 class="fs-card-title" id="fs-family-members-heading">Članovi obitelji · odabrani popisi</h2>
          <p class="fs-card-sub">Dijele se samo odabrani popisi i njihovi artikli. Vaši osobni računi ostaju privatni.</p>
        </div>
      </div>
      ${familyMembers.length
        ? `<div class="fs-member-list">${familyMembers.map((m) => renderMemberRow(m, state, helpers)).join("")}</div>`
        : '<div class="fs-empty">Još nema dodanih članova obitelji.</div>'}
    </section>

    <section class="fs-card" aria-labelledby="fs-file-exchange-heading">
      <h2 class="fs-card-title" id="fs-file-exchange-heading">Razmjena datotekom</h2>
      <p class="fs-card-sub">Izvezite paket i otvorite ga na drugom uređaju kad niste na istoj mreži. Izvoz datoteke ne potvrđuje primitak na drugom uređaju.</p>
      <div class="fs-actions-row">
        <button class="btn btn-outline" type="button" id="btn-export-bundle">Izvezi za moj uređaj</button>
        <label class="btn btn-primary file-button">Uvezi paket<input id="inp-import-bundle" type="file" accept=".json,.list2cart" /></label>
      </div>
      <details class="fs-peer-details">
        <summary class="fs-peer-summary">Pošalji odabrane popise obitelji datotekom</summary>
        <div class="fs-peer-controls">
          <div class="list-choices">
            ${state.lists.length
              ? state.lists
                  .map(
                    (l) =>
                      `<label class="check-option"><input type="checkbox" data-export-list="${esc(l.sync_id)}" ${state.exportListIds.includes(l.sync_id) ? "checked" : ""} /><span>${esc(l.name)}</span></label>`,
                  )
                  .join("")
              : '<p class="fs-card-sub">Najprije kreirajte popis za kupnju.</p>'}
          </div>
          <button class="btn btn-outline" type="button" id="btn-export-family-bundle" ${state.exportListIds.length ? "" : "disabled"}>Izvezi odabrane popise</button>
        </div>
      </details>
    </section>

    <footer class="fs-footer-bar">
      <span class="fs-footnote">Vaši osobni računi ne dijele se s obitelji.</span>
      <div class="fs-actions-row">
        <button class="btn btn-outline" type="button" data-goto="napredno">Napredne postavke</button>
        <button class="btn btn-outline" type="button" data-goto="about">O aplikaciji</button>
      </div>
    </footer>
  </section>`;
}

export function renderMemberDialog(state: FamilyViewState, helpers: ViewHelpers): string {
  const esc = helpers.escapeHtml;
  const member = state.removalMember ?? state.accessMember;
  if (!member) return "";

  if (state.removalMember) {
    return `<div class="modal-backdrop">
      <section class="modal-card fs-modal-card" role="dialog" aria-modal="true" aria-labelledby="member-title">
        <div class="fs-modal-header">
          <div>
            <h2 class="fs-modal-title" id="member-title">Ukloni iz evidencije?</h2>
            <p class="fs-modal-sub">${esc(member.name)}</p>
          </div>
          <button class="icon-button" type="button" data-close-dialog aria-label="Odustani">×</button>
        </div>
        <p class="fs-card-sub">Ovaj se uređaj ili član uklanja iz lokalne evidencije. Već poslane datoteke i primljene kopije ostaju na uređaju primatelja.</p>
        <div class="fs-dialog-actions">
          <button class="btn btn-danger fs-btn-full" type="button" data-remove-member="${esc(member.peer_id)}">Ukloni iz evidencije</button>
          <button class="btn btn-outline fs-btn-full" type="button" data-close-dialog>Odustani</button>
        </div>
      </section>
    </div>`;
  }

  const rec = findPeerRecord(state, member.peer_id);
  const status = formatPeerConfirmation(rec, helpers);
  const isOwn = member.role === "own_device";
  const sharedNames = isOwn
    ? []
    : helpers.sharedListNames(member.shared_lists, [...state.lists]);

  return `<div class="modal-backdrop">
    <section class="modal-card fs-modal-card" role="dialog" aria-modal="true" aria-labelledby="member-title">
      <div class="fs-modal-header">
        <div>
          <h2 class="fs-modal-title" id="member-title">${isOwn ? "Moj uređaj" : "Član obitelji"}</h2>
          <p class="fs-modal-sub">${esc(member.name)} · dodan ${esc(helpers.formatDate(member.added_at))}</p>
        </div>
        <button class="icon-button" type="button" data-close-dialog aria-label="Zatvori pregled pristupa">×</button>
      </div>

      <div class="fs-section-box">
        <div class="fs-card-header">
          <span class="fs-member-name">Status razmjene</span>
          <span class="fs-badge ${status.badgeClass}">${esc(status.badgeText)}</span>
        </div>
        <p class="fs-member-meta">${esc(status.detailLine)}</p>
        ${rec?.last_error ? `<p class="fs-member-error">Zadnja pogreška: ${esc(rec.last_error)}</p>` : ""}
      </div>

      <div class="fs-section-box">
        <div class="fs-member-name">Što dijelite</div>
        <div class="fs-access-table">
          ${isOwn
            ? `<div class="fs-access-row"><span class="fs-access-key">Računi, katalog i svi popisi</span><span class="fs-access-val">Puni pristup</span></div>`
            : `${sharedNames.length
                ? sharedNames
                    .map(
                      (name) =>
                        `<div class="fs-access-row"><span class="fs-access-key">${esc(name)}</span><span class="fs-access-val">Podijeljen popis</span></div>`,
                    )
                    .join("")
                : `<div class="fs-access-row"><span class="fs-access-key">Popisi</span><span class="fs-access-val">Nema podijeljenih popisa</span></div>`}
               <div class="fs-access-row"><span class="fs-access-key">Osobni računi</span><span class="fs-access-val">Ne dijele se</span></div>`}
        </div>
        <p class="fs-footnote">${isOwn
          ? "Ovaj uređaj prima i osobne račune, katalog i sve popise."
          : "Naknadna izmjena pojedinačnih prava po popisu trenutačno nije dostupna; pristup se određuje pri povezivanju uređaja."}</p>
      </div>

      <div class="fs-dialog-actions">
        <button class="btn btn-primary fs-btn-full" type="button" data-close-dialog>Zatvori</button>
        <button class="btn btn-danger-ghost fs-btn-full" type="button" id="btn-review-remove-member">Ukloni iz evidencije</button>
      </div>
    </section>
  </div>`;
}
