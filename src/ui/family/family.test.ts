// @ts-expect-error bun:test is provided by the Bun test runner
import { describe, expect, test } from "bun:test";
import type { FamilyViewState, ViewHelpers } from "../contracts";
import { renderFamily, renderMemberDialog } from "./index";

const helpers: ViewHelpers = {
  escapeHtml: (v) =>
    String(v ?? "")
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#39;"),
  formatDate: (v) => `DATUM(${v})`,
  sharedListNames: (shared, lists) => {
    const ids = shared.split(",").filter(Boolean);
    return lists.filter((l) => ids.includes(l.sync_id)).map((l) => l.name);
  },
};

function baseState(overrides: Partial<FamilyViewState> = {}): FamilyViewState {
  return {
    members: [
      {
        peer_id: "peer-own",
        name: "Tomislav iPhone",
        role: "own_device",
        sas_code: "123456",
        channels: "osobno,obitelj",
        shared_lists: "*",
        added_at: "2026-11-19T07:00:00Z",
      },
      {
        peer_id: "peer-fam",
        name: "Ana <b>iPhone</b>",
        role: "family_member",
        sas_code: "654321",
        channels: "obitelj",
        shared_lists: "list-1",
        added_at: "2026-11-19T07:10:00Z",
      },
    ],
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
      total_versions: 10,
      unique_records: 4,
      tombstones: 0,
      frontier_nodes: 2,
      db_size_bytes: 16384,
    },
    lanStatus: {
      running: true,
      paused: false,
      exchange_addr: "0.0.0.0:63163",
      device_public_key: "dev-key",
      active_conversations: 0,
      last_round_at: "2026-11-19T07:32:00Z",
      peers: [
        {
          peer_public_key: "peer-own",
          peer_name: "Tomislav iPhone",
          last_addr: "192.168.1.10:63163",
          last_sync_at: "2026-11-19T07:32:00Z",
          last_path: "lan",
          last_applied: 2,
          last_sent: 3,
        },
        {
          peer_public_key: "peer-fam",
          peer_name: "Ana",
          last_error: "Veza odbijena",
          last_applied: 0,
          last_sent: 0,
        },
      ],
    },
    lanBusy: false,
    draftManualAddrs: {},
    exportListIds: ["list-1"],
    accessMember: null,
    removalMember: null,
    progress: null,
    ...overrides,
  };
}

describe("renderFamily & renderMemberDialog", () => {
  test("renders own devices, family members, real peer statuses, and preserves existing IDs", () => {
    const html = renderFamily(baseState(), helpers);

    expect(html).toContain("Obitelj Kraljević");
    expect(html).toContain("Spremljeno na ovom uređaju");
    expect(html).toContain('id="btn-open-pairing"');
    expect(html).toContain('id="btn-lan-sync-now"');
    expect(html).toContain('id="btn-lan-pause"');
    expect(html).toContain('id="btn-lan-stop"');
    expect(html).toContain('id="btn-export-bundle"');
    expect(html).toContain('id="inp-import-bundle"');
    expect(html).toContain('id="btn-export-family-bundle"');
    expect(html).toContain('data-goto="about"');
    expect(html).toContain('data-goto="napredno"');
    expect(html).toContain("Ana &lt;b&gt;iPhone&lt;/b&gt;");
    expect(html).not.toContain("<b>iPhone</b>");
    expect(html).toContain("Testni popis");
    expect(html).toContain("Veza odbijena");
    expect(html).toContain("Potvrđeno");
    expect(html).toContain("Veza nije uspjela");
  });

  test("member dialog shows read-only shared lists without fake permission editors", () => {
    const st = baseState();
    st.accessMember = st.members[1];
    const html = renderMemberDialog(st, helpers);

    expect(html).toContain("Član obitelji");
    expect(html).toContain("Testni popis");
    expect(html).toContain("Osobni računi");
    expect(html).toContain("Ne dijele se");
    expect(html).toContain('id="btn-review-remove-member"');
    expect(html).not.toContain("Spremi pristup");
  });

  test("removal dialog renders confirmation with remove button", () => {
    const st = baseState();
    st.removalMember = st.members[1];
    const html = renderMemberDialog(st, helpers);

    expect(html).toContain("Ukloni iz evidencije?");
    expect(html).toContain('data-remove-member="peer-fam"');
  });
});

test("pause describes automatic scheduling and keeps explicit stop available", () => {
  const state = baseState();
  state.lanStatus = {...state.lanStatus!,paused:true};
  const html = renderFamily(state, helpers);
  expect(html).toContain("Automatsko pokretanje razmjene je pauzirano");
  expect(html).toContain("Ručna razmjena, dolazne veze i prijenosi koji već traju ostaju mogući");
  expect(html).not.toContain("ne šalju se dok ne nastavite");
  expect(html).toContain('id="btn-lan-stop" >');
});
