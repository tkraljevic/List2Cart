/** Shared integration boundary. Owned by the main agent; renderers never mutate state. */
import type { FoundLanPeer, IdentityConflict, LanServiceStatus, PairStatusView } from "../api";
import type { LedgerStats, NetworkMember, PeerRole, ShoppingList } from "../types";
import type { SyncProgress } from "../sync-progress";

export type AppTab = "pregled" | "kosarica" | "planer" | "racuni" | "katalog" | "cijene" | "trgovine" | "sync" | "napredno" | "about";

export interface FamilyViewState {
  members: readonly NetworkMember[];
  lists: readonly ShoppingList[];
  ledgerStats: LedgerStats | null;
  lanStatus: LanServiceStatus | null;
  lanBusy: boolean;
  draftManualAddrs: Readonly<Record<string, string>>;
  exportListIds: readonly string[];
  accessMember: NetworkMember | null;
  removalMember: NetworkMember | null;
  progress: SyncProgress | null;
}

export interface PairingViewState {
  pairingOpen: boolean;
  pairingRole: PeerRole;
  pairingAdmit: boolean;
  pairingConfirmed: boolean;
  pairingBusy: boolean;
  pairingAction?: "confirming" | "cancelling" | null;
  pairMode: "lan" | "offline";
  pairStatus: PairStatusView | null;
  pendingSasCode: string;
  draftPairAddr: string;
  draftDeviceName: string;
  draftMemberName: string;
  draftMemberLists: string | null;
  lists: readonly ShoppingList[];
  ledgerStats: LedgerStats | null;
  lanPeersDiscovered: readonly FoundLanPeer[];
}

export interface ConflictViewState {
  conflicts: readonly IdentityConflict[];
  conflictReview: { group: IdentityConflict; keepId: string } | null;
  resolvingConflict: boolean;
}

export interface AboutViewState {
  name: string;
  version: string;
  license: string;
  /** Trusted local asset URL, supplied by the host, never from peer data. */
  iconUrl: string;
  /** Where to support the app's development; null where a donation link is not shown. */
  supportUrl?: string | null;
}

export interface ViewHelpers {
  escapeHtml(value: unknown): string;
  formatDate(value: string): string;
  sharedListNames(shared: string, lists: ShoppingList[]): string[];
}

/** New controls may emit these actions; existing control IDs remain supported. */
export type UiAction =
  | { type: "navigate"; tab: AppTab }
  | { type: "close-dialog" }
  | { type: "open-pairing" }
  | { type: "member-access"; peerId: string }
  | { type: "review-remove-member"; peerId: string }
  | { type: "remove-member"; peerId: string }
  | { type: "lan-start" | "lan-stop" | "lan-sync-now" | "lan-toggle-pause" }
  | { type: "save-manual-address" | "sync-peer"; peerId: string; address: string }
  | { type: "export-package"; role: PeerRole; listIds: readonly string[] }
  | { type: "import-package"; file: File };

export interface ViewActions {
  dispatch(action: UiAction): void | Promise<void>;
  requestRender(): void;
}
