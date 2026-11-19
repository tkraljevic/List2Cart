# Kontrola modula drugog agenta — 19.11.2026.

Opseg: family, pairing, sync, about, family-sync.css, njihovi testovi te povezivanje u main.ts i relevantna semantika LAN servisa. Izvorni pregled: kod i fokusirane render provjere. Nakon odobrenja korisnika sva tri nalaza popravljena u zasebnom zahvatu, opisanom na kraju.

## Nalazi

### P1 — Pauza obećava potpun prestanak slanja koji backend ne jamči

src/ui/family/index.ts:125–128 kaže da se lokalne promjene ne šalju dok se razmjena ne nastavi. paused se u crates/list2cart-core/src/lan_service.rs:275 koristi za automatski coordinator, ali listener (oko 200) nema tu provjeru, a sync_peer_at (375) također nije blokiran. Gumb Razmijeni sada u renderPeerControls ne provjerava paused.

Fokusirani Bun probe potvrdio: pausedPromise=true i manualSyncEnabled=true za paused=true/lanBusy=false. To nije dokaz mrežnog testa dvaju uređaja, ali kod jasno ne provodi obećanje iz poruke. Predloženo: UI jasno nazvati pauzom automatskog pokretanja i navesti da izravna/ručna razmjena ostaje moguća, ili zasebno implementirati potpunu pauzu prijenosa i testirati je u oba smjera. Ne mijenjati backend samo zbog teksta bez dogovora o semantici.

### P2 — Odustajanje ne otkazuje aktivno uparivanje

Novi prikaz src/ui/pairing/index.ts:36 koristi data-close-dialog s oznakom Odustani od povezivanja. Host u src/main.ts:1138–1145 samo postavi pairingOpen=false, bez stopPairListening ili odbijanja pending sesije. Tijekom waiting=true listener može ostati aktivan nakon zatvaranja; tijekom pending=true sesija se ne odbija. Nadalje, u pairingBusy stanju i zatvaranje (36) i odbijanje (114) su onemogućeni, pa tijekom čekanja potvrde nema dostupnog odustajanja.

Ovo je problem integracije s postojećim handlerom i nedovršene semantike otkazivanja, ne samo pogreška novog renderera. Bez odobrenja koda ne slijedi da se pristup automatski daje; nalaz se odnosi na otkazivanje i aktivnost sesije. Potrebno povezati zatvaranje sa stvarnim otkazivanjem odgovarajuće faze i provjeriti na dva uređaja.

### P2 — pairingBusy se pogrešno tumači kao već poslana pozitivna potvrda

src/ui/pairing/index.ts:98–102 za svako busy=true uz pending=true prikazuje Vaša potvrda je zaprimljena i čekanje drugog uređaja. Isti flag main.ts:2053 postavlja pri odbijanju (confirmPairPeer approved:false), a main.ts:2020 ga postavlja prije izvršavanja pozitivne native naredbe. Kod odbijanja će se zato kratko prikazati da je korisnik potvrdio povezivanje; osim toga taj boolean ne potvrđuje da je native naredba već zaprimljena.

Probe je potvrdio render poruke uz generički busy=true. Potrebno odvojiti radnje/prijelaze (npr. potvrđivanje, odbijanje, stvarno čekanje) ili prikazivati neutralnu poruku obrade dok backend ne daje precizniju potvrdu. Postojeći test samo očekuje tu poruku i ne testira odbijanje.

## Što je dobro

- Moduli primaju stanje kroz ugovor i nemaju paralelni store/API.
- CSS je odvojen, s fs- prefiksom; drugi agent nije morao mijenjati zajednički okvir.
- Provjereni korisnički/mrežni stringovi u imenima, kodovima i adresama escapaju se prije HTML interpolacije.
- Nema izmišljene read/write kontrole; podijeljeni popisi prikazani su kao pregled.
- Duplikati nisu lažno predstavljeni kao spajanje polja/povijesti.
- About prima verziju i licencu od hosta; EUPL nije zamijenjen MIT licencom syncNeta.

## Provjera i granice

npm run build prolazi. bun test tools src/ui: 21/21 test prolazi (11 pripada modulima drugog agenta); git diff --check prolazi. Probe /private/tmp/review-agent-ui.ts reproducira proturječnu paused poruku i generičko busy stanje.

Testovi uglavnom provjeravaju generirani HTML i ne pokrivaju otkazivanje aktivne sesije ni ponašanje pauze preko mreže. Stvarno uparivanje/sinkronizacija dva uređaja nije izvršena u ovoj kontroli. Izvorni nalazi popravljeni su nakon ovog pregleda; vidi završnu provjeru ispod.


## Popravci i završna provjera

- P1: poruka i gumb sada izričito govore o pauzi automatskog pokretanja. Dolazna, ručna i već započeta razmjena ostaju mogući; za prekid LAN servisa jasno je naveden Zaustavi LAN. Semantika LAN protokola nije tiho promijenjena.
- P2 otkazivanje: close i Odustani sada pozivaju stopPairListening prije zatvaranja. Stop odbija pending sesiju i zatvara listener koristeći postojeći slot; PairingController dodatno prekida već započeti confirm pomoću CancellationToken-a. Zapisivanje konačnog članstva i otkazivanje imaju usklađen redoslijed kroz zajednički mutex. Ne dodaju se nove ovisnosti niti mijenja syncNet paket.
- Frontend koristi pairingOperation za odbacivanje zakašnjelih rezultata potvrde/pollinga i pairingAction za zaseban prikaz confirming/cancelling. Odustajanje ostaje dostupno tijekom potvrđivanja; ponovljeni klik tijekom prekidanja je blokiran. Pogreška native prekida ostavlja dijalog otvorenim uz jasnu poruku.
- P2 poruke: nema tvrdnje Vaša potvrda je zaprimljena na temelju generičkog busy flaga. Obrada i prekidanje imaju odvojene poruke.
- Poslana pozitivna potvrda ne može se povući zatvaranjem veze. Sučelje to navodi i nakon prekida osvježava stvarnu evidenciju članova, umjesto da pretpostavi da povezivanja nije bilo.

Provjere: npm run build, cargo check -p list2cart --offline i git diff --check prolaze; bun test tools src/ui: 23/23; cargo test --workspace --offline: 119/119 Rust testova. Četiri nova native testa koriste dva stvarna lokalna TLS partnera: zatvaranje čekajućeg porta, odbijanje pending partnera bez članstva, prekid confirm operacije i uspješan normalan obostrani confirm. Testovi otvaranja lokalnih portova pokrenuti su izvan mrežnog sandbox ograničenja nakon početnog EPERM.

Ovo su lokalni procesi na istom računalu, ne ručna provjera na dva fizička telefona. Nema commita, pusha ili releasea.
