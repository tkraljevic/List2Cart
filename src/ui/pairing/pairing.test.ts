// @ts-expect-error bun:test is provided by the Bun test runner
import { describe, expect, test } from "bun:test";
import type { PairingViewState, ViewHelpers } from "../contracts";
import { renderPairingDialog } from "./index";

const helpers: ViewHelpers = {
  escapeHtml: (v) =>
    String(v ?? "")
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#39;"),
  formatDate: (v) => v,
  sharedListNames: () => [],
};

function basePairing(overrides: Partial<PairingViewState> = {}): PairingViewState {
  return {
    pairingOpen: true,
    pairingRole: "family_member",
    pairingAdmit: true,
    pairingConfirmed: false,
    pairingBusy: false,
    pairMode: "lan",
    pairStatus: {
      waiting: false,
      pending: true,
      sas: "482910",
      peer_name: "Ana <img src=x>",
      peer_host: "192.168.1.20:51234",
      peer_public_key: "AbCdEfGhIjKlMnOpQrStUvWxYz",
      can_admit: true,
    },
    pendingSasCode: "482-910",
    draftPairAddr: "",
    draftDeviceName: "MacBook",
    draftMemberName: "",
    draftMemberLists: "list-1",
    lists: [
      {
        id: 1,
        sync_id: "list-1",
        name: "Testni popis",
        items: [],
        total_estimated_cents: 0,
        updated_at: "2026-11-19T07:00:00Z",
      },
    ],
    ledgerStats: {
      node_id: "node-1",
      network_name: "Obitelj Kraljević",
      total_versions: 1,
      unique_records: 1,
      tombstones: 0,
      frontier_nodes: 1,
      db_size_bytes: 1024,
    },
    lanPeersDiscovered: [],
    ...overrides,
  };
}

describe("renderPairingDialog", () => {
  test("returns empty string when closed", () => {
    expect(renderPairingDialog(basePairing({ pairingOpen: false }), helpers)).toBe("");
  });

  test("disables confirm until SAS checkbox is confirmed and escapes peer name", () => {
    const html = renderPairingDialog(basePairing(), helpers);

    expect(html).toContain("482 910");
    expect(html).toContain("Ana &lt;img src=x&gt;");
    expect(html).not.toContain("<img src=x>");
    expect(html).toContain('id="btn-confirm-live-pairing" disabled');
    expect(html).toContain("Prvo usporedite i označite kod");
    expect(html).toContain('id="btn-reject-live-pairing"');
  });

  test("confirmation in flight remains cancellable without claiming backend acknowledgement", () => {
    const html = renderPairingDialog(
      basePairing({ pairingConfirmed: true, pairingBusy: true, pairingAction: "confirming" }),
      helpers,
    );

    expect(html).toContain("Obrada povezivanja");
    expect(html).not.toContain("Vaša potvrda je zaprimljena");
    expect(html).toContain('id="btn-reject-live-pairing" >');
    expect(html).toContain('aria-label="Odustani od povezivanja" >');
  });

  test("shows error banner when pairing fails without sharing data", () => {
    const html = renderPairingDialog(
      basePairing({
        pairStatus: {
          waiting: false,
          pending: false,
          error: "Kodovi se razlikuju",
          can_admit: true,
        },
      }),
      helpers,
    );

    expect(html).toContain("Podaci nisu podijeljeni");
    expect(html).toContain("Kodovi se razlikuju");
    expect(html).toContain('id="btn-start-pair-listen"');
    expect(html).toContain('id="btn-discover-pair-peers"');
  });
});

test("cancellation never claims confirmation or allows a duplicate cancel", () => {
  const html = renderPairingDialog(basePairing({pairingBusy:true, pairingAction:"cancelling"}),helpers);
  expect(html).toContain("Prekidanje povezivanja");
  expect(html).not.toContain("Vaša potvrda je zaprimljena");
  expect(html).not.toContain("Čeka se potvrda drugog uređaja");
  expect(html).toContain('id="btn-reject-live-pairing" disabled');
});
