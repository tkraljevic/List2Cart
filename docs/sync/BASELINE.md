# List2Cart Sync — Baseline (`S00`)

Datum snimanja: 2026-11-17 15:28 CET

## 1. Git i okruženje

- **List2Cart HEAD**: `00f74ecef1b80dff03c35bebacb05387f08bec3a` (`Izravna zamjena shellScript u project.pbxproj za iOS gradnju`)
- **Zatečene lokalne izmjene (sačuvane bez resetiranja)**:
  - `M README.md`
  - `M crates/list2cart-core/src/calc.rs`
  - `M crates/list2cart-core/src/db.rs`
  - `M crates/list2cart-core/src/ledger.rs`
  - `M crates/list2cart-core/src/models.rs`
  - `M src-tauri/src/lib.rs`
  - `M src-tauri/tauri.conf.json`
  - `M src/api.ts`
  - `M src/main.ts`
  - `?? docs/`
- **Referentni goCOP HEAD**: `fcca2c6bda5a2c43fa8b051994d4a5ad1a819e7e`
- **OS i arhitektura**: macOS `26.6.2` (`BuildVersion: 25G83`), `Darwin 25.6.0 arm64`
- **Toolchain**:
  - `rustc 1.99.1 (58cf8f9c3 2026-10-13)`
  - `cargo 1.99.1 (5f94df478 2026-08-27)`
  - `bun 1.3.14`
  - Instalirani Rust target: `aarch64-apple-darwin` (Android/iOS targeti nisu trenutno dodani u `rustup`)
  - Alati kvalitete: `cargo-deny` (instaliran), `cargo-llvm-cov` (instaliran), `cargo-tarpaulin` (nije instaliran)

## 2. SHA-256 sažeci referentnih goCOP datoteka (`/Users/tomislavkraljevic/projekti/goCOP`)

| Datoteka | SHA-256 |
|---|---|
| `internal/ledger/ledger.go` | `a67054bc1f6f91ad60cbda74bbd52532186b247496d06b7c4b4dcf6dc7ee6063` |
| `internal/peers/upis.go` | `2b1f7f1390bae9203d2e80aeef13b4a4cf32cf63a89aa94ceee389c35de3a0fd` |
| `internal/peers/razgovor.go` | `eb59517963cedd2886cfaea9ab4c64fbd0d8c5344e553b524b685ef8ae752cba` |
| `internal/peers/rupe.go` | `d04219474a5bdd09c718d245abd16d3acdd4552331277dcf5e2c25fa3d3171b8` |
| `internal/razmjena/keys.go` | `720d53125d9f26e4a2aa65b553671c99b414e7a850dabc5187ba20cc6e4c417a` |
| `internal/razmjena/pair.go` | `11981e0c400e9d583f27671152c80c307878ad81681367cde262a72fc583d23f` |
| `internal/razmjena/network.go` | `69e09702cfd9c9e044ba0ec6ba1bda28d727024727c05707e8278af4bd455519` |
| `internal/razmjena/opoziv.go` | `8fca9c3aa36d6169881f23b88a2ac62d208388988be3d2adf10e5eb2d10149c5` |
| `internal/razmjena/udp.go` | `7b223027c991519b198e277668f004153569a62e261be9b27fe7092bc635f07d` |
| `internal/razmjena/stun.go` | `466d30c3d586d68ecb336e6d1cb0623a323ad291c6ebcdf8b10d7f79bbccaca9` |
| `internal/razmjena/udarac.go` | `169d5557fe8817573f34237223db023ab9dc96bf6fcbb00bff3e7d54ce8d47a4` |
| `internal/razmjena/quic_slusalica.go` | `1c72a74ea4b2dd28f05a28ea2d8f9388f899e10ec35d89d91d05a28ea2dbcc8f` |
| `internal/razmjena/poziv.go` | `2714a7bc1dfa4d6a895eef0ebb50b4d2cec3d838a0bdfa7adabac31d05fd1d80` |
| `internal/razmjena/okvir.go` | `2d8ccf6a7ab2dd2bab5cd345513453401dd95e6877054b44a932ae71cb5d4e73` |

## 3. Rezultati zatečenog stanja (prije popravaka u `S00`)

1. **`cargo fmt --all -- --check`**: **PAO** (kod `1`), diff u `crates/list2cart-core/src/db.rs:1132`.
2. **`cargo test --workspace`**: **PROŠAO** (`0`), ukupno `12` testova u `list2cart-core` (`0` testova u `list2cart`, `list2cart-lib`, `xtask`).
3. **`cargo clippy --workspace --all-targets -- -D warnings`**: **PAO** (kod `101`), `clippy::too_many_arguments` (8/7) na `src-tauri/src/lib.rs:55` (`fn upsert_product`).
4. **`bun run build` (`tsc && vite build`)**: **PROŠAO** (`0`).
5. **`cargo deny --config quality/deny.toml check`**: **PROŠAO** (`0`): `advisories ok, bans ok, licenses ok, sources ok` (uz `duplicate` upozorenja za `base64` i `bit-set` iz Tauri stabla).
6. **`cargo llvm-cov --package list2cart-core --summary-only`**: **PROŠAO** (`0`):
   - `calc.rs`: 100.00% linija (50/50), 100.00% regija
   - `db.rs`: 88.53% linija (1343/1517), 80.97% regija
   - `excel.rs`: 87.59% linija (120/137), 83.73% regija
   - `ledger.rs`: 87.58% linija (437/499), 81.97% regija
   - `models.rs`: 70.59% linija (12/17)
   - `online.rs`: 0.00% linija (0/48)
   - `sync.rs`: 0.00% linija (0/90)
   - **Ukupno**: 83.21% linija, 76.83% regija.

## 4. Zatečene funkcije vs. nedostajući dijelovi

### Što već postoji
- `crates/list2cart-core/src/ledger.rs`: `record_versions` tablica, strogo monotoni UUIDv7 (`next_monotonic_version_id`), `record_version`, `get_frontier`, `compute_delta`, `ingest_remote_versions`, `auto_compact`, `vacuum_db`, `network_members`, `shared_lists_allows`, `verify_peer_identity`.
- `crates/list2cart-core/src/db.rs`: atomski `save_receipt`, `export_sync_bundle_filtered` (filtriranje po `shared_lists` i isključivanje računa za `FamilyMember`), `import_sync_bundle` u transakciji sa spajanjem postojećih popisa i stavki.
- `src/main.ts` i `src-tauri/tauri.conf.json`: XSS zaštita (`escapeHtml`, `sanitizeImageUrl`, `sanitizeColor`), CSP, čuvanje stanja polja obrasca u modelu, povezivanje `@tauri-apps/plugin-barcode-scanner` i `BarcodeDetector`.

### Što još treba izgraditi / nadograditi (`S01`–`S17`)
1. **Transport i ugovor (`S01`–`S02`)**: novi `crates/list2cart-sync`, jedinstveni `AsyncUdpSocket` demux za STUN/udarce/QUIC, Ed25519 identitet izvan baze, TLS 1.3 s provjerom ključa, commit–reveal SAS, potpisane poruke i testni vektori.
2. **Migracije sheme (`S03`)**: verzionirane migracije (`schema_migrations`), stabilni `sync_id` za `price_history` i `receipt_items`, relacijski `sync_id` stupci (`store_sync_id`, `receipt_sync_id`, `list_sync_id`), `archived` tombstones, tablice povjerenja (`sync_trust_records`), potvrđenog stanja partnera (`sync_peer_state`), rupa (`sync_holes`) i odgođenih projekcija (`pending_projections`), te detekcija klonirane baze bez pripadnog ključa.
3. **Atomičnost knjige (`S04`)**: sve domenske mutacije (uključujući `create_shopping_list`, `add_planned_item`, `toggle_planned_item`, brisanja/tombstones) moraju upisati verziju u istoj SQLite transakciji uz propagiranje pogreške i ispravan kanal (`private:<principal_id>` ili `list:<list_sync_id>`), bez `VACUUM`-a unutar poslovne transakcije.
4. **Jedinstveni prijem (`S05`)**: `apply_remote_versions` koji validira shemu, potpis autora, ovlast pisanja i brojeve, čuva izvorni `version_id`/`author`/`created_at`, rješava dijete prije roditelja (`pending_projections`) i projicira deterministički po UUIDv7 poretku.
5. **Rupe, straničenje i sažimanje (`S06`)**: straničenje delte po bajtovima (`max_bytes`), odvajanje lokalno primljenog od potvrđene neprekinute pokrivenosti, evidentiranje rupa i odbijenih zapisa, oporavak nakon sažimanja (`CompactionSnapshot`).
6. **Prava po primatelju i zaštićeni paketi (`S07`)**: potpisani `Membership`/`ListGrant`/`Revocation`, stroga izolacija izvoza za pojedinog primatelja (bez unije prava i bez privatne povijesti cijena na zajedničkom popisu), šifrirani `ProtectedBundleV2` i ograničavanje starog `import_sync_bundle` na lokalni legacy uvoz bez promjene povjerenja.

## 5. Popis planiranih migracija (`v1 -> v2` u `S03`)
1. Kreiranje `schema_migrations`.
2. Dodavanje `archived INTEGER NOT NULL DEFAULT 0` na `stores`, `products`, `receipts`, `shopping_lists`, `receipt_items`, `planned_items`.
3. Dodavanje `sync_id`, `store_sync_id`, `product_sync_id`, `updated_at` na `price_history` s determinističkim UUIDv5 popunjavanjem postojećih redaka.
4. Dodavanje `sync_id`, `receipt_sync_id`, `updated_at` na `receipt_items` s determinističkim UUIDv5 popunjavanjem.
5. Dodavanje `list_sync_id` na `planned_items` popunjen iz `shopping_lists.sync_id`.
6. Proširenje `record_versions` stupcima `author_pub_key`, `grant_ref`, `signature`, `created_at_unix`.
7. Kreiranje `sync_trust_records`, `sync_peer_state`, `sync_holes`, `pending_projections`.
