# List2Cart Sinkronizacijski Protokol v1 (`PROTOCOL.md`)

Status: izvršivi razvojni ugovor v1 (`S02`), datum: 2026-11-17. Nije još produkcijski mrežni protokol: listener, TLS policy, životni ciklus i recovery zahtijevaju preostale faze. Potpisani format izmijenjen je prije prvog mrežnog izdanja; stari prototipski potpisi nisu kompatibilni.

## 1. Identitet protokola i TLS/QUIC oznake

| Svojstvo | Produkcijska vrijednost | Testna simulacija | Namjerna razlika od goCOP-a |
|---|---|---|---|
| Ime protokola | `list2cart` | `list2cart-sim` | Vlastiti prostor imena; ne komunicira s goCOP mrežom |
| X.509 CommonName | `list2cart-sync` | `list2cart-sim-sync` | `<protokol>-sync` |
| SAN URI vjerodajnica | `urn:list2cart:vjerodajnice:<base64url>` | jednako (raw URL-safe base64 bez paddinga) | Zamjenjuje `gocop:vjerodajnice:` |
| ALPN LAN TLS | `list2cart-lan/1` | jednako | Odvojeni ALPN za LAN razmjenu |
| ALPN WebSync QUIC | `list2cart-web/1` | jednako | `<protokol>-web/1` |
| ALPN Signal TLS | `list2cart-sig/1` | jednako | Signalni kanal nad bajtnim tokom |
| ALPN Rezervni tunel | `list2cart-tun/1` | jednako | End-to-end TLS unutar WS tunela |
| Udarac (NAT punch) info | `list2cart-udarac-v1` | `list2cart-sim-udarac-v1` | HKDF-SHA256 info prefiks |
| Signal potpis prefiks | `list2cart-<vrsta>-v1\|` | `list2cart-sim-<vrsta>-v1\|` | Potpis nad izvornim bajtovima tijela |
| SAS commit prefiks | `list2cart-pair-commit-v2\|` | `list2cart-sim-pair-commit-v2\|` | V2 commit–reveal |
| SAS code prefiks | `list2cart-pair-v2\|` | `list2cart-sim-pair-v2\|` | 6-znamenkasti kod s vodećim nulama |

## 2. Ključevi i certifikati

- **Identitet uređaja**: Ed25519 ključ (32 B javni, PKCS#8 PEM privatni ključ pohranjen izvan SQLite baze uz dozvole `0600`).
- Lokalna baza `list2cart.db` koristi odvojene datoteke `list2cart.db.device.key` i `list2cart.db.network.key`; ključevi nisu u poslovnim payloadima, paketima ni SQLite backupu. Korijenski ključ mreže postoji samo kod administratora. Kopija baze s vezanim fingerprintom i bez odgovarajućeg device ključa zahtijeva oporavak/onboarding; aplikacija ne generira zamjenski identitet postojećeg člana.
- **Žičani i bazni prikaz javnog ključa**: standardni Base64 s paddingom (`RFC 4648`, 44 znaka za 32 bajta).
- **X.509 certifikat**: samopotpisani certifikat koji dokazuje posjedovanje privatnog ključa tijekom TLS 1.3 rukovanja. Povjerenje ne dolazi od CA niti od CN-a nego od:
  1. u pairing načinu: ručno uspoređenog 6-znamenkastog commit–reveal SAS koda,
  2. u normalnoj razmjeni: očekivanog javnog ključa i potpisanog lanca članstva (`NetworkKey` → optional `Ovlast` → `Membership` → `ListGrant`).
- **Odbijanje drugog protokola**: svaka veza čiji certifikat nije Ed25519 ili čiji `CN` ne odgovara `<protokol>-sync` odbija se odmah na provjeri certifikata.

## 3. Kanali i dopušteni entiteti

| Kanal | Entiteti | Tko smije čitati | Tko smije pisati |
|---|---|---|---|
| `private:<principal_id>` | `stores`, `products`, `price_history`, `receipts`, `receipt_items`, privatni `shopping_lists` i `planned_items` | Samo uređaji istog `principal_id` s važećim `OwnDevice` članstvom | Samo ovlašteni uređaji istog `principal_id` |
| `list:<list_sync_id>` | Zajednički `shopping_lists` (`entity_id == list_sync_id`) i njegovi `planned_items` (`list_sync_id == ID iz kanala`) | Članovi s važećim `ListGrant` (`Read` ili `Write`); mrežni korijenski ključ ima administrativni pristup | Članovi s `ListGrant(Write)` i točnim `grant_ref`; mrežni korijenski ključ |
| `control` | `membership`, `ovlast`, `list_grant`, `revocation` | Svi aktivni članovi (samo zapisi nužni za provjeru njihovih odnosa) | Administrator mreže / ovlašteni izdavatelj |

## 4. Kanonski potpisani bajtovi i JSON pravila

1. Svi `[]byte` nizovi u JSON-u kodiraju se isključivo kao standardni Base64 string (`base64_serde` adapter), nikada kao JSON niz brojeva.
2. Potpisi kontrolnih zapisa (`Membership`, `Ovlast`, `ListGrant`, `Revocation`) računaju se nad eksplicitnim kanonskim strukturama s fiksnim redoslijedom polja i cijelim Unix sekundama (`i64`), nikada nad lokalnom vremenskom zonom ili proizvoljnim JSON formatiranjem.
3. **Potpisani omot verzije (`SignedVersion`)**:
   - Kanonski bajtovi potpisa su ASCII prefiks `list2cart-ver-v1|` i kompaktni JSON niz u ovom točnom redoslijedu:
     ```text
     [schema_version, entity, entity_id, version_id, node_id, supersedes,
      archived, channel, created_at, created_at_unix, author_pub_key, grant_ref, payload]
     ```
   - Brojevi su decimalni cijeli brojevi, `archived` je JSON boolean, ostalo su stringovi. Nema razmaka ni završnog novog retka. UTF-8 stringovi se kodiraju kao `serde_json` stringovi (HTML znakovi i Unicode se ne pretvaraju automatski u ASCII escape zapise); navodnici, backslash i kontrolni znakovi se escapeaju. Potpis štiti sva polja osim samog `signature`. `payload` je točno izvorni JSON tekst unutar potpisanog stringa, bez ponovnog parsiranja/preformatiranja. Potpis se stoga ne može prenijeti na drugi node, datum ili razdiobu polja koja sadrže `|`.
   - Cijene i iznosi su isključivo cjelobrojni centi (`i64`) od 0 do `9007199254740991`, kako bi prikaz/IPC u JavaScriptu bio točan. Razlika računa smije biti negativna. Količine u sync payloadu su cijeli tisućinci (`qty_millis: i64`) od 1 do `1000000000000`; lokalni unos prihvaća najviše tri decimale. Za `kom`/`kg`/`L` iznos je `(qty_millis * price_cents + 500) / 1000`; za `g`/`ml` je `(qty_millis * price_cents + 500000) / 1000000`, uz cjelobrojno dijeljenje i prošireni međurezultat. Provjeravaju se pojedinačni iznos i agregat prije commita.
   - `version_id` je kanonski UUIDv7 malim slovima. Njegova Unix sekunda smije se razlikovati od `created_at_unix` najviše jednu sekundu; RFC3339 `created_at` mora označavati istu Unix sekundu kao `created_at_unix`. UUID vrijeme predaleko u budućnosti odbija se neovisno o deklariranom vremenu.
   - `node_id` i `author_pub_key` moraju biti jednaki kanonskom javnom ključu autora; nije dopušten samostalno odabrani naziv čvora. Lokalni zapisivač promatra samo potpisane verzije: ako trenutačni UUID ne stoji iza poznatoga, uzima milisekundu iza njega i zadržava svoj slučajni sufiks. `created_at`/`created_at_unix` tada prate logičko vrijeme; `applied_at` ostaje stvarno lokalno vrijeme upisa. Granica budućnosti vrijedi i za lokalni upis, pa se povrat sata ne može neograničeno širiti mrežom.
   - Stabilni `(entity, entity_id)` ne mijenja kanal. Za objavu privatnog zapisa na zajedničkom popisu treba zaseban objavljeni identitet. Pravo na jedan popis ne dopušta izmjenu stavke čiji `list_sync_id` označava drugi popis.
4. **Odstupanje sata**:
   - Signalne poruke: `±600 s` (`10 min`) od lokalnog vremena primatelja.
   - Buduće izdavanje članstva/ovlasti: najviše `+3600 s` (`1 h`).
   - Domenske verzije čiji je `created_at_unix` > `now + 3600 s` odgađaju se/odbijaju bez primjene.

## 5. Pravila konvergencije, konflikata, brisanja i shema

1. **Pobjednik konflikta (v1)**: za isti `(entity, entity_id)` pobjednička projekcija je verzija s leksikografski najvećim validiranim UUIDv7 `version_id` (UUIDv7 kodira vrijeme u prvim bitovima i jedinstveni sufiks za determinističko razbijanje izjednačenja). Sve važeće konkurentne verzije čuvaju se u `record_versions`.
2. **Brisanje (`archived = true`) i eksplicitna obnova**:
   - Brisanje zapisa ili stavke stvara novu verziju s `archived = true` (tombstone).
   - Konkurentna izmjena sa starijim `version_id` ne vraća obrisani zapis; novija verzija s `archived = true` pobjeđuje stariju izmjenu.
   - Eksplicitna korisnička obnova stvara novu verziju s višim `version_id` i `archived = false`.
   - Ako je roditelj (`shopping_lists`) arhiviran, upiti aktivnih popisa skrivaju i njegove stavke bez brisanja roditeljskog tombstonea.
3. **Dijete prije roditelja**: ako stavka, račun ili cijena stigne prije potrebnog popisa, računa, trgovine ili artikla, zapisuje se u `record_versions` i u `pending_projections`. Obrada ponavlja projekciju lanca dok postoji napredak. Račun se ne veže uz prvu lokalnu trgovinu; cijena čeka točan stabilni ID artikla i barkod.
4. **Nepoznata shema**: svaka verzija s `schema_version > 1` odbija se (uz evidenciju u `sync_peer_state` odbijenih zapisa) bez rušenja sesije ili izmjene SQLite projekcije.

## 6. Ograničenja okvira i poruka

- **4-bajtni kontrolni/signalni okvir (`wire/frame.rs`)**: `u32` big-endian duljina + tijelo. Maksimalna dopuštena veličina čita se iz zaglavlja **prije alokacije**; zadana gornja granica je `64 KiB` (`65536 B`).
- **Poslovna delta stranica**: zadana granica `256 KiB` (`262144 B`), podesiva najviše do `1 MiB` (`1048576 B`). `compute_delta_paged.max_bytes` ograničava cijeli serijalizirani `DeltaBatch`; `ExportRequest.max_bytes` ograničava cijeli plaintext `ProtectedSyncPayload`, uključujući pošiljateljevo zaglavlje. `0` bira zadanu granicu; veći zahtjev ograničava se na 1 MiB. Broje se stvarni UTF-8 JSON bajtovi i escape znakovi, potpisi, rasponi/rupе i snapshot metapodaci. Kriptografski omot/base64 na disku veći je od plaintexta i nije isti budžet.
- Stranica sadrži najveći prefiks koji stane; `has_more` označava preostale verzije. Ako prvi zapis ili obvezni snapshot/metapodaci ne stanu, vraća se greška bez preskakanja tog zapisa i bez prekoračenja. Trenutačni veliki snapshot traži budući nastavak recovery protokola, ne šalje se neograničeno.
- **Fotografija artikla**: najveća veličina pojedinog bloba `512 KiB`, provjera SHA-256 sažetka prije potvrde.

## 7. Zaštićeni paket na disku (`ProtectedBundleV2`)

- **Novi v2 format** (`.list2cart.v2.json` ili binarni omot):
  - Sadrži `format_version: 2`, `protocol`, `sender_pub_key`, `recipient_pub_key`, `ephemeral_x25519_pub`, `salt` (16 B base64), `nonce` (12 B base64), `ciphertext` (`ChaCha20Poly1305` nad JSON-om `ProtectedSyncPayload` s AAD-om `list2cart-bundle-aad-v2|<sender>|<recipient>`) i `signature` (Ed25519 potpis pošiljatelja nad kanonskim zaglavljem + `ciphertext`).
  - Izvedba koristi efemerni X25519 ključ po paketu (`ephemeral_x25519_pub`, 32 B), montgomery oblik primateljeva Ed25519 javnog ključa i odgovarajuću skalaru iz `ed25519-dalek::SigningKey::to_scalar_bytes`. Nema alternativnog simetričnog ključa ni proizvoljnog izbora derivacije. Slabi ključevi i nekontributivni X25519 rezultat odbijaju se.
  - HKDF-SHA256: IKM je 32 B zajednička tajna, salt točno 16 B iz paketa, info/AAD su točni UTF-8 bajtovi `list2cart-bundle-aad-v2|<sender>|<recipient>`. Ključ je 32 B. Nonce je 12 B iz OS slučajnosti, ChaCha20Poly1305 dodaje 16 B taga.
  - Potpis: prefiks `list2cart-bundle-sig-v2|`, decimalni format_version, `|`, protocol, `|`, sender, `|`, recipient, `|`, sirovi 32 B ephemeral key, `|`, sirovi 16 B salt, `|`, sirovi 12 B nonce, `|`, ciphertext. Fixed-size zaglavlja provjeravaju se prije provjere potpisa/dešifriranja. Signature je točno 64 B. Plaintext je najviše 1 MiB; veći skup podataka zahtijeva više stranica/paketa.
  - Ovo je List2Cart razvojni format, nije preuzet gotov interoperabilni goCOP paket niti prošao zasebni sigurnosni pregled. UI ne smije tvrditi da postoji produkcijski sigurna mrežna razmjena na temelju ovog unit testa.
- **Legacy v1 paket (`.list2cart.json`)**:
  - U potpunosti odvojen ulaz (`legacy_import_v1`), dopušten samo za eksplicitni lokalni uvoz starih zapisa bez ikakvog dodjeljivanja članstva, uloge ili pristupa kanalima u `sync_trust_records`.
  - Trenutačni javni ulaz zove se `Database::import_sync_bundle`. Ignorira priloženu knjigu/potpise i lokalno potpisuje samo snapshot promjene. Prvo otvaranje stare lokalne baze također radi jednokratno preuzimanje snapshota; povijesne nepotpisane verzije ostaju nepotpisane, a novi zapisi ne upućuju na njih kao potpisane prethodnike. Normalno ponovno otvaranje ne prepotpisuje primljene podatke.

## 8. Ponovljivost i granice provjere

### Snapshot stranice za vlastite uređaje (`snapshot_pages/1`, razvojni core API)

- Odvojeni snapshot API-ji: `Database::start_snapshot_recovery`, `export_snapshot_page`, `import_snapshot_page`, `snapshot_progress`, `discard_snapshot_transfer`. Novi zajednički `sync_request` → `sync_reply` → `import_sync_page` automatski bira ovaj put kada nedostaje sažeta povijest. Stari delta API ostaje odvojen; nema fallbacka po proizvoljnoj grešci. Razgovor S09 tek treba pregovarati mogućnost i prenositi poruke; Tauri/UI nije povezan u ovom koraku.
- Priprema u jednoj transakciji zamrzava sve zadržane potpisane ne-legacy verzije u trenutno dopuštenim kanalima i kontrolne dokaze. Izvor s nepotvrđenim rupama u tim kanalima odbija pripremu. Podržani su samo ovlašteni vlastiti uređaji istog principala; obiteljski prijenos i povijesne ovlasti ostaju S07.
- Manifest sadrži format `1`, UUID prijenosa, ključeve pošiljatelja/primatelja, skup kanala, frontier, broj verzija, veličinu i SHA-256 nad UTF-8 `serde_json` serijalizacijom uređenog `Vec<Version>` (kontrola prva, zatim UUID redoslijed). Pošiljateljev potpis postojećeg šifriranog v2 omota veže i manifest i stranicu. Manifest dokazuje dovršenost deklarirane zamrznute cjeline; ne zamjenjuje provjere pojedinačnih izvornih potpisa/ovlasti niti dokazuje nepostojanje povijesti nepoznate izvoru.
- Plaintext oblika `{"snapshot_page":{"manifest":...,"offset":0,"versions":[...],"done":false}}` mjeri se u stvarnim JSON bajtovima, uključujući cijeli manifest. Zadano 256 KiB, najviše 1 MiB; prvi preveliki zapis/metapodaci daju grešku. Izvoz čita ograničen skup kandidata za jednu stranicu; priprema i završna primjena mogu držati cijeli snapshot u memoriji unutar ukupne kvote.
- Shema baze v4 čuva manifest, nepromjenjive izlazne verzije, ulazne stranice, idući offset i konačni status. Stranice moraju biti uzastopne. Ponavljanje identične stranice vraća isti napredak; promijenjeni manifest/ponovljeni sadržaj ili preskočena stranica odbijaju se. Ponovno otvaranje baze nastavlja isti prijenos i nakon gašenja obje strane.
- Nedovršeni ulaz čuva se odvojeno od projekcija i knjige. Tek zadnja stranica, nakon provjere broja/bajtova/hash/frontiera, ulazi u isti validacijski i projekcijski kod kao delta, pod vanjskom transakcijom. Bilo koja odbijena verzija, neriješena ovisnost, izgubljena ovlast ili SQL greška poništava završni upis. Sužavanje rupa, potvrde i oznaka dovršenosti pripadaju toj istoj transakciji. Novije lokalne verzije ostaju pobjedničke; snapshot ne briše druge lokalne zapise.
- Na završetku se brišu samo izvedene ulazne stranice, a dovršeni manifest/status ostaje radi idempotentnog ponavljanja. Zajednički API čuva izlaz do potpisane primateljeve potvrde dovršenog commita, pa atomarno označava izlaz dovršenim i briše njegov izvedeni sadržaj. Dostupan je i eksplicitni `discard_snapshot_transfer`; ne briše poslovne podatke. Najviše osam nedovršenih sesija po smjeru, 100.000 verzija i 64 MiB JSON podataka po snapshotu.
- Izvoz i svaki prijem ponovno provjeravaju aktivno vlastito članstvo/opoziv; izvoz ponovno provjerava kanale, a završna primjena čita aktualna prava. Kontrolne promjene u snapshotu ne smiju zaobići tu provjeru. Širi S07 audit delegiranja/povijesnih opoziva i promjena prava ostaje otvoren.

### Potvrda raspona i nastavak (S06, djelomično izvedeno)

- Raspon znači `(from_id, to_id]`; prazan `from_id` označava početak toka autora/kanala. Najveći primljeni UUID sam ne dokazuje prethodnu pokrivenost, ni kod nezavisnih zapisa bez `supersedes` veze.
- Primljeni nepotvrđeni dio ostaje trajna rupa u istoj transakciji kao zapisi/projekcije. Potvrda se sprema tek uz uspješan commit. Greška potvrde vraća i projekcije i promjenu rupa.
- Odbijanje bilo koje verzije u toku blokira potvrdu tog toka za tu stranicu; drugi dopušteni tokovi mogu napredovati. Naknadno odbijeni stariji zapis ponovno otvara raspon. Evidencija odbijanja nije pravo na preskakanje podataka.
- Za sužavanje rupe traži se primljen i prihvaćen potpisani završni zapis istog autora/kanala, dopušten kanal i odsutnost preklapanja s pošiljateljevom rupom. Prazna stranica ne pomiče granicu. Verzija filtrirana prije prijema ne smije potvrditi svoj tok.
- Zahtjev za starom rupom spušta početak delte tog toka. Šalje se kontinuirani prefiks od tog početka, kroz više stranica po potrebi, umjesto spajanja stare popune i nove delte uz prešućeni međuraspon. Konzervativno objedinjavanje rupa može ponoviti već poznate zapise.
- Budžet sada koristi stvarno serijalizirane bajtove. Provjera granice sažimanja uključuje novi uređaj bez granice i staru traženu rupu ispod praga, čak kad primatelj već ima visok MAX; ti slučajevi ne dobivaju običnu deltu s lažnom potvrdom potpunosti.
- Snapshot core sada ima više stranica, trajni nastavak, atomsku potvrdu deklarirane cjeline i zajednički API za automatski odabir oporavka vlastitih uređaja. Naknadne dopune koriste izvorne lokalne oznake inventara opisane niže. Preostaje šira matrica promjena prava uz S07 i filtriranje kontrolnih dokaza po primatelju; nema automatskog mrežnog prijema.

### Potpisani zahtjev i automatsko usmjeravanje (`sync_request/2`)

- `SyncRequest` ima polja `format=2`, `sender`, `recipient`, `created_at` (Unix sekunde), `nonce` (UUID), `frontier`, `holes`, `recovery` (zadnji trajan SnapshotProgress ili null), `backfill_revisions` (kanal → potvrđena oznaka inventara baš ovog izvora), `signature` (standardni base64). Zahtjev je najviše 64 KiB; frontier/holes i oznake najviše 4096 elemenata uz ograničene duljine. Dopušteno odstupanje vremena je ±600 s. Zahtjev za vlastiti isti ključ odbija se. Prethodni razvojni zahtjev v1 odbija se; nema tihog prihvaćanja starog potpisa nad nepotpunim znanjem.
- Potpisani bajtovi: UTF-8 prefiks `list2cart-request-v2|` + kompaktni `serde_json` niz `[format,sender,recipient,created_at,nonce,frontier,holes,recovery,backfill_revisions]`. Mape su BTreeMap s poretkom ključeva; `recovery`, kad postoji, ima polja `snapshot_id,next_offset,total_versions,complete` u tom redoslijedu. Dokaz Ed25519 ključa, točan primatelj i trenutačno aktivno vlastito članstvo provjeravaju se prije rada. Vremenska ograda i potpis nisu tvrdnja o jednokratnom replay cacheu: ponovljeni zahtjev unutar roka može ponoviti slanje iste stranice.
- Primatelj gradi zahtjev iz lokalne potvrđene granice/rupa i najnovije trajne ulazne snapshot sesije za pošiljatelja, uključujući dovršenu. Ograničava metadata kanalima dopuštenim objema stranama. Dovršena novija sesija ima prednost pred starijim napuštenim ulazom.
- Pošiljatelj prvo nastavlja poznati nepotvrđeni snapshot od primateljeva potpisanog offseta. Bez takvog nastavka ponovno koristi postojeći nepotvrđeni izlaz (npr. izgubljena prva poruka). Zatim provjera `recovery_keys` i oznaka inventara odlučuje treba li novi snapshot zbog compaction_floor ili naknadno primljenih podataka; inače se šalje obična delta. Nepovezana greška, poput premalog budžeta obične stranice, ne pokreće snapshot.
- Ako je izgubljen izlazni snapshot ili primatelj traži nastavak već potvrđenog/oslobođenog izlaza, počinje novi snapshot. Stariji djelomični ulaz ne dobiva ponovno prednost nakon završetka novoga. Izgubljene/stare izvedene sesije mogu se eksplicitno odbaciti; njihovo brisanje ne briše knjigu/projekcije.
- Sam izvoz zadnje stranice nije potvrda primitka. Tek idući potpisani zahtjev s potpunim brojem/offsetom i `complete=true` potvrđuje snapshot na izvoru. Oznaka i čišćenje izvedenih stavki su jedna transakcija. Nakon toga odgovor se vraća na deltu, uključujući novije promjene nastale nakon zamrzavanja.
- `import_sync_page` određuje vrstu iz autentificiranog dešifriranog sadržaja i vraća tipizirani `Delta {report,has_more}` ili `Snapshot {progress}`. U automatskom putu odbija stare monolitne reset/snapshot metapodatke bez manifesta stranica. Privatni ključevi nisu dio tih DTO-ova.
- Ovo je transportno neovisan razvojni core tok za vlastite uređaje. Ne predstavlja dovršeni mrežni razgovor S09, LAN, obiteljski S07, UI ni neovisan sigurnosni pregled novog ugovora.

### Naknadne dopune i izvorne oznake inventara (S06)

- MAX autora/kanala ne dokazuje da drugi čvor ima naknadno otkrivenu stariju verziju. Čak i verzija iznad posrednikova lokalnog MAX-a može biti ispod MAX-a primatelja. Zato svaka uspješno primljena **nova** potpisana verzija mijenja izvorno lokalnu oznaku svojega kanala (jednom po kanalu/transakciji). Identično ponavljanje postojećeg version_id ne mijenja oznaku. Izvorne lokalne naredbe ostaju na običnoj delti.
- Shema v5 dodaje `sync_backfill_revisions` i `sync_backfill_ack`. Oznaka je neprozirni UUID, nije zidni sat ni broj koji se uspoređuje po veličini. Uspoređuje se jednakost; nepostojanje je prazna oznaka. Tako povrat starije kopije oznake ne maskira buduću novu dopunu. Migracija jednom označava sve postojeće potpisane ne-legacy kanale kako prethodno postojeće dopune ne bi ostale prešućene; ponovljeno otvaranje ne mijenja oznake.
- Oznaka i upis/prikaz primljenih verzija dijele transakciju. Snapshot manifest ima opcionalni `backfill_revisions` sa zamrznutim oznakama dopuštenih kanala. Potpis šifriranog paketa veže mapu. Na završnom commitu primatelj sprema potvrdu po `(ključ izvora, kanal)`; ne kopira izvorne oznake u svoj lokalni inventar. Nepotpun snapshot ili rollback ne potvrđuju novu oznaku.
- Ako nova promjena stigne izvoru dok se snapshot već prenosi, dovršava se stari zamrznuti sadržaj i potvrđuje samo njegova oznaka. Sljedeći zahtjev vidi razliku i pokreće novi oporavak. Noviji zapis na primatelju ostaje sačuvan. Oznake se šalju/provjeravaju samo unutar aktualno dopuštenih kanala.
- Izvedba je namjerno konzervativna: prosljeđivanje novoprimljenih podataka može ponovno poslati cijeli ograničeni snapshot, umjesto optimizacije točno preskočenih verzija po partneru. Nakon potvrde i bez novih prijema nema nove obnove. To je prilagodba goCOP `peers/granica_nakon_uvoza.go` invariante; nisu kopirani Go izvori niti uvedena tvrdnja o mrežnoj interoperabilnosti.

### Opsezi opoziva i stabilnost kontrolnih identiteta (prvi dio S07)

- Potpisani `Revocation.kind` prihvaća `clanstvo`/`membership`, `list_grant` i `ovlast`; nepoznat novi opseg se odbija. Ključ subjekta mora biti kanonski valjan, cilj neprazan, a vrijeme opoziva ne smije prethoditi izdavanju ni biti u budućnosti. Kanonski potpisani bajtovi opoziva nisu promijenjeni.
- Članstvo opoziva cijeli ključ uređaja. Grant opoziva samo točan grant ID i ključ, ne ostale popise. Delegacija opoziva administratorsku ovlast i prava izdana preko nje; stara ugrađena Ovlast potvrda ne zaobilazi opoziv. Mrežni korijenski ključ ne može se opozvati kao uređajsko članstvo/delegacija.
- Cijeli opoziv administratorova ključa (`membership` ili konzervativni `legacy_key`) također zatvara prava izdana preko njegove delegacije. Provjera kanala/own-device privatnog pristupa ponovno provjerava potpisane dokaze i opseg opoziva, pa ranije pohranjen revoked=0 nije dopuštenje. Eksplicitni revoked=1 i dalje blokira pristup. Provjera write granta veže i stvarni potpisani sadržaj na ključ autora, kanal, grant_ref i Write permission.
- Isti kontrolni ID ne može promijeniti vrstu, subjekt ili cilj. Grant i opoziv imaju nepromjenjiv sadržaj; novi grant ID omogućuje ponovno dopuštanje popisa. Članstvo ne može promijeniti principal; starije izdavanje ne prepisuje novije, a različit sadržaj s istim vremenom se odbija. Aktivno pravo zahtijeva i da izdavanje nije u budućnosti. Opozvanom uređaju potreban je novi ključ.
- Shema v6 normalizira stare opsege opoziva i ponovno računa izvedene revoked oznake. Time ispravlja stari preširoki opoziv jednog granta. Nepoznat/neispravan legacy opseg i dvosmislen cilj ostaju konzervativno blokirani kao `legacy_key`. Migracija je atomska i ponovljiva.
- Upis kontrolnog cachea i svih njegovih revoked oznaka dijeli savepoint: greška ne ostavlja djelomičan dokaz ni promjenu prava, čak i kad se ostatak paketa nastavi. Objavljivanje potpisane kontrolne verzije mora biti u istoj vanjskoj transakciji s knjigom.
- Odabrani kontrolni zapisi prate obiteljske stranice i automatski recovery kako je opisano niže. Tauri/UI i LAN ostaju otvoreni. Provjera opoziva ostaje konzervativna i za povijesne verzije.

### Povijesno važenje ovlasti (S07, 18. 11. 2026.)

- Prijem čuva i potvrde koje su danas istekle: njihov potpis/lanac provjerava se u trenutku izdavanja, uz zabranu budućeg izdavanja. To ne daje današnji pristup; odabir kanala i dalje zahtijeva trenutačno važeće članstvo/grant.
- Autor domenske verzije dokazuje članstvo i write grant u `created_at_unix` verzije. Članstvo se traži u cacheu i potpisanoj ne-legacy kontrolnoj knjizi. Odabire se najnovije izdavanje do tog trenutka, pa se provjeravaju potpis, mreža, rok i delegacija. Nema povratka na starije, šire članstvo nakon downgradea ili isteka novijeg. Dvosmislena ista izdavanja se odbijaju i kad ih noviji cache više ne pokazuje.
- Delegirani potpisnik kontrolnog omota provjerava se prema potpisanoj Ovlast povijesti u trenutku omota; potpisnik opoziva u trenutku izdavanja opoziva. Ugrađena delegacija članstva/granta i dalje mora pokrivati izdavanje i cijeli rok te potvrde. Istek danas ne poništava staru valjanu promjenu niti dopušta novu nakon isteka.
- Ako postoji lokalno poznata novija Ovlast izdana do trenutka izdavanja članstva/granta, ugrađena potvrda mora odgovarati tadašnjoj najnovijoj potvrdi. Stara dulja potvrda ne smije zaobići noviju kraću. Provjera se ponavlja pri uporabi pohranjenih dokaza, uključujući kad je kraća potvrda stigla naknadno. Certifikati izdani prije te promjene ne odbijaju se samo zato što postoji novija potvrda.
- Rok je uključiv: potvrda vrijedi na expires_at, a istječe poslije te sekunde. Odabir kanala koristi trenutačno vrijeme; autor povijesne verzije dokazuje pravo u vremenu verzije. Istek pristupa tijekom nepotpunog prijenosa ne dopušta commit stare zamrznute domene; automatski obiteljski dispatcher može poslati novi kontrolni snapshot dok je članstvo još aktivno.
- Primljeni opoziv ključa/granta/delegacije i dalje blokira odgovarajući povijesni dokaz bez obzira na starost promjene. Timestamp autora nije neovisan dokaz nastanka prije opoziva. Kontrolna knjiga već je izuzeta iz compaction brisanja; ovaj korak ne mijenja shemu v6 ni potpisane formate.
- To vrijedi i za ranije primljene odgođene projekcije. Kada kasnije stigne ovisnost, prije prikaza stavke ponovno se provjeravaju autorov potpis/pravo, opozivi i aktualno dopušteni kanali primatelja. Blokirana stavka ostaje u potpisanoj knjizi i pending redu, a neovisni valjani zapisi smiju se primijeniti. ApplyReport.blocked_projections (serde default prazni niz) bilježi te ID-eve i razloge odvojeno od rejected novih verzija. Snapshot ne potvrđuje verziju s nedostajućom/blokiranom projekcijom. Već ranije prikazani podaci ne brišu se retroaktivno ovim pravilom.
- Ako autorovi dokazi nedostaju, verzija se odbija bez projekcije i granica ostaje otvorena. Članstvo/grant mogu stići bilo kojim redoslijedom; ponavljanje iste verzije nakon potpune provjere smije se prihvatiti i zatvoriti coverage. Unutar jednog paketa kontrole i opozivi uvijek prethode domeni, bez obzira na redoslijed elemenata u JSON-u.
- Ovo rješava provjeru kad dokazi postoje lokalno. Njihov prijenos i automatski exchange/recovery opisani su niže.

### Obiteljski kontrolni pogled i stranice (S07, 18. 11. 2026.)

- Izvoz bira puni kontrolni pogled samo za root primatelja ili vlastiti uređaj istog principala kao pošiljatelj. Drugi primatelji, uključujući tuđi OwnDevice, dobivaju ograničeni pogled bez privatnih kanala. Prava, dokazi i stranica čitaju se u jednoj DB transakciji/snapshotu.
- Odabir kreće od primatelja i autora svih lokalnih potpisanih zapisa trenutno dopuštenih kanala. Uključuje njihovu povijest članstva, točne grant_ref autora na tim popisima, primateljeve vlastite grantove (i opozvane/istekle), potrebne Ovlast potvrde/izdavatelje omota i relevantne opozive. Ovisnosti se dopunjavaju do zatvorenog skupa. Nepovezani grantovi drugih osoba/popisa i administratori se izostavljaju. Ovo je konzervativni skup po dopuštenim kanalima, ne najmanji mogući skup za svaku pojedinu stranicu.
- Izvorni potpisani kontrolni omoti ponavljaju se u `batch.versions` uz svaku stranicu. Njihovi bajtovi rezerviraju se prije običnog straničenja domene i uključeni su u ukupni budžet plaintexta s wrapperom (najviše 1 MiB). Nedostajući omot koji postoji samo u trust cacheu, nesiguran relevantni legacy opoziv ili prevelik skup završava greškom. Izvoz ne izmišlja izvorne potpise i ne dijeli dokaze na nepotpune stranice.
- Kontrolni frontier/rupe primatelja ne preskaču te dokaze. Obiteljska stranica nema globalni kontrolni covered_range/sender_hole; domenski kanali koriste postojeće granice. Uvoz s pošiljatelja koji nije vlastiti uređaj ignorira i nametnute kontrolne granice/rupe. Primanje podskupa zato ne potvrđuje da su primljeni svi tuđi kontrolni zapisi. Primljeni dokazi normalno se čuvaju i ponavljanje je idempotentno.
- Ovlast omoti obrađuju se prije drugih delegiranih kontrola čak i kad je njihov UUID kasniji; opozivi se obrađuju nakon ostalih kontrola i prije domenskih verzija. Postojeći transakcijski rollback pokriva dokaze, podatke i napredak zajedno.
- Veliki skupovi dokaza mogu koristiti izričiti trajni snapshot opisan niže. Automatski obiteljski odabir/oporavak i šira matrica opozvanih povijesnih autora preostaju. Ručni ograničeni izvoz nije dovršeni automatski obiteljski razgovor/S09 niti mrežni produkcijski protokol.

### Trajni obiteljski snapshot (`SnapshotManifest.format=2`)

- `start_family_snapshot_recovery(recipient)` zamrzava list kanale zajednički dostupne pošiljatelju i primatelju te isti odabrani skup izvornih potpisanih dokaza. Root ima administrativni pristup bez posebnog granta. `export_snapshot_page`, `import_snapshot_page` i `snapshot_progress` koriste iste trajne tablice/offset kao format 1. Obje strane moraju već biti aktivni članovi. Novi primateljev grant može stići unutar snapshota, ali se provjerava prije završnog commita; ovaj tok nije zamjena za uparivanje. Format 1 zadržava ograničenje vlastitih uređaja.
- Format 2 dopušta samo `control` i `list:` kanale. Njegov frontier i backfill_revisions nemaju kontrolni kanal. Obavezni `proof_sha256` veže kanonski JSON niz odabranih kontrolnih verzija; puni sha256 i broj/veličina verzija vežu cijeli zamrznuti skup. Opcionalno polje ne mijenja čitanje postojećih manifesta formata 1.
- Dokazi se mogu podijeliti preko više stranica bez ponavljanja cijelog skupa. Primatelj ih samo stagea do potpunog hash/veličina/frontier pregleda; tek tada jedna transakcija primjenjuje kontrole, projekcije, domenske ACK i backfill potvrde. Greška vraća i završnu stranicu/napredak, pa se smije ponoviti. Nema potvrde globalnog kontrolnog kanala.
- Svaki nastavak provjerava aktivno članstvo i pošiljateljev pristup; izvor provjerava i primateljeva prava. Na ulazu novi grant smije čekati u stagingu, a tek završni commit mora dokazati pravo obiju strana na svaki kanal. Lokalni sažetak trust cachea sprema se samo u lokalnu kopiju manifesta (`_local_acl_hash`, nije dio wire DTO-a): promjena cachea prekida postojeći djelomični ulaz. Izvor ponovno računa sažetak relevantnih dokaza: promijenjen dokaz/opoziv/regrant zahtijeva novi snapshot čak i kada skup list kanala izgleda jednako. To ne poništava već ranije preuzete podatke. Svaka izlazna stranica čita prava/dokaze/sadržaj u istom DB snapshotu.
- Ostaju granice 100.000 verzija/64 MiB i 8 nedovršenih sesija po smjeru. Jedna verzija i manifest moraju stati u maksimalnu stranicu 1 MiB; nema rezanja pojedinačnog potpisa/payload zapisa. Izvor s domenskom rupom ne može jamčiti potpuni snapshot; djelomični kontrolni pogled ne potvrđuje globalne kontrolne rupe.
- API omogućuje izričiti i automatski nastavak nakon restarta. Obiteljski dispatcher opisan je niže. Tauri/UI i LAN/S09 nisu povezani.

### Automatski obiteljski razgovor (`sync_request/3`)

- `sync_request` bira postojeći format 2 za vlastite uređaje, a format 3 za druge aktivne članove. Obiteljski zahtjev ima istu strukturu polja, ali prefiks potpisa `list2cart-family-request-v3|`; potpis veže i format. Frontier, holes i backfill_revisions smiju sadržavati samo `list:` kanale i generiraju se samo za zajednički dopuštene popise, bez privatnih/kontrolnih metapodataka. Ostaju 64 KiB, ±600 s, nonce/primatelj/potpis provjere. Sam format 3 ne daje prava; obje strane moraju imati aktivno članstvo.
- Obiteljski odgovor je namjerno konzervativan: prvi razgovor, nova domenska verzija, kasna dopuna/backfill, promijenjen skup prava/dokaza, izgubljeni izlaz ili zastarjelo znanje pokreću snapshot formata 2. Nema posebnih obiteljskih podatkovnih delti u ovom dispatcheru. Nakon potvrde nepromijenjenog stanja vraća se mali šifrirani prazan Delta odgovor, bez ponavljanja velikog skupa dokaza.
- Potpisani `recovery` nastavlja poznati nepromijenjeni zamrznuti izlaz. Bez primljenog početka koristi se postojeći nepotvrđeni izlaz od offseta 0, uključujući nakon restarta. Relevantna promjena scopea/dokaza zamjenjuje stari izvedeni izlaz novim snapshotom. Novi ulazni epoch zamjenjuje samo nedovršeni obiteljski staging istog peera. Lokalna promjena trust cachea pri idućem zahtjevu odbacuje taj djelomični ulaz i traži početak; nijedna projekcija se time ne briše.
- Izvoz zadnje stranice ne oslobađa izlaz. Tek primateljev sljedeći valjano potpisan zahtjev s istim snapshot_id, točnim total_versions i next_offset te complete=true u jednoj transakciji označava izlaz dovršenim i briše njegove izvedene stavke. SQL greška vraća i oznaku i brisanje. Ponovljena potvrda je idempotentna. Potpisani lažni iskaz primatelja o vlastitom znanju nije neovisan dokaz pohrane; izvor smije osloboditi kopiju na temelju njegove potvrde.
- Nakon potvrde ponovno se uspoređuju trenutni dokazi, frontier, compaction uvjeti i domenske oznake backfilla. Naknadni stariji zapis zato se ne skriva iza MAX-a. Promjena/nepoznat završeni izlaz konzervativno traži novi snapshot. Stare vlastite v2 poruke i njihove delte zadržavaju postojeći put. Nema trajnog one-shot nonce cachea; ponavljanje potpisanog zahtjeva unutar roka može ponovno vratiti stranicu.

`tools/sync-fixtures/main.go` koristi Go standardnu biblioteku i javne testne seedove. Generirani `crates/list2cart-sync/testdata/go-vectors.json` ulazi u Rust testove. Vektori pokrivaju javne ključeve, SAS, HKDF/HMAC udarac, ovlast, signalni potpis i List2Cart potpis verzije. Omot verzije namjerna je List2Cart razlika; prolaz ovih vektora nije dokaz kompatibilnosti cijelog goCOP protokola.

Otvoreno za S05–S09: pravila obnove/tombstone konkurencije, ovlasti povijesnih/delegiranih autora nakon opoziva, filtriranje potrebnih kontrolnih zapisa za prosljeđivanje i dokaz potpunosti raspona/snapshot oporavka. Postojeća djelomična implementacija tih funkcija nije zaključena ovim dokumentom. Stanje je u [STATUS.md](STATUS.md).
