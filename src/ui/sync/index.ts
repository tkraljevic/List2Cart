import type { IdentityConflict, PeerSyncRecord } from "../../api";
import type { SyncProgress } from "../../sync-progress";
import type { ConflictViewState, FamilyViewState, ViewHelpers } from "../contracts";

export function renderSyncProgress(
  progress: SyncProgress | null,
  helpers: ViewHelpers,
): string {
  if (!progress) return "";
  const esc = helpers.escapeHtml;
  const bannerClass =
    progress.state === "running"
      ? "fs-banner--info"
      : progress.state === "done"
        ? "fs-banner--success"
        : "fs-banner--error";
  const stateLabel =
    progress.state === "running"
      ? "U tijeku"
      : progress.state === "done"
        ? "Dovršeno"
        : "Zaustavljeno greškom";

  return `<section class="fs-card" aria-busy="${progress.state === "running"}" aria-live="polite">
    <div class="fs-banner ${bannerClass}">
      <div class="fs-banner-title">${esc(progress.title)} · ${esc(stateLabel)}</div>
      ${progress.detail ? `<p class="fs-banner-text">${esc(progress.detail)}</p>` : ""}
    </div>

    <ol class="fs-step-list">
      ${progress.steps
        .map((label, index) => {
          const isDone = index < progress.completed;
          const isCurrent = index === progress.current && !isDone;
          const isError = isCurrent && progress.state === "error";
          const itemClass = isDone
            ? "fs-step-item--done"
            : isError
              ? "fs-step-item--error"
              : isCurrent
                ? "fs-step-item--active"
                : "";
          const marker = isDone ? "✓" : isError ? "✕" : isCurrent ? "●" : "○";
          return `<li class="fs-step-item ${itemClass}">
            <span class="fs-step-marker" aria-hidden="true">${marker}</span>
            <span>${esc(label)}${isCurrent && progress.state === "running" ? " — u tijeku" : ""}</span>
          </li>`;
        })
        .join("")}
    </ol>

    <div class="fs-footnote">
      ${progress.committed ? "Lokalni podaci su spremljeni. " : ""}Posljednja promjena: ${esc(new Date(progress.updatedAt).toLocaleTimeString("hr-HR"))}
    </div>
  </section>`;
}

function peerRecordFor(state: FamilyViewState, peerId: string): PeerSyncRecord | undefined {
  return state.lanStatus?.peers.find((p) => p.peer_public_key === peerId);
}

/**
 * Dedicated per-device sync overview built strictly on existing FamilyViewState.
 * Never fabricates per-peer pending counts, progress percentages, or unconfirmed receipts.
 */
export function renderSyncOverview(state: FamilyViewState, helpers: ViewHelpers): string {
  const esc = helpers.escapeHtml;
  const networkName = state.ledgerStats?.network_name?.trim() || "Obiteljska mreža";
  const lan = state.lanStatus;

  const peerCards = state.members.map((member) => {
    const rec = peerRecordFor(state, member.peer_id);
    const hasConfirmed = Boolean(rec?.last_sync_at && !rec?.last_error);
    const hasError = Boolean(rec?.last_error);
    const badgeClass = hasError
      ? "fs-badge--error"
      : hasConfirmed
        ? "fs-badge--success"
        : "fs-badge--warning";
    const badgeText = hasError
      ? "Prekid ili greška"
      : hasConfirmed
        ? "Potvrđen primitak"
        : "Čeka potvrdu";

    return `<div class="fs-member-item">
      <div class="fs-member-top">
        <div class="fs-member-copy">
          <div class="fs-member-name">${esc(member.name)}</div>
          <div class="fs-member-meta">${rec?.last_sync_at
            ? `Zadnja potvrđena razmjena: ${esc(helpers.formatDate(rec.last_sync_at))} · primljeno ${rec.last_applied}, poslano ${rec.last_sent}`
            : "Ovaj uređaj još nije potvrdio primitak promjena."}</div>
          ${rec?.last_error ? `<div class="fs-member-error">Pogreška: ${esc(rec.last_error)}</div>` : ""}
        </div>
        <div class="fs-actions-row">
          <span class="fs-badge ${badgeClass}">${esc(badgeText)}</span>
          ${rec?.manual_addr || rec?.last_addr
            ? `<button class="btn btn-outline" type="button" data-sync-peer-at="${esc(member.peer_id)}" ${state.lanBusy ? "disabled" : ""}>Pokušaj ponovno</button>`
            : ""}
        </div>
      </div>
    </div>`;
  });

  return `<section class="fs-page" aria-labelledby="fs-sync-heading">
    <header class="fs-header">
      <div class="fs-header-copy">
        <h1 class="fs-title" id="fs-sync-heading">Sinkronizacija</h1>
        <p class="fs-subtitle">${esc(networkName)} · Izravna razmjena između odabranih uređaja</p>
      </div>
      <button class="btn btn-outline" type="button" data-goto="sync">← Obitelj</button>
    </header>

    ${renderSyncProgress(state.progress, helpers)}

    <section class="fs-card">
      <div class="fs-card-header">
        <h2 class="fs-card-title">Status po uređaju</h2>
        <span class="fs-badge ${lan?.running && !lan.paused ? "fs-badge--success" : "fs-badge--warning"}">${!lan?.running
          ? "LAN zaustavljen"
          : lan.paused
            ? "Automatsko pokretanje pauzirano"
            : lan.active_conversations > 0
              ? `Aktivno (${lan.active_conversations})`
              : "LAN aktivan"}</span>
      </div>
      ${peerCards.length
        ? `<div class="fs-member-list">${peerCards.join("")}</div>`
        : '<div class="fs-empty">Još nema povezanih uređaja za razmjenu.</div>'}
      <p class="fs-footnote">Podaci su spremljeni lokalno. Potvrda primitka vrijedi samo za navedeni uređaj kada razmjena uspješno završi.</p>
    </section>
  </section>`;
}

export function renderIdentityConflicts(
  state: ConflictViewState,
  helpers: ViewHelpers,
  describeItem: (group: IdentityConflict, id: string) => string,
): string {
  const esc = helpers.escapeHtml;
  const review = state.conflictReview;

  return `<section class="fs-card" aria-label="Sukobi i duplikati zapisa">
    <div class="fs-card-header">
      <h2 class="fs-card-title">Duplikati zapisa${state.conflicts.length ? ` (${state.conflicts.length})` : ""}</h2>
      <button class="btn btn-outline" type="button" id="refresh-conflicts" ${state.resolvingConflict ? "disabled" : ""}>Osvježi</button>
    </div>

    ${review
      ? `<div class="fs-banner fs-banner--warning" role="status">
           <div class="fs-banner-title">Potvrdite odabir aktivnog zapisa</div>
           <p class="fs-banner-text">Odabirom aktivnog zapisa duplikati se arhiviraju. Izvorni zapisi, povijest cijena i računi ostaju sačuvani — polja i povijest se ne spajaju niti prepisuju.</p>
         </div>
         <div class="fs-section-box">
           <div class="fs-member-name">Aktivan ostaje: ${esc(describeItem(review.group, review.keepId))}</div>
           <div class="fs-member-meta">Arhivirat će se duplikati:</div>
           <ul class="fs-step-list">
             ${review.group.sync_ids
               .filter((id) => id !== review.keepId)
               .map(
                 (id) =>
                   `<li class="fs-member-meta">• ${esc(describeItem(review.group, id))} <span class="fs-conflict-id">${esc(id)}</span></li>`,
               )
               .join("")}
           </ul>
         </div>
         <div class="fs-actions-row">
           <button class="btn btn-primary" type="button" id="confirm-conflict" ${state.resolvingConflict ? "disabled" : ""}>${state.resolvingConflict ? "Spremanje…" : "Potvrdi i arhiviraj duplikate"}</button>
           <button class="btn btn-outline" type="button" id="cancel-conflict" ${state.resolvingConflict ? "disabled" : ""}>Odustani (sačuvaj oba)</button>
         </div>`
      : state.conflicts.length
        ? `<div class="fs-banner fs-banner--warning" role="status">
             <div class="fs-banner-title">Potrebna je vaša odluka</div>
             <p class="fs-banner-text">Oba zapisa su sačuvana i ništa nije prepisano. Možete ih ostaviti oba ili odabrati jedan aktivan zapis dok se ostali arhiviraju (bez spajanja polja ili povijesti).</p>
           </div>
           ${state.conflicts
             .map(
               (group, index) => `<div class="fs-conflict-group">
                 <div class="fs-member-name">${group.entity === "stores" ? "Trgovine istog naziva" : "Artikli istog barkoda"}: ${esc(group.value)}</div>
                 ${group.sync_ids
                   .map(
                     (id) => `<div class="fs-conflict-option">
                       <div class="fs-member-copy">
                         <div class="fs-member-name">${esc(describeItem(group, id))}</div>
                         <div class="fs-conflict-id">${esc(id)}</div>
                       </div>
                       <button class="btn btn-outline" type="button" data-conflict-group="${index}" data-conflict-keep="${esc(id)}">Zadrži ovaj zapis</button>
                     </div>`,
                   )
                   .join("")}
               </div>`,
             )
             .join("")}`
        : '<p class="fs-card-sub">Nema nerazriješenih duplikata zapisa.</p>'}
  </section>`;
}
