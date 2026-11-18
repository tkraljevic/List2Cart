# List2Cart: plan funkcionalne sinkronizacije i WebSynca

Datum: 17. 11. 2026. Status: prijedlog izvedbe za implementacijskog agenta; nije opis dovršenih funkcija.

**Aktualni opseg (17. 11. 2026.):** korisnik traži aplikaciju bez vlastitog clouda i pomoćnih servera. Rad nastavlja po ovisnostima S06 → S07 → sigurno uparivanje/razgovor/LAN, uz lokalne zaštićene pakete. Sastajalište, posrednički tunel i serverski rezervni put iz izvorne F3 nisu odobreni dio aktualne izvedbe; S12–S14 ostaju odgođena izvorna specifikacija. Ne uključivati te servise automatski niti ih uvjetovati za rad aplikacije. Status izvedbe i preostali uvjeti su u STATUS.md.

Počni ovdje. Izvršivi zadaci su u [AGENT_TASKS.md](AGENT_TASKS.md), a mapa Go → Rust i zamke portiranja u [GOCOP_RUST_CHEATSHEET.md](GOCOP_RUST_CHEATSHEET.md).

## 1. Cilj i granica kompatibilnosti

Izgraditi izvorni Rust sustav razmjene po uzoru na goCOP: rad bez mreže, sigurno uparivanje, automatska LAN razmjena, izravna internetska razmjena preko WebSynca, rezervni put kad izravna veza ne uspije i ručni prijenos paketa. Svaki put mora koristiti isti mehanizam validacije i primjene promjena.

List2Cart ima vlastiti protokol i vlastitu mrežu povjerenja. Ne uparuje se s produkcijskim goCOP čvorovima niti šalje podatke u njihovu mrežu. Go služi kao referentna implementacija transporta i testni uzorak. Ne obećavati kompatibilnost domenskih podataka dviju aplikacija. Ne uvoditi Go sidecar u konačni proizvod; mali Go testni program dopušten je samo u izoliranom razvojnom okruženju.

Portirati ugovore i provjereno ponašanje, a ne doslovno sve datoteke. COP karte, vodostaji, sektori, izvještaji i organizacijske uloge nisu List2Cart funkcije.

## 2. Polazište koje agent mora ponovno provjeriti

Referentni goCOP HEAD tijekom pregleda: `fcca2c6bda5a2c43fa8b051994d4a5ad1a819e7e`.
List2Cart HEAD: `19ca8efd6ad0ad6791d579f875ce217dcf23467e`, uz postojeće necommitirane promjene.

Radna kopija mijenjala se tijekom analize. Aktualno su uočeni `SyncBundle.receipts`, `export_sync_bundle_filtered`, `shared_lists_allows`, transakcijski uvoz i spajanje postojećih stavki popisa. To poništava raniji nalaz da ti dijelovi uopće ne postoje; nije dokaz njihove potpune ispravnosti. `verify_peer_identity` još dopušta povrat na ulogu iz samog paketa. SAS iz datoteke i samoprijavljena uloga nisu dokaz identiteta.

Postoje UUIDv7 verzije, frontier, delta i sažimanje knjige. Još treba dokazati potpun zapis svih promjena, jedinstvenu projekciju iz verzija, ovlasti po primatelju, oporavak nakon rupa i mrežni transport. Ne graditi novu arhitekturu na pretpostavci da je postojeći uvoz konačno rješenje.

Prethodne provjere unutar razgovora: frontend build, 11 Rust testova i formatiranje prošli; strogi workspace Clippy pao na `upsert_product` s previše argumenata. Zbog naknadnih izmjena to je povijesni baseline, ne aktualni rezultat. Agent prije rada bilježi novi baseline i čuva tuđe izmjene.

## 3. Predložena struktura

```text
crates/list2cart-core/              postojeća domena, SQLite, projekcije, migracije
crates/list2cart-sync/              dodati postupno, tek uz izvršivu funkciju
  src/identity.rs                  ključevi i identitet
  src/trust.rs                     članstvo, grantovi, opozivi
  src/pairing.rs                   SAS i obostrana potvrda
  src/wire/                       DTO, potpisani bajtovi, okviri, ograničenja
  src/transport/                  LAN TLS, UDP demux, signal, QUIC, tunel
  src/session.rs                  frontier → delta → primjena → sadržaj → potvrda
  src/service.rs                  raspoređivanje, stanje, izbor puta
src-tauri/                         konfiguracija, lifecycle, commands/events
src/                               uparivanje, odabir dijeljenja i status
```

Ovo je raspodjela odgovornosti, ne nalog za stvaranje praznih modula. Transport ne smije ovisiti o Tauri sučelju ni poznavati strukturu računa. Sinkronizacijska jezgra treba biti dostupna headless testu i kasnijem programu za sastajalište. Dodati zaseban binarni paket sastajališta kad njegov prvi end-to-end scenarij postoji.

SQLite upise obavlja jedan kontrolirani izvršitelj ili kratke sinkrone operacije izvan async mrežnog toka. Ne držati `Mutex<Database>` ni transakciju preko `.await`. Mrežni dio dobiva ograničen snapshot/deltu; upis se obavlja odvojenom naredbom i potvrđuje nakon commita.

## 4. Pravila podataka

1. Lokalna promjena i njezina verzija spremaju se u istoj transakciji. Nema uspješnog povratka nakon propuštenog zapisa u knjigu.
2. Primljena verzija čuva izvorni ID, autora, kanal i vrijeme. Ne prolazi kroz lokalni create/upsert koji stvara novu promjenu i ponovno je šalje.
3. Lokalni SQLite integer ID nije mrežni identifikator. Veze među podacima putuju preko stabilnih `sync_id` vrijednosti; na prijemu se mapiraju na lokalne ključeve.
4. Popis i svaka stavka popisa zasebne su cjeline. Istodobno dodavanje različitih stavki ne smije završiti zamjenom cijelog popisa i gubitkom jedne od njih.
5. Preporuka za v1: deterministički odabir pobjedničke verzije po validiranom UUIDv7 redoslijedu za istu cjelinu; konkurentne verzije ostaju u povijesti. Ovo nije spajanje proizvoljnih polja niti semantika stvarnog vremena. Posebno definirati konkurenciju brisanja i izmjene, arhiviranje roditelja i eksplicitnu obnovu. Test mora zaključati odabrano pravilo prije implementacije.
6. Brisanje je tombstone. Sažimanje nikad ne smije omogućiti povrat obrisanih zapisa preko starog uređaja. Snapshot/reset protokol mora pokriti uređaj stariji od dostupne povijesti.
7. Primitak najveće verzije ne dokazuje da su stigle sve ranije. Frontier i evidencija pokrivenosti/rupa čuvaju se po autoru i kanalu, u okviru dopuštenog skupa podataka. Odvojiti lokalno primljeno od potvrđenog znanja pojedinog partnera.
8. Ugovor uključuje račune i njihove stavke, katalog, trgovine, povijest cijena, popise, stavke i reference na fotografije. Lokalne postavke uređaja, privatni ključevi, cache i mrežne sesije ne razmjenjuju se.
9. Brojevi, količine, cijene, jedinice, ukupni iznosi i veze validiraju se na Rust granici. Derivirani zbrojevi računaju se iz primljenih validnih stavki. Model ne smije ovisiti o float prikazu JSON-a za potpis ili identitet verzije.

## 5. Povjerenje i obiteljska privatnost

Uspješno TLS povezivanje potvrđuje posjedovanje ključa; članstvo određuje pripadnost mreži; zasebna prava određuju što se smije čitati i mijenjati. To su tri različite provjere.

Predloženi List2Cart kanali:

| Kanal | Sadržaj | Pristup |
|---|---|---|
| `private:<principal_id>` | računi, privatni katalog/cijene i privatni popisi | samo provjereni vlastiti uređaji iste osobe |
| `list:<list_sync_id>` | objavljeni zajednički popis i njegove stavke | točno određeni članovi, zasebno read/write |
| kontrolni zapisi | članstva, grantovi, opozivi, inačice ugovora | samo nužni dio za provjeru pripadnog odnosa |

Ovo je List2Cart prilagodba; goCOP sektorske kanale ne kopirati. Stabilni identitet osobe povezuje njezine uređaje. Odabir `own_device` u UI-u nije dovoljan da se novi ključ veže uz tu osobu: to mora potvrditi ovlašteni uređaj. Član obitelji ne smije sam sebi dodijeliti pravo na privatni kanal.

Grantovi se odnose na stabilne ID-eve popisa i ključeve/osobe, ne nazive i slobodni tekst. Isti naziv dvaju popisa ne daje isto pravo. Izvoz je za konkretnog primatelja; unija prava svih članova nije dozvola jednog člana. Filtrirati podatke, verzije, frontier i sadržaj u oba smjera. Prijenos preko trećeg uređaja ne smije proširiti ovlasti.

Za sigurno prosljeđivanje preko više uređaja predložiti i specificirati potpisani omot verzije s izvornim autorom, kanalom i referencom na ovlast. To je List2Cart dodatak: pregledani goCOP `Version` nije dokaz takvog potpisa. Prije izvedbe zamrznuti potpisane bajtove i pravilo provjere povijesnih ovlasti; mTLS trenutnog pošiljatelja sam ne dokazuje autora proslijeđenog zapisa.

Definirati granice opoziva: zaustavlja buduće dopuštene razmjene čim čvor sazna za opoziv; ne može obrisati već preuzete podatke s tuđeg uređaja. Offline čvor još ne zna za novi opoziv. Kontrolne zapise primijeniti prije domenskih zapisa u istom paketu. Nesigurnu ili nepoznatu ovlast odbiti bez potvrde uspješne primjene. Za neodrediv odnos stare verzije i opoziva primijeniti dokumentirano konzervativno pravilo, ne vjerovati vremenu koje autor sam tvrdi.

## 6. Kako se odvija jedna razmjena

```text
odabir puta → dokaz očekivanog ključa → članstvo i prava
→ Hello (protokol, schema, podržane mogućnosti)
→ dopušteni kanali + frontier/pokrivenost + zahtjevi za rupama
→ ograničene delte s oznakom nastavka i pokrivenih raspona
→ validacija, provjera ovlasti, transakcijski upis + projekcija
→ zahtjevi za nedostajućim fotografijama → sadržaj s provjerom hasha
→ potvrda stvarno primijenjenih verzija/sadržaja
→ nastavak dok ima delti ili zabilježena odgoda s razlogom
```

Paket iz datoteke ulazi u isti validacijski i podatkovni put. Novi potpisani format razlikovati od starog `.list2cart.json`; nepodržanu verziju odbiti bez promjene baze. Stari nepotpisani paket ostaje eksplicitni lokalni uvoz, bez automatskog stjecanja članstva ili prava. Ne tumačiti njegov SAS kao lozinku. Izvoz paketa za drugog člana veže se uz njegov ključ i dopušteni opseg; definirati šifriranje osjetljivog sadržaja na disku prije distribucije novog formata.

Jedan protokol koristi LAN TLS, QUIC i tunel. Izbor puta po goCOP uzoru: prvo LAN; zatim živa provjerena WebSync sesija; zatim konfigurirane izravne/server/VPN/tunelske veze, uz pripremu novog WebSynca u pozadini. Identitet se na svakom putu provjerava jednako. Pokvarena sesija ne smije blokirati rezervni put.

## 7. WebSync infrastruktura i izvedbeni rizik

Sastajalište služi prijavi dostupnosti i prosljeđivanju signalnih poruka. Nije središnja baza računa niti red za pohranu izmjena dok je drugi uređaj ugašen. STUN otkriva javnu adresu; ne prenosi poslovne podatke. Uređaji trebaju biti istodobno dostupni ili razmjena ide kasnije preko drugog dopuštenog uređaja/paketa.

Izravni WebSync ne prolazi kroz svaki NAT/vatrozid. Za cilj usporediv s goCOP treba i rezervni server/tunelski put. U tunelu podaci ostaju zaštićeni međusobno autentificiranim TLS-om između uređaja; obični HTTPS do posrednika nije dovoljna zamjena. Sastajalište vidi određene metapodatke povezivanja. README mora razlikovati lokalnu pohranu podataka od korištenja pomoćnih mrežnih usluga.

Najveći rizik porta: goCOP `quic.Transport.ReadNonQUICPacket` dijeli jednu UDP utičnicu sa STUN-om i udarcima. Ne pretpostaviti isti API u Quinnu. Rani prototip mora dokazati jedan vlasnički receive loop, razvrstavanje paketa i slanje s iste utičnice/porta, uključujući QUIC metapodatke i backpressure. Dvije niti koje zasebno čitaju isti socket nisu rješenje. Ako prototip ne prolazi, zapisati nalaz i usporediti održivu alternativu prije nastavka F3.

## 8. Faze i izlazni uvjeti

| Faza | Zadaci | Dokaz koji otključava sljedeću fazu |
|---|---|---|
| F0: ugovor i rizici | S00–S02 | aktualni baseline, specifikacija i uspješan mali UDP/QUIC prototip |
| F1: podatkovna ispravnost | S03–S07 | tri izolirane baze konvergiraju preko paketa; ACL, brisanja i rupe prolaze |
| F2: sigurna lokalna mreža | S08–S10 | dva uređaja stvarno uparena, LAN automatski razmjenjuje iste podatke |
| F3: internet | S11–S14 | signal → NAT → QUIC → isti protokol, uz dokaz rezervnog puta kad UDP ne prolazi |
| F4: proizvod i platforme | S15–S17 | stvarni uređaji, lifecycle, migracija, dijagnostika, regresije i release provjere |

Rani mrežni prototip radi na sintetičkim bajtovima; ne zaobilazi F1. S08 može početi nakon zamrznutog ugovora, ali se LAN razmjena podataka ne proglašava gotovom prije F1. Nema obveze paralelnog rada agenata; zadaci su izvedivi redom.

## 9. Kada je cilj postignut

- A i B mijenjaju podatke offline; nakon razmjene sve dopuštene projekcije jednake su bez obzira na redoslijed poruka i ponavljanje.
- C, koji komunicira samo s B, dobiva dopuštene A-ove promjene uz provjerljiv izvor. Ne dobiva privatne podatke izvan svojih prava.
- Promjena jedne stavke ne briše tuđu novu stavku. Brisanje ostaje brisanje nakon povratka starog uređaja i nakon sažimanja.
- Prekid nakon commita, prije potvrde, rješava se ponavljanjem bez duplikata. Prekid prije commita ne ostavlja djelomičnu projekciju.
- Promjena prava, ponovno dopušten kanal, stranica delte i kasno stigla verzija ne stvaraju trajnu rupu.
- Zamjena ključa, krivi SAS, opozvani uređaj, lažna uloga, prevelika poruka i korumpirana fotografija imaju provjereno odbijanje.
- LAN, WebSync i rezervni put provjereni su na stvarnim mrežama. iOS/Android izvještaj jasno razlikuje rad u prvom planu od dopuštenog pozadinskog rada. Ne obećavati stalno slušanje dok OS suspendira aplikaciju.
- Nema pristupa pravim bazama, ključevima ili mrežama iz automatiziranih testova. Demo/Test i produkcija imaju različite identitete i putanje.
- Upute pokrivaju samostalno podizanje sastajališta i rezervnog puta. Objavljivanje javnog servisa nije dio automatske implementacije.

## 10. Što nije potrebno za prvi pouzdani sustav

Nije potreban opći CRDT framework, cloud račun za korisnika, kopiranje cijelog goCOP-a ni prepisivanje ostatka UI-a. PocketBase put ne proširivati kao drugi autoritativni mehanizam spajanja. Postojeću funkciju ne uklanjati prije migracijskog plana; nakon prijelaza jasno odvojiti ili povući legacy put kako ne bi izvan knjige stvarao promjene.

Trošak i trajanje procijeniti nakon S00–S02. Bez dokazanog UDP adaptera, pravila autorizacije i mobilnog prototipa precizna procjena bila bi nagađanje.
