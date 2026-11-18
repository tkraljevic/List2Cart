# goCOP → Rust: cheatsheet za agenta

Uz [PLAN.md](PLAN.md) i [AGENT_TASKS.md](AGENT_TASKS.md). Ovo je mapa izvora i postupka, ne gotov mrežni protokol ili provjerena Rust implementacija.

## 1. Gdje gledati i kojim redom

Lokalni izvor: `/Users/tomislavkraljevic/projekti/goCOP`.
Pregledani HEAD: `fcca2c6bda5a2c43fa8b051994d4a5ad1a819e7e`.
Prije preuzimanja provjeri promjene relevantnih datoteka i zabilježi hash njihova sadržaja. Repo je tijekom pregleda imao i neprateće runtime datoteke; njih ne kopirati. Za domenske podatke autoritativan je List2Cart ugovor, ne goCOP model.

| Red | goCOP izvor | Što prenijeti | Predloženo Rust mjesto |
|---|---|---|---|
| 1 | `internal/ledger/ledger.go`, `schema.go`, `shema.go` | verzije, transakcijski Recorder, delta/frontier, validacija i sažimanje | postojeći `list2cart-core/ledger.rs` + migracije |
| 2 | `internal/peers/upis.go`, `razgovor.go` | granica između primljenih verzija i projekcije, redoslijed razgovora i potvrda | core apply + sync session |
| 3 | `internal/razmjena/keys.go` | trajni ključ, javni oblik, certifikat, provjera protokola, SAS | identity + TLS |
| 4 | `internal/razmjena/pair.go` | Hello, commit–reveal, SAS, Finish u obje uloge | pairing |
| 5 | `internal/razmjena/network.go`, `opoziv.go`, `potvrda_prava.go` | članstvo, delegacija gdje treba, provjerljive ovlasti, opoziv | trust; List2Cart grantovi po popisu |
| 6 | `internal/razmjena/exchange.go`, `provjera_kljuca.go`, `vrata_adrese.go` | autentificirani tok, ograničenja rukovanja, očekivani ključ i odbijanja | transport LAN + wire |
| 7 | `internal/peers/rupe.go`, `granica_nakon_uvoza.go`, `granica_odbijenih.go`, `ograde_razgovora.go` | nepotpun prijenos, rupe, odbijeni zapisi, granice po bajtovima | session + trajna sync evidencija |
| 8 | `internal/razmjena/signal.go`, `signal_tls.go`, `signal_veza.go`, `sastajaliste.go`, `poziv.go`, `okvir.go` | signalizacija, sastajalište, potpisani bajtovi i replay zaštita | signal client/server + wire |
| 9 | `internal/razmjena/udp.go`, `stun.go`, `netcheck.go`, `kandidati.go`, `kandidati_druge.go` | jedan UDP port, STUN, klasifikacija mreže, kandidati | UDP adapter + netcheck |
| 10 | `internal/razmjena/udarac.go`, `udaranje.go`, `probijanje.go` | autentificirani paketi i čisti automat prije async efekata | punch codec + state machine |
| 11 | `internal/razmjena/quic_slusalica.go`, `web_sync.go`, `sesija.go`, `nova_uticnica.go` | QUIC, životni ciklus sesije, stara/nova priprema i promjena mreže | QUIC + session manager |
| 12 | `internal/peers/redoslijed.go`, `priprema_weba.go`, `razmjena/tunel.go`, `put.go` | izbor puta, pričuvni put, TLS unutar tunela, zatvaranje gubitnika | service + tunnel |
| 13 | `internal/sadrzaj`, sadržajni dio `peers/razgovor.go` | hash → zahtjev → sadržaj → provjera | blobs |
| 14 | `cmd/gocop/web_sync.go`, `internal/web/handlers_pairing.go`, `handlers_sync.go` | povezivanje servisa, konfiguracije i UI-a; nije transportna biblioteka | Tauri lifecycle/commands/events |

Ne postoji dovoljno dobar ekvivalent postupku „kopiraj samo ledger.rs i dodaj HTTP”. goCOP raspodjeljuje odgovornost kroz ledger, peers, razmjena i sadržaj.

`/Users/tomislavkraljevic/projekti/syncnet/README.md` opisuje izdvojenu Go biblioteku za ključeve, SAS, TLS i LAN beacon. Korisna je za razumijevanje manje mrežne jezgre; ne pretpostavljaj da sadrži aktualni goCOP WebSync. Usporedi konkretne simbole i testove. Go `internal` paketi nisu javni uvozni API drugog projekta.

## 2. Rust gradivni elementi

Ovo su kandidati, ne novi već instalirani dependencyji. Prije dodavanja provjeri međusobnu kompatibilnost, MSRV, licencu i sve ciljne platforme. Zadrži postojeći `rusqlite`, `serde` i `uuid` gdje odgovaraju. Nemoj uvoditi drugi SQLite driver samo zbog async API-ja.

| Go pojam | Rust kandidat / obrazac | Napomena |
|---|---|---|
| goroutine, channel, context | Tokio task, ograničeni `mpsc`/`oneshot`, `CancellationToken`, `JoinSet` | isti cancel scope za sesiju i sve pripreme; čekati završetak taskova |
| `crypto/ed25519` | `ed25519-dalek` | kontrolirati oblike ključeva i potpisa, ne miješati 32 B seed i 64 B keypair |
| `crypto/tls`, TCP | `rustls` + `tokio-rustls` | provjerljivi peer certifikat i dokaz posjedovanja ključa u oba smjera |
| `quic-go` | `quinn` | nema pretpostavke o izravnom ekvivalentu ReadNonQUICPacket |
| `crypto/sha256`, HMAC, HKDF | održavane Rust implementacije SHA-256/HMAC/HKDF | točan ulaz, prefiks, salt/info i smjer potvrditi vektorima |
| X.509/PKCS#8/PEM | kompatibilni encoder/parser, npr. rcgen i odgovarajući parser | provjeriti SAN/Ed25519 i potreban oblik ključa; ne pisati ASN.1 ručno |
| WebSocket | `tokio-tungstenite`, tanak byte-stream adapter gdje je potreban | WS poruke nisu TCP bajtovni tok; podržati djelomično čitanje/pisanje |
| atomska brojila / `sync.Mutex` | `AtomicU64`, kratki mutex, jedan actor za stanje | ne prenijeti Go lock opseg preko Rust `.await` |
| vrijeme | `Instant` za rokove, UTC za wire/povijest, injektirani sat u testu | promjena zidnog sata ne produljuje deadline |
| prikazivanje stanja | tipizirani enum + Tauri događaji | greška po putu/partneru, bez tajni |

Službena dokumentacija provjerena pri izradi plana:

- [Quinn](https://docs.rs/quinn/latest/quinn/) i [AsyncUdpSocket](https://docs.rs/quinn/latest/quinn/trait.AsyncUdpSocket.html).
- [Rustls](https://docs.rs/rustls/latest/rustls/) i [Tokio Rustls](https://docs.rs/tokio-rustls/latest/tokio_rustls/).
- [Ed25519 Dalek](https://docs.rs/ed25519-dalek/latest/ed25519_dalek/).
- [CancellationToken](https://docs.rs/tokio-util/latest/tokio_util/sync/struct.CancellationToken.html).
- [Tokio Tungstenite](https://docs.rs/tokio-tungstenite/latest/tokio_tungstenite/).

URL `latest` nije pin verzije. S01 bilježi stvarno odabrane verzije i točne API-je. Ne umeći novi Tokio runtime unutar Tauri runtimea bez razloga; integriraj lifecycle i kontrolirano gašenje s postojećim izvršavanjem.

## 3. Portiranje bez nagađanja

Za svaki modul slijedi isti postupak:

1. Pročitaj Go tipove, funkcije, komentare s invariantama i odgovarajuće `*_test.go` datoteke.
2. Zapiši ugovor: ulazni bajtovi, izlazni bajtovi, stanje, greške, rokovi, ograničenja i tko posjeduje resurs.
3. Izdvoji čistu funkciju/automat u Rust. Prvo usporedi Go i Rust testne vektore.
4. Tek zatim dodaj Tokio I/O adapter. Injektiraj sat, generator slučajnosti i transport gdje to omogućuje stvaran rubni test.
5. Provedi test pogreške i gašenja. Provjeri koliko je taskova/socketa ostalo nakon cancel/retry.
6. Zabilježi namjerne List2Cart razlike. Prolaz vektora potvrđuje samo provjereni dio, ne kompatibilnost cijelog sustava.

Go testni harness nemoj dodavati u radni goCOP repo. Kopiju potrebnog izvora/fixture generatora smjesti u izolirani testni modul u List2Cartu uz točno podrijetlo i licencu; Go `internal` ograničenja riješi strukturom tog modula. Testni privatni ključevi javni su fixturei i nikada se ne koriste za uređaje.

## 4. Kriptografija i kodiranje: najčešće pogreške

### Ključ nije certifikat i nije naziv uređaja

goCOP javni ključ predstavlja standardnim base64 kodiranjem sirovih 32 Ed25519 bajta. Privatni ključ čuva u PKCS#8 PEM datoteci izvan baze. Neuspješno učitavanje postojećeg ključa nije razlog za novi ključ.

U Rust TLS verifikatoru provjeri ključ, protokol i TLS handshake potpise. Nemoj vratiti bezuvjetni `assertion()` iz svih provjera zato što certifikat nije iz javne CA mreže. Pinning zamjenjuje CA/hostname model povjerenja za P2P; ne uklanja dokaz posjedovanja privatnog ključa. Pairing način prihvaća nov dokazani ključ samo do korisničke SAS odluke, normalna razmjena traži već dopušten ključ/članstvo.

U goCOP `keys.go` postoje i specifični CN/SAN oblici i prefiks vjerodajnica. List2Cart imenuje vlastiti protokol i prefikse. Ne kopiraj produkcijsko ime `gocop` da bi test prošao. Promijenjen prefiks mora se dosljedno promijeniti u certifikatu, SAS-u, potpisima, discoveryju i ALPN-u.

### SAS: točno što se računa

Referenca: `keys.go:SASCode`, `pair.go:dogovoriKod`, `obvezaZa`, `Finish`.

```text
commit = SHA256(UTF8(protocol + "-pair-commit-v2|") || caller_nonce)
caller šalje commit
listener šalje svoj CSPRNG nonce (32 B)
caller otkriva svoj CSPRNG nonce (32 B)
listener provjerava commit

x, y = oba javna ključa sortirana po sirovim bajtovima
h = SHA256(UTF8(protocol + "-pair-v2|") || x || y
           || caller_nonce || listener_nonce)
sas = zero_pad_6(big_endian_u32(h[0..4]) mod 1_000_000)
```

SAS može početi nulom. Redoslijed nonceova određuje uloga u uparivanju, ne sortiranje ključeva. Obje strane moraju potvrditi; SAS nije trajna lozinka niti polje koje uvezeni JSON smije koristiti za dodjelu `own_device` prava.

### JSON bajtovi i potpis

- Go `encoding/json` za `[]byte` koristi base64 string. Rust `serde_json` za obični `Vec<u8>` daje niz brojeva. Dodaj eksplicitni standard-base64 serde adapter, s fixtureom. [`serde_bytes`](https://docs.rs/serde_bytes/latest/serde_bytes/) sam po sebi nije dogovor da JSON postaje Go base64.
- Go `omitempty` nije isto što i slanje `null`, praznog stringa ili praznog niza. Potpisani oblik mora propisati svako polje.
- Go string escape ponašanje (`<`, `>`, `&`, U+2028/U+2029), redoslijed polja, vrijeme i brojčani zapis mogu se razlikovati od serde izlaza. Ne pretpostavljaj da dva semantički jednaka JSON-a imaju isti potpis.
- Signal `poziv.go` potpisuje prefiks `protocol + "-" + vrsta + "-v1|"` i **izvorne bajtove tijela**. Na prijemu verificiraj te bajtove, a ne ponovno serijaliziranu strukturu.
- `Membership.signedBytes()` zasebno konstruira kanonski niz polja i Unix sekunde. Wire `issuedAt` prikaz nije isto što i potpisani oblik. Go nulta `time.Time` nije Unix epoch. Dodaj vektore za to pravilo ili namjerno definiraj novu List2Cart shemu.
- Ne potpisuj nestabilno float/string predstavljanje količine. Za novi omot verzije zamrzni canonical payload bytes; za valutu koristi cente, a preciznost količine specificiraj.

### HKDF/HMAC paketi udaraca

Referenca: `udarac.go:NoviKljuceviUdarca`, `kljucSmjera`, `sastavi`, `provjeri`, `oknoUdaraca`.

```text
IKM = call_secret || response_secret
salt = prazno / nil prema HKDF-SHA256 ugovoru
info = UTF8(protocol + "-udarac-v1") || session_id_16B || direction_1B
key = HKDF-SHA256(IKM, salt, info, 32 B)

offset  veličina  značenje
0       1        0x10 udarac / 0x11 odgovor
1       1        smjer (0 ili 1); A ima manji javni ključ
2       1        verzija 1
3       1        rezervirano
4       16       session ID
20      8        brojač, big-endian
28      32       HMAC-SHA256 nad bajtovima [0..28)
ukupno: 60 B
```

Pričuva brojača ima prozor 64; nula, duplikati i prestari brojevi odbijaju se. Go shift ≥64 ima drukčiji praktični rub od Rust pomaka: eksplicitno granaj velike pomake umjesto panike/overflowa. HMAC provjeravaj odgovarajućom konstantnovremenskom rutinom. Pozvani daje novu tajnu za novi pokušaj sesije nakon restarta, da stari snimljeni paketi ne otvore novu pripremu.

Sastajalište vidi tajne signalnog poziva/odziva u pregledanom goCOP obliku; ovaj HMAC nije zaštita od zlonamjernog sastajališta. Konačna QUIC/TLS autentifikacija očekivanog ključa ostaje obavezna.

## 5. Framing i tokovi

Ne pomiješaj dva goCOP oblika:

| Promet | goCOP oblik |
|---|---|
| Signal i kontrola QUIC sesije | 4 B big-endian duljina + JSON; sadržaj najviše 64 KiB (`okvir.go`) |
| Poslovni razgovor na TLS/QUIC toku | tok JSON `Envelope` poruka (`exchange.go`) s ograničenjem po očekivanoj vrsti |
| QUIC tok | prvi bajt 0x00 kontrola ili 0x01 razgovor; ostali kanali u pregledanom kodu nisu aplikacijski podaci |
| UDP udarac | fiksni binarni paket 60 B; nije JSON i nije QUIC stream |

goCOP JSON dekoder može pročitati unaprijed više poruka, zato računanje ograničenja mora obuhvatiti i već buffirane bajtove. Rust parser mora obraditi fragmentiranu poruku i više poruka u jednom čitanju. `read_line` bez definiranog i ograničenog line framinga nije ekvivalent.

Za vlastiti List2Cart v1 S02 smije odabrati jednostavniji bounded framing i za poslovne poruke. To se označava razlikom; onda Go testni adapter govori taj List2Cart okvir, a ne izvorni goCOP razgovor. Ne mijenjaj framing usred implementacije bez nove ugovorene inačice.

Provjeri duljinu prije alokacije. Kontrolne poruke, metapodaci, delta i blob imaju zasebne granice. Nemoj kopirati goCOP ograničenja stotina MiB opravdana njegovim kartama u mobilnu košaricu. Kad se iscrpi kvota, prekini/odgodi s kodom, ne gomilaj neograničene kanale. Tokio Rustls nakon upisa može tražiti `flush` prije čekanja odgovora; uključi to u test redoslijeda.

## 6. Jedan UDP socket: obavezan prototip

goCOP ima jedan `quic.Transport` i čitač paketa koji nisu QUIC. U Quinnu istraži `AsyncUdpSocket` i endpoint integraciju za zaključanu verziju. To je mjesto integracije, ne dokaz da gotov adapter postoji.

```text
jedan socket / jedan vlasnik primitka
    ├── STUN → čekatelj odgovarajućeg 12 B transaction ID-a
    ├── udarci → ograničena sesija, HMAC i replay provjera
    ├── QUIC → Quinn endpoint
    └── ostalo → odbaci + ograničeno brojilo
```

Slanje STUN-a, udaraca i QUIC-a mora zadržati istu lokalnu utičnicu i NAT mapping. Ne stvaraj novi `UdpSocket` za svaku fazu i ne čitaj socket iz više nepovezanih taskova. `SO_REUSEPORT` nije jamstvo ispravnog razvrstavanja.

Očuvaj/obradi adrese, ECN/dst-IP metapodatke, batch/segment semantiku gdje je koristi odabrani Quinn adapter. Klasifikacija paketa, GSO/GRO i MTU su dio prototipa. Nejasan API rješava se malim mjerenjem i ADR-om, ne improvizacijom sigurnosnih bitova.

## 7. Async model i stroj stanja

`probijanje.go` je dobar predložak: `state + event + time → new_state + actions`. Portiraj ga kao čistu funkciju s enumima. Async izvršitelj obavlja akcije (pošalji, udaraj, pokreni QUIC, otkaži); rezultat vraća s identifikatorom pripreme/pokušaja.

Invariante iz `web_sync.go` i `sesija.go` koje treba zadržati:

- Jedna priprema i jedna aktivna sesija po partneru. Novi rezultat starog pokušaja ne zamjenjuje novu sesiju.
- Istodobni pozivi rješavaju se deterministički po ključu. Sesija sa samim sobom odbijena.
- Pozdrav nakon QUIC rukovanja mora imenovati istu signalnu sesiju; sam otvoreni QUIC nije dovršen WebSync.
- Članstvo/ovlast provjerava se za svaki poslovni razgovor i nakon opoziva; trajna sesija nije trajna dozvola.
- Ograničeni redovi i broj priprema/razgovora. Zajednička kvota vrijedi za sve transportne putove.
- Cancel zatvara socket/stream gdje je to potrebno, a servis čeka završetak djece. Samo ispušten `JoinHandle` ne zaustavlja Tokio task.
- Transakcija baze ne traje dok mreža čeka; obavijest UI-u dolazi tek nakon potvrđenog upisa.

goCOP koristi održavanje NAT-a i rokove sesije prilagođene desktop krugu. Rust mobilni klijent mora imati vlastitu mjerenu politiku baterije i suspend/resume; ne prenositi brojke kao univerzalnu obvezu.

## 8. Frontier nije potpuna potvrda primitka

Primjer koji svaki agent mora moći objasniti:

```text
A je stvorio v10, v20, v30 u istom kanalu.
B je primio v30 preko C, ali još nema v20.
Ako B prijavi samo MAX=v30 i traži >v30, v20 više nikad neće stići.
```

Zato prouči `peers/rupe.go` i povezane granice, ne samo SQL `MAX`. Podaci o rasponima i nedostajućem moraju izdržati crash, filtriranje ovlasti i sažimanje. UUIDv7 ovdje nije niz uzastopnih cijelih brojeva: ne traži „svaki broj između”, nego dogovorene pokrivene intervale i konkretne zahtjeve za popunom.

Potvrda znači stvarno potvrđen commit, ne da je socket primio bajtove. Odvojeno broji odbijene/nepodržane verzije. Kad peer dobije pravo na novi popis, stari visoki frontier nekog drugog popisa istog autora ne smije sakriti povijest novog popisa. Snapshot stariji od zadržane povijesti traži eksplicitni recovery protokol.

## 9. Koje testove prvo otvoriti

| Tema | Go izvori za scenarije |
|---|---|
| ključ i krivi identitet | `razmjena/kljucevi_test.go`, `razmjena_test.go`, `provjera_kljuca_test.go` |
| SAS i članstvo | `razmjena/sas_test.go`, `network_test.go`, `rubovi_test.go`, `opoziv_test.go` |
| framing i ograničenja | `razmjena/prijenos_test.go`, `rubovi_test.go`, `peers/ograde_razgovora_test.go` |
| signal i replay | `razmjena/signal_test.go`, `signal_rubovi_test.go`, `signal_ispravci_test.go`, `poziv_test.go` |
| UDP/NAT automat | `razmjena/stun_test.go`, `kandidati_test.go`, `udarac_test.go`, `udaranje_test.go`, `probijanje_test.go`, `probijanje_natsim_test.go` |
| WebSync sesija | `razmjena/web_sync_pravila_test.go`, `web_sync_sigurnost_test.go`, `web_sync_dijelovi_test.go`, `web_sync_spoj_test.go`, `sesija_test.go` |
| podaci i putevi | `peers/rupe_razmjena_test.go`, `web_sync_razmjena_test.go`, `web_sync_redoslijed_test.go`, `opoziv_rokovi_test.go`, `ledger/compact_test.go` |

Prenesi scenarij i očekivani invariant, ne Go testni framework. Ako izvori evoluiraju, pretraži odgovarajući simbol umjesto pretpostavke da put i dalje vrijedi.

## 10. Kratki kontrolni popis prije tvrdnje „radi”

- Ista bazna i mrežna pravila za paket, LAN, WebSync i tunel.
- A↔B i A→B→C s permutacijama, duplikatima i prekidima daju jednaku dopuštenu projekciju.
- Obiteljski član ne čita privatni kanal, ne piše tuđi popis i ne širi svoja prava lažnom ulogom.
- Krivi ključ, SAS, potpis, replay, nepoznata shema i prevelik okvir imaju dokaz odbijanja.
- Više od jedne stranice delte, nova pretplata, rupe i stari uređaj nakon sažimanja ne gube promjene.
- STUN/udarci/QUIC dijele port; blokirani UDP provjereno koristi rezervni put.
- Restart, promjena mreže i otkazivanje ne ostavljaju taskove, sesije i otvorene testne procese.
- Mobilni nalaz navodi stvarne uređaje i foreground/background granice.
- Kopirani izvor ima provjerenu licencu i atribuciju izvornog projekta; nema AI autorstva u Gitu.

Ovim redom: ugovor → čisti Rust codec/automat → vektori → I/O → integracija → stvarni uređaji. Uspješno rukovanje dokazuje vezu; konvergencija, ovlasti i oporavak dokazuju sinkronizaciju.
