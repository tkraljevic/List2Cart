# Stanje preuzetog rada — 17. 11. 2026.

## S07 — redoslijed dokaza i opoziv prije odgođene projekcije (18. 11. 2026.)

- Novi negativni test reproducirao je propust: valjano primljena stavka bez roditeljskog popisa ušla je u pending_projections, zatim je autor/grant opozvan, ali kasniji roditelj ipak je automatski projicirao stavku bez nove provjere prava.
- Svako pražnjenje pending reda sada ponovno provjerava potpisano autorovo povijesno pravo, trenutačno poznate opozive i trenutačni pristup primatelja kanalu. Blokirana stavka ne ulazi u projekciju. Potpisana knjiga i red ostaju očuvani; neovisni valjani roditelj i drugi zapisi mogu se normalno primijeniti.
- ApplyReport dodaje zadano prazni blocked_projections s ID-em/autorom/kanalom/razlogom. To su dijagnostike već ranije primljenih odgođenih zapisa, odvojene od rejected za nove verzije. Snapshot i dalje ne može potvrditi cjelovitost ako njegova verzija ostaje u pending redu. Nema brisanja već ranije projiciranih podataka niti promjene konzervativnog pravila povijesnog opoziva.
- Tri nova testa: grant ili uređaj opozvan prije zakašnjelog roditelja (dokaz očuvan, valjani roditelj prihvaćen, stavka blokirana); članstvo i grant dolaze u oba redoslijeda nakon odbijene verzije, replay poslije dokaza uspije i coverage/ACK se zatvara bez duplikata; grant/key opoziv i domenski zapis u istom paketu s oba ulazna redoslijeda uvijek obrađuju opoziv prije domene, replay ne vraća prava.
- Opaženo: 100 core + 24 sync = 124 workspace testa prolazi; strict Clippy i fmt check prolaze. Bez promjene sheme v6, potpisanih formata, ovisnosti, produkcijskih baza/ključeva, UI-a ili commita.
- Preostaje provjera potpunog snapshot oporavka s ranijim verzijama opozvanog autora i protuslovnim povijesnim dokazima: konzervativna provjera može odbiti takav snapshot, pa se cijeli S07 audit još ne proglašava dovršenim. Pairing/TLS/S09 LAN i Tauri/UI ostaju iza toga.

## S07 — provjera isteka i administratorskih opoziva (18. 11. 2026.)

- Dva nova negativna testa prvo su reproducirala propuste: cijeli opoziv administratorova ključa nije zatvarao ranije delegirane read grantove, a stara duga ugrađena Ovlast mogla je izdati novi certifikat nakon novije skraćene potvrde.
- Opoziv `membership`/`legacy_key` sada postavlja revoked i na prava izdana preko toga administratora. Odabir primateljevih kanala dodatno provjerava potpisano članstvo, grant i opoziv/delegaciju pri uporabi, pa stare neistinite revoked=0 oznake ne vraćaju čitanje ni privatni own-device pristup. Postojeća eksplicitna revoked=1 zabrana i dalje se poštuje. Za zaštitu postojećih cacheova nije potrebna migracija.
- Pri izdavanju članstva/granta ugrađena delegacija mora odgovarati najnovijoj poznatoj Ovlast potvrdi izdanoj do tog trenutka. Ista provjera radi i za povijesno članstvo te čitanje/pisanje već pohranjenog granta. Novija potvrda primljena naknadno zato zatvara nevaljano novije izdavanje preko stare potvrde. Povijesno izdavanje prije promjene ostaje dopušteno. Write provjera sada provjerava stvarni potpisani grant (ključ, popis, ID, permission, rok), a ne samo indeksirane cache stupce.
- Treći novi test prati točnu granicu isteka granta i automatski razgovor: grant vrijedi na expires_at, nakon te sekunde stari djelomični list snapshot zamjenjuje se kontrolnim bez primjene staged podataka; članstvo može ostati aktivno. Nakon isteka i članstva novi zahtjev se odbija.
- Opaženo: 97 core + 24 sync = 121 workspace test prolazi; strict Clippy i fmt check prolaze. Bez promjena sheme v6, potpisanih formata, ovisnosti, produkcijskih baza/ključeva, frontend/UI-a ili commita.
- Ovo zatvara pronađene propuste u tom dijelu pregleda, ne cijeli S07 audit. Ostaju širi scenariji povijesno opozvanih autora i prijenosa nakon promjena ovlasti te pairing/TLS/S09 LAN i povezivanje Tauri/UI-a. Automatski obiteljski dispatcher iz prethodnog koraka ostaje dostupan.

## S07 — automatski obiteljski razgovor i completion ACK (18. 11. 2026.)

- sync_request/reply/import_sync_page sada odabiru own-device v2 ili obiteljski zahtjev v3. V3 zaseban potpisni prefiks veže isti DTO, uz zabranu privatnih/kontrolnih frontier/holes/backfill podataka. family_exchange.rs provjerava članstva i zajednički scope, nastavlja postojeći izlaz nakon gubitka odgovora/restarta te mijenja zastarjele dokaze/scope novim snapshotom.
- Obiteljski put konzervativno prenosi potpuni ograničeni snapshot nakon promjene podataka/prava/backfilla. Nepromijenjeni potvrđeni prijenos završava malim praznim Delta odgovorom; veliki skup dokaza ne mora ponovno stati u jednu stranicu. Own-device delte ostaju.
- Tek potpisani sljedeći zahtjev s točnim potpunim SnapshotProgress oslobađa zamrznuti izlaz; completion oznaka i brisanje izvedenih stavki dijele transakciju. Ponovljena potvrda je sigurna. Izvoz zadnje stranice nije ACK. Zastarjeli izlazi i djelomični obiteljski ulazi istog peera mogu se zamijeniti bez brisanja domenskih projekcija.
- Novi grant može doći stagingom prije nego ga primatelj lokalno poznaje. Potpisani lanac i završno pravo na svaki kanal moraju proći prije projekcije. Lokalni trust-cache hash čuva početno stanje djelomičnog ulaza; njegova promjena prekida nastavak i pri idućem zahtjevu traži početak. To je lokalni dodatak spremljenom manifestu, bez promjene žičanog SnapshotManifest DTO-a ili sheme v6.
- Dva nova integracijska testa preko zajedničkog request/reply/receive API-ja: veliki dokazi koji ne stanu u 5 KiB delta stranicu, izgubljen prvi odgovor i isti ID, stvarni disk reopen obaju izoliranih SQLite izvora, nastavak, signed completion/tamper, SQL fault rollback čišćenja/retry, ponovljena idle potvrda, kasni neovisni item ispod MAX-a pokreće novi snapshot; promjena prava usred prijenosa, opoziv i kasniji novi read grant automatski zamjenjuju epoch i vraćaju samo read pristup.
- Opaženo: 94 core + 24 sync = 118 workspace testova prolazi; strict Clippy i fmt check prolaze. Bez produkcijskih baza/ključeva, migracije, novih ovisnosti, commita ili frontend/UI promjena.
- Sljedeće: šira matrica vremenskih promjena/isteka/delegiranih i opozvanih povijesnih autora te preostali S07 sigurnosni pregled; zatim pairing/TLS životni ciklus i S09 LAN. Obiteljski prijenos trenutačno ponavlja snapshot za svaku promjenu i može biti skuplji od delte. Mrežni razgovor i Tauri/UI nisu povezani; aplikacija nije proglašena dovršenom.

## S07 — trajni obiteljski snapshot i podijeljeni dokazi (18. 11. 2026.)

- Dodan izričiti `Database::start_family_snapshot_recovery`; postojeći export/import_snapshot_page i snapshot_progress nastavljaju format 2 nakon ponovnog otvaranja baze. Format 1 ostaje vlastitim uređajima. Obiteljski format prenosi zajedničke dopuštene list kanale i odabrane izvorne kontrolne dokaze, bez privatnih kanala ili globalnog kontrolnog frontier/ACK/backfill tokena.
- Skup dokaza više ne mora stati u jednu stranicu: zamrznuti dokazi i domenske verzije prenose se kroz manifest/hash/offset u postojećim trajnim staging tablicama. Ni dokazi ni projekcije ne primjenjuju se prije potpunog hash/veličina/frontier pregleda i jedne završne transakcije. Ostaju kvote 100.000 verzija, 64 MiB i 8 nedovršenih sesija po smjeru; pojedinačna verzija i manifest moraju stati u stranicu do 1 MiB.
- Pristup obaju članova provjerava se pri svakom nastavku i završnom commitu. Inicijalna aktivna članstva i potrebni grantovi moraju već biti poznati kroz uparivanje/dodjelu. Izvor veže sažetak odabranih kontrolnih dokaza: relevantna promjena prava/opoziv ili novi grant za isti kanal zaustavlja stari izlaz i zahtijeva novi snapshot. Izvoz prava i sadržaja stranice koristi jedan DB snapshot. Kontrolne rupe ne lažno proglašava pokrivenima; domenske rupe na izvoru i dalje blokiraju potpuni snapshot.
- Tri nova testa: skup dokaza veći od 5 KiB stranice, nastavak nakon stvarnog zatvaranja/otvaranja obaju SQLite izvora, idempotentno ponavljanje, bez djelomičnih projekcija, final SQL fault/rollback/retry, potvrda samo domenskog backfilla; opoziv na oba kraja i regrant istog kanala zaustavljaju stari prijenos; privatni opseg/globalni kontrolni ACK/skok offseta/promjena manifesta/downgrade u format 1 se odbijaju.
- Opaženo: 92 core + 24 sync = 116 workspace testova prolazi; strict Clippy i fmt check prolaze. Nema nove DB migracije (v6), novih ovisnosti, promjena produkcijske baze/ključeva, commita ni UI promjena. Novi opcionalni manifest proof_sha256 koristi format 2; stari format 1 ga ne zahtijeva.
- Sljedeće: povezati ovaj obiteljski put s autentificiranim automatskim request/reply odabirom, potpisanom potvrdom završetka/čišćenjem izlaza te promjenama grantova i backfilla između razgovora. Automatski exchange je i dalje own-device; ovaj izričiti API nije LAN/S09 ni UI. Odbačeni/napušteni izlazi zasad se uklanjaju postojećim discard API-jem. Šira matrica povijesno opozvanih autora ostaje otvorena.

## S07 — obiteljske stranice i potrebni kontrolni dokazi (18. 11. 2026.)

- Novi family.rs izračunava kontrolne ovisnosti prije budžeta/straničenja: članstva primatelja i autora dostupnih zapisa, točne autorove grantove, primateljeve vlastite grantove/opozive te potrebne administratorske potvrde i opozive. Nepovezani članovi, autorovi drugi grantovi, administratori i opozivi izostaju. To je skup za trenutno dopuštene kanale, ne optimizirani minimum pojedine stranice.
- Potpuni odabrani skup izvornih potpisanih dokaza ponavlja se uz svaku stranicu. Kontrolni frontier primatelja ga ne može preskočiti; kontrolni kanal ne dobiva covered_ranges ni sender_holes. Prijem preko druge osobe zanemaruje i nametnutu potvrdu cijelog kontrolnog kanala. Podatkovni kanali zadržavaju postojeće delte/ACK. Cache-only dokaz ili prevelik skup završava jasnom greškom, bez prepotpisivanja ili djelomičnog paketa.
- Samo korijenski primatelj i vlastiti uređaji istog principala koriste puni kontrolni pogled. OwnDevice drugog principala koristi ograničeni pogled bez privatnih kanala. Ovlast dokazi obrađuju se prije delegiranih omota neovisno o UUID-u omota; opozivi i dalje prije domene. Cijeli izvoz koristi jedan DB snapshot za prava, dokaze i podatke.
- Šest novih testova: više stranica i neotkrivanje čak ni velikog nepovezanog kontrolnog zapisa; ponavljanje paketa i završetak bez prazne beskonačne stranice; nedostajući izvorni dokaz; opoziv i novi read grant unatoč visokom kontrolnom frontieru; delegacija s kasnijim omotom potvrde; tuđi OwnDevice bez privatnih podataka/globalnog ACK; SQL fault vraća dokaze/podatke/napredak i retry uspijeva.
- Opaženo: 89 core + 24 sync = 113 workspace testova prolazi; strict workspace Clippy i fmt check prolaze. Bez nove sheme (ostaje v6), formata potpisa, ovisnosti, frontend/UI promjena, produkcijskih podataka/ključeva ili commita.
- Preostaje S07: zaseban ograničeni prijenos velikih skupova dokaza, trajni obiteljski snapshot/backfill oporavak i šira matrica povijesno opozvanih autora. Sadašnja konzervativna provjera može odbiti prosljeđivanje starih promjena opozvanog autora; ovaj korak to ne zaobilazi. Automatski request/reply/recovery ostaje samo own-device. LAN/S09, pairing životni ciklus i Tauri/UI nisu povezani; S07 i aplikacija nisu proglašeni dovršenima.

## S07 — povijesna članstva i delegacije (18. 11. 2026.)

- Prijem sada zadržava potpisane istekle potvrde kao povijesne dokaze. Autorova prava provjeravaju se u trenutku promjene, a delegirani kontrolni potpisi prema tadašnjoj Ovlast potvrdi. Članstvo se rekonstruira iz cachea i očuvane kontrolne knjige, pa novije članstvo više ne skriva staro valjano izdavanje.
- Najnovije prethodno izdavanje ima prednost; nakon njegova isteka ili niže uloge nema vraćanja starijeg šireg prava. Dvosmislen sadržaj istog vremena odbija se i u povijesti. Današnji pristup i dalje zahtijeva aktualne rokove. Opoziv ključa/granta/delegacije ostaje konzervativan i za stare verzije.
- Tri nova testa pokrivaju novije članstvo prije stare potvrde, istekli grant uz valjanu staru promjenu i odbijanje nove, delegirane stare omote nakon isteka administratora, odbijanje novog izdavanja izvan delegacije, downgrade/istek bez povratka na OwnDevice, dvosmislenost te očuvanje kontrolnih dokaza nakon sažimanja.
- Opaženo: 83 core + 24 sync = 107 workspace testova prolazi; strogi workspace Clippy i fmt check prolaze. Izolirane baze/ključevi; bez produkcijske migracije, promjene sheme v6, novih potpisanih formata, commita ili UI promjena.
- Sljedeće: minimalni kontrolni dokazi za obiteljskog primatelja. Postojeći izvoz filtrira nakon računanja stranice, propušta nepovezane Ovlast/opoziv zapise i izostavlja dokaze drugih autora. Treba filtrirati prije budžeta i uskladiti kontrolne granice/ACK te ponovno dopuštene kanale. To nije riješeno ovim korakom. Automatski exchange/recovery ostaje za vlastite uređaje; S07/family, S09 LAN i Tauri/UI nisu dovršeni.

## S07 — opsezi opoziva i kontrolni identiteti (17. 11. 2026.)

- Ispravljen reproduciran kvar: opoziv granta za jedan popis više ne ukida drugi popis ni članstvo istog uređaja. Odvojeni su opsezi članstva, granta i administratorske delegacije; opozvana ugrađena Ovlast ne vraća delegirana prava. Read grant ne dopušta pisanje.
- Kontrolni ID ne može preuzeti drugi identitet/principal/cilj. Grant je nepromjenjiv; ponovno dopuštanje koristi novi ID. Staro članstvo ne prepisuje novije; buduće izdavanje nije aktivno. Korijenski mrežni ključ ne može biti opozvan kao uređajsko članstvo.
- Dodan acl.rs i atomski kontrolni savepoint: greška ažuriranja oznaka poništava i dokaz opoziva, uključujući prijem paketa koji nastavlja druge valjane zapise. Migracija v6 normalizira opsege i popravlja stare preširoke oznake; nepoznate/dvosmislene legacy opozive zadržava blokiranima. Fault-injection potvrđuje rollback i ponavljanje migracije.
- Opaženo: 80 core + 24 sync = 104 workspace testa prolazi; strogi workspace Clippy i fmt check prolaze. UDP test zahtijeva izlazak iz sandboxa radi lokalnog porta; puna provjera tako prolazi. Testovi koriste izolirane baze/ključeve. Nema produkcijske migracije, commita ni UI/runtime promjena u ovom koraku.
- Referenca je goCOP trust/opoziv model iz prethodnog pregleda, prilagođen privatnim principal/list kanalima; nisu mijenjani Go izvori. S07 nije dovršen: sljedeći su povijesni/delegirani dokazi, minimalni kontrolni dokazi obiteljskog primatelja i matrica ponovne dodjele prava. Automatski exchange/recovery ostaje samo za vlastite uređaje; obiteljski transport, S09 LAN i Tauri/UI nisu povezani.

Antigravity je ostavio izvršivi prototip i djelomičnu izvedbu F1, bez završnog izvještaja. Njegov vidljivi zadatak bio je započeti F0 (S00–S02). Ovo je pregled stvarno provjerenog stanja nakon preuzimanja; ne označava svih 18 zadataka završenima.

## Provjereno i popravljeno

- Inicijalizacija baze sada stvarno poziva novu migraciju. Time su uklonjena tri pada testa zbog nedostajućeg `author_pub_key` stupca.
- Migracije imaju test ponovljivog backfilla veza/ID-eva, rollbacka nakon greške, odbijanja buduće sheme i odbijanja klonirane ili neispravne vanjske identifikacije.
- UDP prototip više ne prikriva zauzet port prelaskom na drugi. Test ima ukupni rok, provjerava gašenje svih tokova i ponovno otvaranje istog porta. Prerano ubijanje QUIC I/O-a popravljeno je; registri i redovi su ograničeni, dupli ID-evi odbijeni, poll predaje vrijeme drugim taskovima.
- Potpis verzije sada veže i `node_id`, `created_at` i autora te ima jednoznačne granice polja. Dodani su negativni testovi prepravljanja metapodataka i datuma/rokova ovlasti.
- Provjeravaju se UUIDv7 vrijeme i RFC3339 vrijeme, odnos stavke i kanala, write grant te zabrana preuzimanja istog identiteta iz drugog kanala. Sama `own_device` uloga ne daje sva prava svih zajedničkih popisa.
- Tri izolirane baze konvergiraju u testiranom scenariju s različitim redoslijedom dolaska roditelja/stavki, konfliktima i ponovljenim verzijama. Fault-injection projekcije potvrđuje rollback knjige i uspješno ponavljanje.
- Dodan je ponovljiv Go generator i spremljeni fixturei. Ne zahtijeva promjenu goCOP-a ni njegove produkcijske ključeve. Go i Rust potvrđuju odabrane kriptografske bajtove, uključujući novi List2Cart omot verzije.
- Novi zaštićeni paket odbija neispravne duljine, nekontributivni X25519 i prevelik plaintext. Ekstreman potpisani signalni timestamp ne ruši provjeru overflowom.
- Strogi workspace Clippy popravljen je kroz tipizirane kontekste/rokove i konkretne lint ispravke. Path dependency ima eksplicitnu verziju radi cargo-deny pravila.

## Nastavak: S04 i povezane S05 ovisnosti

- Sve postojeće domenske naredbe sada zapisuju potpuni, potpisani snapshot i promjenu u istoj transakciji: trgovina, artikl/povijest cijene, račun/stavke, popis/stavke i checked. Greška potpisa, ovlasti ili knjige poništava cijeli upis, uključujući grešku na zadnjoj cijeni računa.
- Dodan je core API za eksplicitno arhiviranje/obnovu svih sedam entiteta. Tombstones ostaju u knjizi, aktivni upiti ih skrivaju, a arhiviranje stavke računa ažurira i potpisuje roditeljski zbroj. Novi UI za te operacije ostaje S15.
- `Database::open` učitava zasebne trajne device/network ključeve izvan baze i provjerava vezu s bazom prije migracija. Nedostajući vezani device ključ ne zamjenjuje se. Ponovno otvaranje replike ne prepotpisuje tuđe podatke niti stvara dodatni katalog.
- Lokalno naslijeđeni snapshoti preuzimaju se jednom kao novi lokalni zapisi. Sintetička puna v1 baza potvrđuje očuvanje vrijednosti, veza i starih nepotpisanih tvrdnji o autoru. Legacy uvoz ignorira priložene verzije/potpise, ne mijenja novi trust te stvara lokalne verzije stvarnih promjena.
- Novi lokalni zapisi imaju privatni kanal principala. Promjena zajedničkog popisa nasljeđuje njegov kanal i mora referencirati važeći write grant; read grant nije dovoljan. Lokalni uređaj potpisuje svojim ključem, odvojenim od administratorova ključa mreže.
- Rust i UI računaju stavke cjelobrojnim tisućincima i centima, uz jednako half-up zaokruživanje, ograničenja raspona i provjeru agregatnog overflowa prije commita.
- UUIDv7 zapisivač čuva slučajni sufiks kad pomiče logički sat iza primljene verzije. Vrijeme potpisa prati taj sat unutar ograničenja odstupanja. Testovi pokrivaju dva neovisna nastavka nakon verzije 30 minuta u budućnosti i neprihvaćanje starog nepotpisanog UUID-a kao novog autoritativnog sata.
- Prijem odgađa i lance trgovina → račun → stavka te artikl → cijena prema stabilnim ID-evima. Obrnuti redoslijed dolaska završava bez zaostalih projekcija. Neispravan ID artikla ili prevelik iznos odbija se bez gubitka drugih važećih zapisa u paketu.

Referenca za S04: goCOP `internal/ledger/ledger.go`, `Recorder.RecordIn`/`ArchiveIn` i `write`, pregledani HEAD iz cheatsheeta. Preuzet je transakcijski obrazac; potpisani List2Cart omot, kanali, validacija i ograničeni logički sat vlastiti su ugovor.

## Rezultati naredbi

| Provjera | Opaženi rezultat |
|---|---|
| `cargo test --workspace --offline` | 60/60 testova: core 36, sync 24; uključuje lokalni UDP/QUIC test |
| `cargo clippy --workspace --all-targets --offline -- -D warnings` | prolazi |
| `cargo fmt --all -- --check` | prolazi |
| `bun run build` | TypeScript i Vite build prolaze |
| `bun test tools/money.test.ts` | 2/2 testa, 15 provjera: isti centi/zaokruživanje u UI-u, odbijanje neispravnih količina/iznosa i prikaz rubnog dopuštenog iznosa |
| `cargo deny --offline --config quality/deny.toml check` | advisories, bans, licenses, sources prolaze nakon ispravka path verzije; provjera koristi lokalnu RustSec bazu, ne novo mrežno osvježenje |
| Go regeneracija + `cmp` spremljenih fixturea | bajtovno jednaki |

U ovom okruženju loopback bind i RustSec lock zahtijevali su pokretanje izvan sandboxa; test nije preskočen. Postojeća upozorenja o višestrukim verzijama tranzitivnih biblioteka nisu pretvorena u zabrane niti utišana izmjenom pravila.

## Status po planu

| Zadaci | Stanje |
|---|---|
| S00 | postoji Antigravity baseline; gornje naredbe daju aktualnu provjeru preuzimanja |
| S01 | lokalni isti-port STUN/udarci/QUIC prototip potvrđen; mobilni build i stvarni NAT ostaju neprovjereni |
| S02 | ugovor i ponovljivi vektori postoje; sigurnosni pregled punog trust/recovery protokola ostaje otvoren |
| S03 | migracije i jednokratno potpisano preuzimanje sintetičke pune v1 baze provjereni; stvarna kopija korisničke baze nije migrirana u testu |
| S04 | implementirano za postojeće domenske naredbe i core arhiviranje/obnovu svih sedam entiteta; fault-injection, ovlasti, zbrojevi, sat i trajni identitet pokriveni testovima |
| S05 | potpisane lokalne naredbe, svih sedam tombstone/restore projekcija i obrnute ovisnosti prolaze; potpuni ugovor računa, relacijske kolizije i novi paket još zahtijevaju završetak |
| S06–S07 | postoje nacrti; pokrivenost/rupama, snapshoti, delegirani opozivi i potpuni ACL izvoz još nisu zaključeni testovima |
| S08–S17 | mrežno uparivanje, autentificirani LAN, sadržaj, sastajalište, stvarni WebSync, fallback, Tauri lifecycle/UI i mobilna provjera nisu dovršeni |

## Točka nastavka

### Vidljiv status razmjene paketa

- `src/sync-progress.ts` prati faze stvarnih async operacija, završetak, vrijeme posljednje promjene i grešku na točnom koraku. Blokira preklapanje prijenosa; greška osvježavanja nakon uspješnog commita izričito kaže da su podaci već spremljeni.
- UI status ostaje vidljiv preko promjene taba. Uvoz ima faze čitanja datoteke, provjere/primjene u bazu i provjere cjelovitosti/osvježavanja; rezultat uključuje brojeve zapisa i čekajuće račune. Izvoz ima izradu paketa i spremanje datoteke. Nema izmišljenih postotaka ni potvrde primitka na drugom uređaju.
- Tijekom stvarnog QA izvoza stari Blob-download put ostavio je aplikaciju bez dostupnog UI odgovora. Uzrok nije definitivno dijagnosticiran; QA proces je zaustavljen i zamijenjen rebuildom. Native Tauri izvoz sada sprema jedinstveno nazvanu datoteku u Downloads preko `save_sync_bundle_file`, s `create_new`, `write_all` i `sync_all`. Browser fallback ostaje Blob-download. Stvarno native spremanje i završni status potvrđeni su u `hr.list2cart.qa` aplikaciji; testni paket `List2Cart_SVE_1794945884573918000.list2cart.json` ostao je u Downloads.
- Opaženo: 4 Bun testa / 26 provjera (uključujući dva nova testa faza, grešaka i preklapanja), frontend build, workspace strict Clippy, fmt check i QA `.app` build prolaze. Uvoz nije dodatno UI-testiran u ovom koraku; njegov backend je pokriven ranijim 49 core testova. Aplikacija ostavljena otvorena na dovršenom izvozu.
- Status se sada odnosi na postojeće ručne pakete. Automatski LAN/WebSync prijenos još ne postoji i nije prikazan kao dovršen.

### Cjelovitost računa kroz odvojene pakete

- Potpisani `ReceiptSyncPayload` uključuje opcionalni `item_ids`: točan skup aktivnih stavki. Novi lokalni račun potpisuje zaglavlje nakon stavki, unutar iste transakcije; arhiviranje/obnova izrađuje novi aktualni manifest. Popis odbija duple/prazne ID-eve i više od 10000 stavki.
- `receipt_is_complete` uspoređuje točan skup projiciranih aktivnih ID-eva i cjelobrojni zbroj s najnovijim potpisanim zaglavljem. Provjerava i overflow. Računi koji čekaju stavke ili imaju neusklađen zbroj ostaju sačuvani u bazi/knjizi, ali ne ulaze u `list_receipts` ni legacy izvoz kao dovršeni računi.
- Tauri/TS `incomplete_receipt_count` i obavijest u UI-u pokazuju broj nedovršenih projiciranih računa. Dolazak ostatka paketa prirodno dovršava prikaz bez novog lokalnog potpisa i bez duplikata.
- Pri otvaranju izvornog autorskog uređaja stara potpisana lokalna zaglavlja bez manifesta dobivaju manifest jednom. Tuđa potpisana zaglavlja ne prepotpisujemo: ona čekaju ažurirano zaglavlje izvornog autora. Nepotpisani legacy snapshoti i dalje prolaze eksplicitni lokalni uvoz i lokalno potpisano preuzimanje.
- Opaženo: **49 core testova**, workspace strict Clippy, fmt check i frontend build prolaze. Novi testovi: tri izolirane replike s različitim redoslijedom/paketima/replayem, nedostajuća besplatna stavka, netočan zbroj pa ispravak, nevaljan manifest uz valjan nepovezan zapis, prekid na disku/ponovno otvaranje/nastavak prijenosa i jednokratna nadogradnja lokalnog starog zaglavlja. Postojeći archive/restore i transakcijski fault testovi također prolaze.
- Ove najnovije promjene nisu ponovno pokrenute u macOS testnoj `.app`; provjeren je frontend build i Tauri kroz workspace Clippy. Sljedeće: širi S06 ugovor raspona/rupa, straničenja oporavka i novih grantova. Nemoj zaključiti da je cijeli S05–S07 sigurnosni audit dovršen samo iz ovih testova.

### Automatsko sažimanje i VACUUM

- `auto_compact` sada atomarno briše suvišne međuv­erzije i sprema `compaction_floor`; kvar spremanja granice poništava i brisanje. Odbija aktivnu poslovnu transakciju i nevaljanu politiku. VACUUM je zaseban korak nakon commita.
- Trajno se zadržavaju sve tombstone verzije, kontrolni dokazi, prve potpisane veze identiteta, aktualni snapshoti i zadnji zapisi svakog autora, pending projekcije i tokovi s otvorenim rupama. Arhivirani domenski podaci se ne brišu trajno. Prag 30 dana / 3 verzije odnosi se samo na ostale suvišne međuv­erzije.
- `run_scheduled_maintenance` radi najviše jednom dnevno, uz trajno vrijeme posljednjeg uspjeha tek nakon sažimanja i VACUUM-a; neuspjeh ostaje za ponovni pokušaj. Tauri radnik provjerava pri pokretanju i svakog sata, s SQLite poslom na blocking izvršitelju pod postojećim mutexom. Dok OS suspendira aplikaciju nema obećanja pozadinskog rada.
- UI opis usklađen je sa stvarnim ponašanjem. Testna `.app` ponovno izgrađena i otvorena. Read-only provjera baze `hr.list2cart.qa` potvrdila je `maintenance_last_success` i `freelist_count=0` nakon automatskog starta.
- Opaženo: **45 core testova**, workspace strict Clippy, fmt check, frontend build i debug `.app` build prolaze. Novi testovi pokrivaju potpisani tombstone, zastarjelu repliku/replay, zaštićeni paket nakon sažimanja, očuvanje kontrolnih zapisa, dnevni raspored i ponovno otvaranje diska te rollback verzija/granica/vremena pri grešci.
- Ovo dovršava konzervativno automatsko održavanje; širi S06 ugovor potvrde raspona, straničenja snapshotova, novih grantova i zatvaranja svih rupa **nije proglašen završenim**. Ne tvrditi da se arhivirani zapis trajno uklanja nakon roka niti da je puna mrežna sinkronizacija dovršena.

### Provjera u pokrenutoj macOS aplikaciji

- Izgrađen i pokrenut `target/debug/bundle/macos/List2Cart Test.app` s privremenim configom `/private/tmp/list2cart-qa.json`, identifier `hr.list2cart.qa`, naslov `List2Cart TEST — Duplikati`. Koristi zaseban testni profil.
- Kroz stvarni Tauri UI dodana su dva `QA Duplikat` zapisa: pregled odabira, Odustani (oba ostaju), potvrda (drugi se arhivira), ponovno otvaranje (aktivan ostaje `Prvi zapis`) provjereni su opažanjem.
- Otkriven i ispravljen stale pregled: sada se sukobi učitavaju pri initu, nakon dodavanja trgovine i legacy uvoza; osvježavaju se i statistike. Rebuild testne `.app` prolazi. Nakon novog unosa sukob se odmah prikazao; nakon ponovnog otvaranja prikazan je već na početnom ekranu.
- Aplikacija je ostavljena otvorena s dodatnim testnim parom (`Prvi zapis` / `Provjera osvježavanja`) za korisnikov pregled. Prethodni `Drugi zapis` ostaje arhiviran. Nema aktivnog Vite dev servera. Ovaj nalaz zamjenjuje starije ograničenje da UI tok nije provjeren u Tauriju.
- Sljedeće ostaje provjera cjelovitosti računa kroz više paketa (S05), pa S06/S07.

### Aktualno: pregled i potvrđeno razrješenje duplikata

- Tauri naredbe `identity_conflicts` / `resolve_identity_conflict` povezane su s TypeScript API-jem. Pregled prikazuje odvojene zapise s nazivom, lokacijom/cijenom i punim ID-em, izbor zapisa, pregled arhiviranja, potvrdu, odustajanje, osvježavanje i greške.
- Pregled je vidljiv iznad sadržaja kada postoje sukobi; u Sinkronizaciji je dostupan i bez sukoba. Potvrda čuva odabrani aktivni zapis i potpisano arhivira ostale u jednoj transakciji. Odbija zastarjelu ili nepotpunu grupu i nepodržan entitet. Povijesne reference i payloadi se ne prepisuju.
- Ovo je razrješenje izborom aktivnog zapisa i arhiviranjem duplikata, **nije alias koji automatski prenosi polja/povijest**. UI to izričito objašnjava prije potvrde. Arhivirani zapisi zadržavaju stabilni identitet; kasnija eksplicitna obnova može ponovno stvoriti sukob po postojećem tombstone ugovoru.
- Budući unos po barkodu bira jedini aktivni artikl, bez mijenjanja arhiviranog duplikata. Cijene se dohvaćaju za njegov stabilni ID. Dvosmislen klik kataloga prikazuje grešku umjesto detalja proizvoljnog proizvoda.
- Opaženo: **43 core testa**, strogi workspace Clippy (uključuje Tauri), fmt check i frontend build prolaze. Testovi uključuju stale izbor, rollback na neuspjelom potpisanom zapisu, replikaciju arhiviranja, očuvanje povijesne veze i budući upis u odabrani artikl. Browser provjera obuhvatila je prazan prikaz sukoba; UI potvrda s aktivnim duplikatima nije end-to-end isprobana u Tauri aplikaciji. Privremeni Vite server ugašen.
- Sljedeće: cjelovitost računa kroz više paketa, zatim S06/S07. Nema rada sa stvarnom bazom ni commita.

### Aktualno: oba identiteta ostaju sačuvana

Korisnik je odabrao: **sačuvati oba zapisa i ponuditi ručno spajanje**, umjesto automatskog izbora pobjednika.

- Shema v3 uklanja jedinstvenost naziva trgovine i barkoda artikla. `sync_id` ostaje jedinstveni identitet. Migracija čuva lokalne integer ID-eve i veze trgovina/računa/popisa te povijest cijena, provjerava foreign keys prije commita i vraća izvorno stanje FK postavke i pri grešci. Barcode FK povijesti uklonjen je jer barkod više nije identitet; potpisani prijem i dalje provjerava `product_sync_id` + barkod.
- Potpisani prijem prihvaća različite ID-eve s istim nazivom/barkodom; projekcija artikla radi po `sync_id`. Nepromjenjivost roditelja/barkoda istog ID-a ostaje provjerena, uključujući odgođene račune/stavke/cijene.
- `Database::identity_conflicts()` vraća aktivne sukobe u stabilnom poretku. Suprotan redoslijed paketa daje iste grupe i čuva oba ID-a. Upis/pretraga po dvosmislenom barkodu i dohvat cijena prijavljuju sukob umjesto proizvoljnog odabira.
- Legacy uvoz također traži trgovine/artikle po stabilnom ID-u; račune i popise povezuje preko izvorne tablice trgovina iz paketa. Fallback po nazivu dopušten je samo za jednoznačan rezultat; uklonjen je fallback na prvu lokalnu trgovinu.
- Opaženo: **41 core test**, strogi workspace Clippy i fmt check prolaze. Testovi uključuju migraciju/ponavljanje, očuvanje FK i rollback na nevaljanoj vezi, suprotan redoslijed kolizija i odbijanje dvosmislenog lokalnog upisa bez promjene knjige.
- **Još nije implementirano:** Tauri/UI prikaz sukoba i eksplicitna potpisana operacija ručnog spajanja/aliasa uz očuvanje povijesti i referenci. Backend popis sukoba nije dovršena korisnička funkcija spajanja. S05 računi kroz više paketa i S06/S07 ostaju otvoreni.

Sljedeći stariji odjeljak o odbijanju prirodnih kolizija opisuje prethodni korak; zamijenjen je shemom v3 i čuvanjem oba zapisa.

### Nastavak: zaštita stabilnog identiteta

- Potpisani prijem prije upisa u knjigu odbija drugi stable ID s već zauzetim nazivom trgovine ili barkodom artikla; ne prevezuje postojeći zapis. Projekcija trgovine traži samo `sync_id`, a upsert artikla ne mijenja `sync_id`.
- Veze artikl/barkod, cijena/artikl/trgovina, račun/trgovina i stavka/roditelj ne smiju se mijenjati kroz već prihvaćene potpisane verzije, uključujući odgođene projekcije.
- Potpisani popis ne traži trgovinu po nazivu bez stabilne reference.
- Testovi potvrđuju odbijanje kolizija uz prihvat nepovezanog valjanog zapisa, očuvanje izvornih ID-eva i roditelja odgođene stavke. `rtk cargo test -p list2cart-core --offline`: 39 prolazi; strict workspace Clippy i fmt check prolaze.
- Ograničenje: ovo je sigurno odbijanje kolizije prema postojećoj projekciji, još nije konvergentno razrješenje dvaju legitimnih različitih ID-eva s istim prirodnim ključem. Taj dio S05 ostaje otvoren, uz cjelovitost računa kroz više paketa.

### Nastavak 17. 11.: javni API za zaštićeni paket

- `Database::export_protected_bundle` i `Database::import_protected_bundle` koriste unutarnji device ključ, mrežni javni ključ i lokalni principal; pozivatelj ne predaje privatni ključ ni identitet koji bi mogao zamijeniti.
- Izvoz prima javni ključ konkretnog primatelja, frontier, rupe i granicu stranice. Uvoz koristi postojeći jedinstveni validacijski put.
- Integracijski test na izoliranim memorijskim bazama potvrđuje prijenos privatnog popisa nakon uspostavljenog povjerenja, idempotentno ponavljanje i odbijanje korupcije/krivog primatelja bez novih zapisa. Članstvo pošiljatelja prethodno se uspostavlja kao preduvjet uparivanja.
- Opaženo: `rtk cargo test -p list2cart-core --offline` prolazi 37 testova; `rtk cargo clippy --workspace --all-targets --offline -- -D warnings` prolazi; `cargo fmt --all` izvršen.
- S05 ostaje otvoren zbog identitetskih kolizija i cjelovitosti računa kroz više paketa. Ovaj API još nije izložen u Tauri/UI.
- Aktualni smjer korisnika: bez clouda i servera, lokalni privatni podaci. Ne uključivati planirano sastajalište/WebSync ni rezervne servere bez usklađivanja plana s tim zahtjevom. Postojeći mrežni prototipi nisu uklonjeni.

Sljedeće je zaključati S05–S07 prije uključivanja automatskog mrežnog prijema:

1. S05: dovršiti provjeru cjelovitosti računa kroz više paketa, veze i kolizije identiteta (bez spajanja trgovina/artikala samo po nazivu/barkodu) te povezati zaštićeni paket kroz javni `Database` API na jedinstveni prijem. Legacy uvoz još ima stara pravila spajanja i ne smije postati mrežni ulaz.
2. S06: potvrda raspona ne smije preskočiti odbijenu/nedostajuću verziju. Testirati više stranica, kasni stariji zapis, promjenu prava i snapshot nakon sažimanja; obrisani zapis ne smije oživjeti. Automatsko sažimanje uklonjeno je iz lokalnih mutacija i legacy uvoza dok taj ugovor nije zaključen; postojeći ručni nacrt nije dokaz sigurnog oporavka.
3. S07: obiteljski primatelj mora dobiti potrebne dokaze izvornog autora bez curenja drugih podataka. Zaključati delegirane/povijesne ovlasti i opozive te izvoz za konkretnog primatelja.

Trenutni QUIC test koristi prototipsku TLS konfiguraciju, a `pairing.rs` zasad računa SAS/commit, bez cjelovitog mrežnog postupka potvrde. Te funkcije ne predstavljati u UI-u kao dovršen WebSync.

Nisu mijenjani produkcijska baza, privatni ključevi ni goCOP izvor, nije objavljen servis i nisu rađeni commitovi. Zatečene izmjene ostale su sačuvane.


## Pretraga javnih cjenika i fleksibilan CSV — 17. 11. 2026.

- Dodan izravni dohvat poslovnica i dnevnih cjenika za Konzum, Lidl, Kaufland i Spar. Upit se pretražuje lokalno; ne šalju se računi ni pojmovi pretrage. Izvor/datum prikazani su uz rezultate, uz upozorenje za zastarjeli cjenik. Budući datumi su isključeni (trenutačno po UTC datumu). Predmemorija je samo u RAM-u, do četiri cjenika/30 min.
- Dodana pretraga poslovnica po gradu/adresi. Ovo nije univerzalno otkrivanje novih trgovačkih lanaca na internetu.
- Za bilo koju lokalno odabranu trgovinu može se otvoriti CSV/TSV: UTF-8 s/bez BOM-a, Windows-1250 te UTF-16 LE/BE s BOM-om; razdjelnici zarez, točka-zarez, tabulator i okomita crta; navodnici i višeredna polja. Zaglavlje je obvezno; najviše 12 MB i 100.000 podatkovnih redaka.
- Poznata jednoznačna zaglavlja daju prijedlog. Nepoznata/dvosmislena traže ručni izbor naziva i maloprodajne cijene u EUR; neobvezno barkod/marka. Prikazuje se uzorak i zahtijeva potvrda. Nema tvrdnje o podršci svakog proizvoljnog CSV-a, automatske konverzije valuta ni spremanja profila mapiranja. Nevaljane cijene/prazni nazivi izostavljaju se uz prethodnu obavijest u obrascu.
- Rezultat popunjava nacrt košarice ili kompatibilnog aktivnog popisa. Korisnik provjerava prodajnu jedinicu, količinu i cijenu; nema automatskog zapisa u bazu. Lokalni CSV i njegovo mapiranje nisu trajno spremljeni.
- Core: 54 testa prolaze; strogi workspace Clippy, fmt i frontend build prolaze. scraper je nadograđen s 0.24 na 0.29 zbog posrednog fxhash upozorenja; cargo-deny advisories/licenses/sources prolaze na lokalnoj RustSec bazi.
- Izolirani QA bundle uspješno izgrađen. Native UI provjera: nepoznati opis/iznos/ean stupci ručno odabrani; pretraga mlijeko vratila 1,29 EUR i barkod 0012345; nacrt košarice popunjen, košarica ostala prazna. Produkcijski profil nije korišten.
- Nakon nadogradnje parsera i scraper biblioteke ponovljen stvarni dohvat: Konzum 573, Lidl 116, Kaufland 53, Spar 144 poslovnice; sva četiri izvora od 17. 11. 2026. Pretraga mlijeko uspješno vratila rezultate za svaki lanac.


## S06 — prvi provjereni korak: potvrde i popuna rupa (17. 11. 2026.)

- Po korisničkom nalogu nastavljen je plan od S06. PLAN.md sada izričito odgađa serversko sastajalište/tunel (izvorni S12–S14) prema zahtjevu bez clouda/servera.
- Dva nova testa prije popravka pokazala su da prazna tvrdnja o pokrivenosti ili raspon s odbijenom verzijom mogu neopravdano pomaknuti potvrdu. Primljeni visoki UUID sada ostavlja rupu dok se ne potvrdi raspon; to vrijedi i za nezavisne zapise bez supersedes veze. Prazan početak rupe više se ne gubi pri spajanju.
- Odbijanje blokira potvrdu pripadnog toka za stranicu; drugi dopušteni kanali mogu napredovati. Kasno odbijena starija verzija ponovno otvara raspon. Potvrda zahtijeva prihvaćen potpisani završni zapis, dopušten kanal i odsutnost preklapanja s pošiljateljevom rupom. Privatne verzije filtrirane prije admissiona ne potvrđuju svoj tok. SQL greška pri potvrdi vraća i nov zapis/projekciju i sužavanje rupe.
- Zahtjev za starom rupom spušta početak kontinuirane delte, umjesto spajanja odvojene stare popune s novim dijelom uz prešućeni međuraspon. Test u više stranica potvrđuje dopunu svih 12 verzija i istu projekciju kada primatelj već ima najnoviji UUID.
- Referenca ugovora: goCOP HEAD fcca2c6bda5a2c43fa8b051994d4a5ad1a819e7e, peers/rupe.go (SHA256 d04219474a5bdd09c718d245abd16d3acdd4552331277dcf5e2c25fa3d3171b8), granica_odbijenih.go (47b0fa3633cf608f2af28ce6dcc90f519904d260699e9502ba4f2135e0cfdafc), granica_nakon_uvoza.go (c0b3b1db1d6521b6f6c0cd68e2471d9ad78d8d7fec628b52f7a2db806e3320c6). Prenesene su invariante; nisu kopirani Go izvori. List2Cart zasad konzervativno blokira cijeli tok stranice pri odbijanju i može ponovno slati poznate zapise; nema goCOP optimizacije preskakanja odbijenih.
- Širi test otkrio je postojeći povremeni UDP/QUIC kvar zatvaranja. Lokalni izvor quinn-proto 0.11.19 config/mod.rs i connection/packet_builder.rs potvrđuje zadani grease_quic_bit i nasumično uklanjanje fixed bita. Naš demux očekuje taj bit; endpoint ga sada eksplicitno ne pregovara. ADR-001 bilježi razlog. Nije promijenjen TLS identitet/šifriranje.
- Opaženo: 59 core + 24 sync = 83 workspace testa prolaze; strogi workspace Clippy i fmt check prolaze. UDP/QUIC scenarij nakon popravka prošao je pet uzastopnih ciljanih ponavljanja. Testovi su izolirani; produkcijska baza/ključevi i goCOP nisu mijenjani. Frontend nije mijenjan niti je ovaj backend korak dodatno UI-testiran.
- S06 NIJE cijeli dovršen: sljedeće točan serijalizirani bajtovni budžet (postojeća procjena nije tvrda granica), ograničene snapshot/reset stranice s dokazom potpunosti nakon sažimanja, restart/recovery scenariji i promjena prava. S07 kontrolni dokazi i ovlasti ostaju otvoreni. Nema automatskog LAN prijema.


## S06 — točan bajtovni budžet i granica oporavka (17. 11. 2026.)

- compute_delta_paged više ne koristi payload+signature+180 procjenu ni iznimku za prvi preveliki zapis. Broji stvarne UTF-8 JSON bajtove preko brojačkog Write izlaza, uključujući escape znakove, sve metapodatke, has_more i snapshot sadržaj. Prefiks bira binarnim traženjem; završna puna provjera veličine obvezna je prije povrata. Zadano 256 KiB, najviše 1 MiB; 0 bira zadano.
- ExportRequest.max_bytes ograničava cijeli zaštićeni plaintext, ne samo batch: stvarna veličina pošiljateljeva JSON omota rezervira se prije izrade stranice, a cijeli plaintext provjerava se prije šifriranja. Vanjski cipher/base64 omot nije ista granica.
- Prvi zapis ili snapshot/metapodaci koji ne stanu daju jasnu grešku; ne šalju se iznad granice niti se preskaču. Snapshot trenutno nije straničen: veći snapshot blokiran je uz poruku o potrebnom ograničenom oporavku, ne proglašava se riješenim.
- Detekcija reset potrebe sada uzima efektivni početak nakon zahtjeva za rupama. Obuhvaća uređaj bez frontier ključa i stare rupe ispod compaction_floor unatoč visokom MAX-u. Ne daje običnu delta pokrivenost preko sažete povijesti.
- Novi testovi: escape/Unicode i metapodaci, točna granica i jedan bajt manje, preveliki prvi zapis/mali header, snapshot ne zaobilazi budžet, novi uređaj i stara rupa pokreću recovery, dešifrirani protected paket uključuje wrapper u granicu. Postojeći višestranični test sada provjerava stvarnih <=1400 B za svaku stranicu.
- Opaženo: 62 core + 24 sync = 86 workspace testova prolazi; strict workspace Clippy i fmt check prolaze. UI i ovisnosti nisu mijenjani. Produkcijski profil nije korišten. Referentni ugovor pregledan u goCOP peers/ograde_razgovora.go; izvor nije kopiran/mijenjan.
- Sljedeće: ograničeni snapshot/reset kroz više stranica s dokazom potpunosti i trajnim oporavkom nakon restarta, zatim promjena prava/S07. Cijeli S06 još nije dovršen. Kandidati se i dalje učitavaju u memoriju prije odabira prefiksa; ovo je granica izlazne poruke, ne tvrdnja o streaming memorijskoj granici.


## S06 — trajni snapshot kroz više stranica za vlastite uređaje (17. 11. 2026.)

- Dodan recovery.rs i aditivna atomska migracija v4 za izlazne/ulazne snapshot sesije i stavke. Priprema zamrzava zadržane potpisane verzije dopuštenih kanala u jednoj transakciji; izvor s rupama ne može jamčiti snapshot. Manifest veže UUID, pošiljatelja/primatelja, kanale, frontier, broj/bajtove i SHA-256 uređenog sadržaja.
- Nove Database metode: start_snapshot_recovery, export_snapshot_page, import_snapshot_page, snapshot_progress, discard_snapshot_transfer. Podržani su samo aktivni vlastiti uređaji istog principala. Stranice koriste postojeći recipient-bound šifrirani/potpisani v2 omot, stvarni plaintext budžet (zadano 256 KiB, najviše 1 MiB) i uzastopni offset. Izvoz učitava kandidate za jednu stranicu, ne sve preostale verzije.
- Nedovršene stranice ne ulaze u poslovne projekcije. Zadnja stranica provjerava cjelinu, izvorne potpise, ovlasti i ovisnosti; primjena, granice/rupе i dovršenost atomarni su. apply_remote_versions dijeli internu transakcijsku primjenu s recoveryjem; postojeći javni API i dalje sam otvara/potvrđuje transakciju. Ponavljanje je idempotentno, neispravan redoslijed/manifest/sadržaj odbija se. Novije lokalne izmjene ostaju.
- Na završetku se uklanja samo izvedeni ulazni sadržaj; manifest i napredak ostaju. Izlazni snapshot zadržava se do eksplicitnog odbacivanja. Kvote: 8 nedovršenih prijenosa po smjeru, 100.000 verzija / 64 MiB JSON-a po snapshotu. Priprema/završna provjera još mogu držati cijeli snapshot u memoriji unutar te kvote.
- Test stvarno zatvara i ponovno otvara obje privremene baze uz zasebne sintetičke device ključeve; nakon sažimanja i arhiviranja 1 od 24 stavke dovršava isti snapshot u stranicama <=4096 B. Provjereni su ponovljena/preskočena/izmijenjena stranica, loš izvorni potpis uz točan hash i valjan vanjski potpis, rollback pri završnom SQL kvaru, očuvanje novije lokalne izmjene, opoziv između poziva, krivi primatelj, atomska priprema, kvote/odbacivanje, trajni konačni status te nastavak običnim zaštićenim delta paketom za promjene nastale nakon zamrzavanja. V4 migracija ima rollback/ponovljivost test.
- Opaženo: 66 core + 24 sync = 90 workspace testova prolazi; strict workspace Clippy i fmt check prolaze. Nema novih ovisnosti, UI promjena, produkcijske baze/ključeva ni commita. Ovaj korak nije UI/native testiran.
- Ograničenje: novi snapshot put je eksplicitni core API, nije automatski fallback stare compute_delta_paged funkcije niti povezan u Tauri/UI/mrežni razgovor. Obiteljska prava ostaju S07. S06 još zahtijeva povezivanje recovery zahtjeva/napretka, kasne starije uspješne dopune između razgovora i širu matricu promjena prava. Ne proglasiti cijeli S06 ili LAN dovršenim. Dokaz manifesta odnosi se na deklarirani zamrznuti skup, ne na svu povijest nepoznatu izvoru.


## S06 — automatski odabir delta/snapshot i potpisani nastavak (17. 11. 2026.)

- Dodan exchange.rs i Database::sync_request / sync_reply / import_sync_page. Zahtjev veže oba javna ključa, vrijeme/nonce, potvrđenu granicu, rupe i najnoviji trajan snapshot napredak Ed25519 potpisom. Ograničen je na 64 KiB i aktivne vlastite uređaje istog principala; provjeravaju se primatelj, potpis, vrijeme ±600 s i duljine metadata. Nema privatnih ključeva u DTO-u. Ugovor je dokumentiran u PROTOCOL.md kao razvojni sync_request/1.
- recovery_keys je zajednička odluka za običnu deltu i dispatcher. Nedostatak sažete povijesti automatski priprema snapshot; aktivan prijenos nastavlja se iz potpisanog offseta. Izgubljena prva poruka/restart izvora ponovno koristi isti zamrznuti izlaz. Izgubljeno izlazno stanje započinje novi snapshot; noviji dovršeni ulaz ima prednost nad starijim djelomičnim. Proizvoljna greška (npr. premala obična stranica) nije povod za snapshot.
- Izvoz zadnje stranice ne označava primitak. Tek primateljev idući potpisani complete zahtjev potvrđuje izvorov izlaz i atomarno uklanja izvedene izlazne stavke. SQL greška čuva kopiju za retry. Nakon potvrde odgovor je ponovno delta za promjene nastale nakon zamrzavanja. Tipizirani ishod prima se iz autentificiranog sadržaja; stari monolitni reset ne prolazi automatskim putem.
- Novi testovi rade samo preko zajedničkog request/reply/import toka: obična delta bez recovery sesije, automatski snapshot nakon sažimanja, izgubljena prva poruka, stvarno ponovno otvaranje obje privremene baze s testnim ključevima, nastavak istog ID-a, ACK rollback, povratak na deltu i naknadne nove podatke. Dodatno izgubljen izlaz i napušten stariji ulaz, krivotvoren frontier/napredak/potpis, krivi primatelj, ekstremno staro vrijeme, opozvani uređaj, self-sync i odbijanje starog reset formata bez promjene baze.
- Opaženo: 69 core + 24 sync = 93 workspace testa prolazi; strict workspace Clippy i fmt check prolaze. Nema promjena ovisnosti, frontend/Tauri UI-a, produkcijskih podataka/ključeva ni commita.
- Sljedeće u S06: kasne starije uspješne dopune između razgovora (da MAX ne sakrije novi backfill), zatim šira matrica promjena prava/S07. Novi core API nije povezan s mrežnim razgovorom S09 ni UI-jem; automatski LAN nije dovršen. Uparivanje/TLS/S09 ostaju iza zaključivanja podatkovnih uvjeta. Stari eksplicitni delta/snapshot API-ji zadržani su. Vremenska ograda zahtjeva nije trajni one-shot replay cache; ponavljanje unutar roka smije ponoviti stranicu, a zastarjeli nastavak već oslobođenog izlaza može pokrenuti novu konzervativnu obnovu.


## S06 — naknadno primljene starije verzije više ne skriva MAX (17. 11. 2026.)

- Novi test s tri izolirana uređaja prvo je pao: posrednik je naknadno primio neovisni stariji zapis, MAX se nije promijenio, a treći uređaj zapis nije dobio. Nakon popravka isti scenarij prolazi, uključujući drugi stariji zapis primljen tijekom već zamrznutog snapshota.
- Dodan backfill.rs i atomska migracija v5: sync_backfill_revisions (izvorno lokalna oznaka po kanalu) i sync_backfill_ack (primateljeva potvrda po ključu izvora/kanalu). Svaka NOVA uspješno primljena potpisana verzija mijenja UUID oznaku kanala u istoj transakciji; duplikat ne mijenja ništa. Markiraju se svi novi remote prijemi jer čak i zapis iznad lokalnog MAX-a posrednika može biti ispod MAX-a trećeg primatelja. Izvorne lokalne naredbe i dalje koriste delte. Cijena konzervativne sigurnosti: prosljeđivanje novoprimljenih podataka može ponoviti cijeli ograničeni snapshot.
- Migracija v5 jednom označava već postojeće potpisane kanale radi prethodno prešućenih dopuna; idempotentni prolaz ne mijenja oznake. Dvije tablice/verzija/seed imaju transakcijski rollback. Nema produkcijske migracije u ovom koraku.
- SnapshotManifest opcionalno nosi zamrznute backfill_revisions. Tek završni commit sprema izvornu potvrdu kod primatelja. Potvrda ne kopira token u njegov vlastiti inventar. UUID se uspoređuje po jednakosti, ne kao rastući sat; prazna vrijednost označava odsutnost. Novija dopuna tijekom prijenosa ne potvrđuje se starim snapshotom: slijedi novi oporavak. Primateljeve novije verzije ostaju.
- Potpisani SyncRequest sada je razvojni format 2/prefiks list2cart-request-v2|; kanonski niz dodaje backfill_revisions nakon recovery. V1 se odbija. Scope i ukupne kvote poruke ostaju. Dispatcher uspoređuje samo dopuštene kanale; mismatch pokreće postojeći trajni snapshot put bez nove mrežne infrastrukture.
- Testovi: donor → posrednik → treći uređaj s nepromijenjenim MAX-om; posrednikov manji MAX od primateljeva; dvije dopune preko zamrznutog snapshota; stvarni disk reopen posrednika; duplikat bez ponovne obnove; source marker SQL fault vraća i novi zapis; receiver ACK SQL fault vraća projekcije i potvrde; validna novija stavka na primatelju očuvana; krivotvorene oznake i stari request format odbijeni; v5 rollback i jednokratni upgrade seed/ponovljivost.
- Opaženo: 73 core + 24 sync = 97 workspace testova prolazi; strict workspace Clippy i fmt check prolaze. Nisu mijenjani frontend, ovisnosti, produkcijska baza/ključevi ni goCOP. Nema commita ni UI/native runtime provjere ovog backend koraka. Referentna invarianta: goCOP peers/granica_nakon_uvoza.go; vlastita konzervativna prilagodba, bez kopiranja izvora.
- Sljedeće: šira matrica promjene prava/ponovno dopuštenih kanala i S07 kontrolni dokazi/obiteljsko dijeljenje. Cijeli S06/S07 sigurnosni audit nije proglašen dovršenim. API je i dalje samo za vlastite uređaje; S09 razgovor/LAN i Tauri/UI nisu povezani. Generički peer-specific replay cache i precizno slanje samo kasnih verzija nisu uvedeni.
