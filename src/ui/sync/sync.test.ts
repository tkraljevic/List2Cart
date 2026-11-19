// @ts-expect-error bun:test is provided by the Bun test runner
import { describe, expect, test } from "bun:test";
import type { ConflictViewState, FamilyViewState, ViewHelpers } from "../contracts";
import { renderIdentityConflicts, renderSyncOverview, renderSyncProgress } from "./index";

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

describe("sync UI renderers", () => {
  test("renderSyncProgress renders real steps and committed notice", () => {
    const html = renderSyncProgress(
      {
        title: "Uvoz paketa",
        steps: ["Čitanje datoteke", "Spremanje u bazu", "Osvježavanje prikaza"],
        current: 2,
        completed: 2,
        state: "running",
        committed: true,
        detail: "",
        updatedAt: Date.now(),
      },
      helpers,
    );

    expect(html).toContain("Uvoz paketa");
    expect(html).toContain("Čitanje datoteke");
    expect(html).toContain("Lokalni podaci su spremljeni.");
  });

  test("renderSyncOverview shows confirmed receipt only when last_sync_at has no error", () => {
    const state: FamilyViewState = {
      members: [
        {
          peer_id: "p1",
          name: "MacBook",
          role: "own_device",
          sas_code: "111222",
          channels: "osobno,obitelj",
          shared_lists: "*",
          added_at: "2026-11-19T07:00:00Z",
        },
        {
          peer_id: "p2",
          name: "Ana",
          role: "family_member",
          sas_code: "333444",
          channels: "obitelj",
          shared_lists: "l1",
          added_at: "2026-11-19T07:00:00Z",
        },
      ],
      lists: [],
      ledgerStats: null,
      lanStatus: {
        running: true,
        paused: false,
        exchange_addr: "0.0.0.0:1234",
        device_public_key: "self",
        active_conversations: 0,
        peers: [
          {
            peer_public_key: "p1",
            peer_name: "MacBook",
            last_sync_at: "2026-11-19T07:32:00Z",
            last_applied: 1,
            last_sent: 2,
          },
        ],
      },
      lanBusy: false,
      draftManualAddrs: {},
      exportListIds: [],
      accessMember: null,
      removalMember: null,
      progress: null,
    };

    const html = renderSyncOverview(state, helpers);
    expect(html).toContain("Potvrđen primitak");
    expect(html).toContain("Čeka potvrdu");
    expect(html).toContain("Ovaj uređaj još nije potvrdio primitak promjena.");
  });

  test("renderIdentityConflicts preserves both records and never claims field/history merge", () => {
    const state: ConflictViewState = {
      conflicts: [
        {
          entity: "products",
          value: "3850108012345",
          sync_ids: ["id-a", "id-b"],
        },
      ],
      conflictReview: {
        group: {
          entity: "products",
          value: "3850108012345",
          sync_ids: ["id-a", "id-b"],
        },
        keepId: "id-a",
      },
      resolvingConflict: false,
    };

    const html = renderIdentityConflicts(state, helpers, (_g, id) => `Mlijeko (${id})`);
    expect(html).toContain("polja i povijest se ne spajaju");
    expect(html).toContain('id="confirm-conflict"');
    expect(html).toContain('id="cancel-conflict"');
    expect(html).not.toContain("Pregledaj spajanje");
  });
});
