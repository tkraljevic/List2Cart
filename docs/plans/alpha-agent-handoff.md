# Ugovor spreman za drugog agenta

src/ui/contracts.ts je pripremljen. Glavni agent jedini ga uređuje.

Molim prvi modul isporučiti s izvozima:
- renderFamily(state: FamilyViewState, helpers: ViewHelpers): string
- renderMemberDialog(state: FamilyViewState, helpers: ViewHelpers): string
- renderAbout(state: AboutViewState, helpers: ViewHelpers): string

Drugu cjelinu:
- renderPairingDialog(state: PairingViewState, helpers: ViewHelpers): string
- renderSyncProgress(progress: SyncProgress | null, helpers: ViewHelpers): string

Izvozi iz index.ts unutar pripadajućeg direktorija. Za zaseban pregled sinkronizacije predloži dodatni izvoz s postojećim tipovima. Ne stvaraj vlastiti router/store.

Prva integracija može zadržati postojeće DOM ID-jeve i data-atribute iz odgovarajućih render funkcija u main.ts: postojeći bindEvents ostaje aktivan. Nove akcije zahtijevaju dogovoreni UiAction i integraciju glavnog agenta; modul ne smije registrirati paralelne listenere nad istim kontrolama.

About dobiva verziju, licencu i izvornu ikonu od hosta. CSS koristi postojeće --primary, --text-main, --text-muted, --bg-body, --bg-card, --border, --success i --warning varijable. Mobilna granica 768 px. Nove klase s prefiksom fs- u family-sync.css; ne dirati globalni CSS.

Važno: API trenutačno ne daje granularne faze LAN sesije po uređaju ni read/write ovlasti po popisu. Prikaži samo dostupne podatke; ne izmišljati broj čekajućih promjena, postotke ili ovlasti. Postojeći SyncTracker odnosi se na praćenu lokalnu operaciju, ne automatski na sve udaljene uređaje.

Početna provjera: npm run build prolazi prije novih izmjena. Početni diff pohranjen u /private/tmp/list2cart-alpha-baseline/pre-implementation.diff. Rad nije commitan niti pushan.

Integracija A pokrenuta: renderFamily/renderMemberDialog/renderAbout povezani u main.ts, family-sync.css uvezen. src/ui/app-metadata.ts isporučuje verziju iz package.json i licencu iz Cargo.toml kroz Vite define. Testovi src/**/*.test.ts izuzeti iz produkcijskog tsc (Bun test runner provjerava ih zasebno). Ne mijenjati tuđe module tijekom integracije.

Integracija B: renderPairingDialog/renderSyncProgress/renderIdentityConflicts/renderSyncOverview također povezani. Tijek sinkronizacije dostupan iz host gumba u Obitelj, bez nove AppTab vrijednosti; poziv i pridruživanje razdvojeni kroz postojeći pairingAdmit. Moduli ostaju u vlasništvu drugog agenta.

Glavni agent preuzeo popravke iz alpha-agent-review.md: tekst pauze, pairing prikaz/ugovor i stvarno otkazivanje sesije. Ne uređivati iste datoteke paralelno dok popravak traje.
