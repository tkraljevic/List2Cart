# Zadaci za implementacijskog agenta

Čitaj uz [PLAN.md](PLAN.md) i [GOCOP_RUST_CHEATSHEET.md](GOCOP_RUST_CHEATSHEET.md). Dokument čuva izvorni nalog i izlazne uvjete; aktualni napredak i opaženi rezultati su u [STATUS.md](STATUS.md).

## Radni ugovor

- Prije svake faze pročitaj aktualne datoteke, upute i Git status. Sačuvaj postojeće izmjene; ne resetiraj radnu kopiju. Za novu granu koristi `codex/` prefiks.
- EMM na početku i kratki digest na kraju; tilth za pregled koda, RTK za ostale naredbe. Ne stavljaj privatne ključeve u memoriju ili izvještaj.
- goCOP i syncnet pregledavati kao izvore; ne mijenjati ih radi ovog zadatka. Testne prilagodbe izvuci u List2Cart testno područje uz očuvanje pripadnih obavijesti o licenci.
- Jedan zadatak = mali pregledljiv zahvat s opaženim dokazom. Ne označavaj dovršenim zato što kompajlira. Nema AI autorstva/koautorstva u commitima.
- Testovi rade na privremenim bazama i testnim ključevima. Mrežne testove ograniči rokovima i izoliranim adresama. EMM testove, ako ih uopće bude, usmjeri na kopiju baze.
- Automatsko podizanje javnog servisa, objava i izmjena produkcijske mreže nisu dio zadataka. Lokalni razvojni serveri imaju kontrolirano gašenje.

## S00 — Baseline i evidencija aktualnog stanja

**Ovisnosti:** nema. **Izvori:** postojeći workspace, `xtask`, globalni standard kvalitete #2845.

- Zabilježi commit, status izmjena, toolchain, OS, postojeće testove i način pokretanja. Popiši kritične funkcije i već dovršene dijelove uvoza.
- Pokreni formatiranje bez pisanja, frontend build, workspace testove i strogi Clippy. Evidentiraj postojeće padove zasebno od novih regresija. Provjeri dostupnost cargo-deny i postojeću konfiguraciju; ne proglasi preskočenu provjeru uspješnom.
- Za kritične sync module zabilježi dostupni coverage i alat; ako nedostaje, izričito navedi da nije mjeren. Ne uvodi alat ili prag samo radi brojke.
- Izradi `docs/sync/BASELINE.md` i popis migracija. Provjeri lokalni goCOP HEAD i promjene relevantnih datoteka, ne samo ovog plana.

**Gotovo kada:** postoji ponovljiv zapis stvarnih naredbi/rezultata i točan popis dijelova koje treba nadograditi. Tuđe izmjene ostaju sačuvane.

## S01 — Rani Rust transport prototip

**Ovisnosti:** S00. **Izvori:** goCOP `razmjena/udp.go`, `udarac.go`, `quic_slusalica.go`, Quinn API.

- Odaberi kompatibilne verzije Rust ovisnosti, MSRV, crypto provider i ciljne platforme. Zaključaj ih u Cargo.lock; nemoj kopirati API iz primjera druge glavne verzije.
- Na jednom UDP portu po čvoru demonstriraj STUN zahtjev/odgovor, validni testni udarac i obostranu QUIC vezu. Jedan komponentni vlasnik čita datagrame.
- Izmjeri ponašanje pri nepoznatim datagramima, opterećenju, otkazivanju i ponovnom otvaranju mreže. Provjeri recv/send metapodatke, wakeup, MTU i isključi nepodržane optimizacije eksplicitno.
- Provjeri barem build minimalnog identiteta/TLS/QUIC dijela za Android i iOS kad su alati dostupni; nedostupan target označi kao rizik.

**Gotovo kada:** izvršiv prototip prenosi sintetičke bajtove, ponavlja test bez izgubljenih čitača/socketa i postoji kratki ADR s odabranim UDP adapterom. Neuspjeh vodi odluci o alternativi, ne tihom korištenju dvaju portova.

## S02 — Zamrznuti ugovor v1 i testni vektori

**Ovisnosti:** S00; transportni dio uzima rezultat S01. **Izvori:** cheatsheet, goCOP `keys.go`, `network.go`, `poziv.go`, `okvir.go`, `peers/razgovor.go`.

- Izradi `PROTOCOL.md`: ime protokola, ALPN, DTO, byte/base64 pravila, kanali, stabilni ID-evi, veličine poruka, mogućnosti i odbijanje nepoznate obvezne sheme.
- Izradi ADR za principal → uređaji, izdavatelje grantova, čitanje/pisanje/administriranje popisa, opoziv i povijesne ovlasti. Opseg starog SAS importa ne smije dati mrežna prava.
- Odredi potpisani omot verzije za prosljeđivanje, točne potpisane bajtove, referencu na ovlast i odvajanje kontrolnih od poslovnih poruka. Označi razlike prema goCOP-u.
- Odredi konflikt izmjena/brisanja, eksplicitnu obnovu, granicu prihvatljivog odstupanja sata i ponašanje nakon starog snapshota.
- Dodaj javne sintetičke Go/Rust vektore za ključeve, SAS, članstvo, signal, HKDF/HMAC i framing. Go harness koristi testni protokol, nikad produkcijski identitet.
- Specifikacija određuje i novi zaštićeni paket na disku: autentikaciju, šifriranje za primatelja i odvojeni legacy import.

**Gotovo kada:** iz istih ulaza oba jezika daju očekivane bajtove ili je razlika dokumentirana kao namjerna List2Cart promjena. Nema neodređenog pravila koje utječe na potpis ili prava. Prije pune izvedbe kriptografskih ugovora provesti tehnički pregled specifikacije.

## S03 — Migracije i identitet podataka

**Ovisnosti:** S02. **Mjesta:** `core/models.rs`, shema baze, novi DTO.

- Dodaj verzionirane migracije; stabilne identifikatore i indekse za sve entitete, povijest cijena i veze. Ugovor ne prenosi lokalne integer ID-eve kao identitet.
- Definiraj ponovljivo preuzimanje postojećih podataka i stare knjige u novi ugovor, bez stvaranja drugačijih ID-eva svakim pokretanjem.
- Lokalno naslijeđeni podaci mogu postati novi lokalni zapisi uz eksplicitnu migraciju; stari uvezeni zapisi ne dobivaju retroaktivno lažan potpis izvornog autora.
- Snapshot/restore ne smije pokrenuti klon izvornog uređaja. Nedostajući ključ zahtijeva novi identitet/onboarding; ne generiraj neprimjetno zamjenski ključ postojećeg člana.

**Gotovo kada:** migracija kopije aktualne baze čuva vrijednosti i veze, drugi prolaz ne mijenja rezultat; rollback ne ostavlja pola migracije. Originalna baza nije korištena za test.

## S04 — Atomski zapis svih lokalnih promjena

**Ovisnosti:** S03. **Izvori:** `ledger.Recorder.RecordIn`, postojeći Rust `record_version`.

- Sve domenske naredbe pišu promjenu i verziju u istu transakciju, uključujući checked, izmjene stavki, račune, cijene i arhiviranje.
- Dodaj tipizirane validatore, provjeru zbrojeva i pravilnu dodjelu kanala. Uvedi tombstone operacije ondje gdje nedostaju.
- Ukloni prešućene pogreške kritičnog upisa; ne radi `VACUUM` u aktivnoj poslovnoj transakciji.

**Gotovo kada:** fault-injection između izmjene i knjige daje potpuni rollback; svaka podržana naredba ima provjeren odgovarajući događaj. Nema dvostrukog zapisa za jednu promjenu.

## S05 — Jedinstveni prijem i projekcija verzija

**Ovisnosti:** S04 i trust ugovor S02. **Izvori:** `ledger.Apply`, `peers/upis.go`; domenska projekcija prilagođava se List2Cartu.

- Uvedi `apply_remote` za validaciju, deduplikaciju, provjeru potpisa/ovlasti i atomsku projekciju u SQLite tablice. Koristi dogovoreni redoslijed i konfliktno pravilo.
- Obradi dijete prije roditelja, više promjena istog zapisa, nepoznatu shemu, pogrešan ID i isti version_id s različitim sadržajem. Zavisne podatke odgodi s evidencijom ili vrati jasno definiran odbijeni rezultat.
- Primljeno ne prepisuj novim lokalnim vremenom niti stvaraj novu izvornu verziju. Brojače uspjeha emitiraj nakon commita.
- Preusmjeri novi paket na isti put; legacy snapshot import izoliraj i ograniči. Ne održavaj dva različita algoritma spajanja za mrežu i datoteku.

**Gotovo kada:** A→B→C i C→A konvergiraju; ponovljeni paket ne mijenja broj verzija ni projekciju; pad projekcije vraća transakciju. Brisanja i stvarna konfliktna pravila pokrivena su testom.

## S06 — Frontier, nastavci, rupe i sažimanje

**Ovisnosti:** S05. **Izvori:** `ledger.Delta`, `peers/rupe.go`, `granica_nakon_uvoza.go`, `granica_odbijenih.go`, `razgovor.go`.

- Slati delte prema tuđem dopuštenom znanju, s granicom po bajtovima i oznakom nastavka. Ukloni pretpostavku da je prvih 5000 kompletan paket.
- Razdvoji primljeno, primijenjeno, odbijeno, nedostajuće i potvrđeno drugom uređaju. Ni MAX UUID ni tuđi frontier sami ne podižu potvrđenu pokrivenost.
- Ponovno dopušten kanal mora dobiti ranije nedostupne zapise. Odbijeni/filtrirani zapisi ne smiju trajno zakloniti dopuštene zapise istog autora.
- Dodaj sigurnu obnovu nakon sažimanja: snapshot/reset epoha ili ekvivalent s dokazom potpunosti. Sačuvaj tombstones i kontrolne zapise koji štite od povrata.

**Gotovo kada:** test s više stranica, kasnim starijim zapisom, promjenom prava i dugo offline C čvorom nema trajnog gubitka podataka. Prekid prije potvrde ne preskače podatke.

## S07 — Prava po primatelju i zaštićeni paketi

**Ovisnosti:** S05–S06, trust S02. **Izvori:** goCOP network/opoziv, List2Cart specifikacija.

- Implementiraj potpisana članstva, grantove i opozive te provjeru po stabilnom principalu/ključu/popisu. Za v1 može jedan ovlašteni administrator mreže; delegiranje je zasebna podržana mogućnost, ne prešutno pravo svih vlastitih uređaja.
- Zamijeni mrežno povjerenje u `claimed_role`, naziv mreže i SAS polje dokazom ključa i ovlasti. Nepoznat ili nepotpun dokaz nema pristup privatnom kanalu.
- Primijeni odabir konkretnog primatelja na DTO, ledger, frontier i fotografije; član A ne nasljeđuje uniju prava A+B. Dodavanje artikla na zajednički popis ne objavljuje njegovu privatnu povijest cijena.
- Implementiraj paket iz S02 provjerenim kriptografskim formatom/bibliotekom, bez vlastite nove šifre. Lokalni legacy import izričito ne mijenja trust stanje.

**Gotovo kada:** negativni testovi za lažni own_device, istoimene popise, neovlašteno pisanje, prosljeđivanje, opoziv i krivog primatelja prolaze. F1 izlaz: tri baze razmjenjuju dopuštene promjene preko paketa bez curenja prava.

## S08 — Ključevi, TLS i stvarni SAS

**Ovisnosti:** S02–S03; integracija prava S07. **Izvori:** `keys.go`, `pair.go`, `provjera_kljuca.go`, pripadni Go testovi.

- Sigurno kreiraj/učitaj trajni Ed25519 ključ izvan baze/paketa. Oštećen ili nečitljiv postojeći ključ prijavi; nikad ga automatski ne prepiši.
- Implementiraj TLS 1.3 s dokazom posjedovanja ključa, provjerom protokola i očekivanog javnog ključa; odvoji pairing način od normalne razmjene.
- Portiraj commit–reveal SAS i obostranu potvrdu; prekid ili odbijanje ne stvara aktivno članstvo. Oporavak jednostrano dovršenog završnog upisa mora biti definiran; mreža ne jamči atomsku potvrdu dviju baza.
- Tauri vraća UI-u samo SAS/javne metapodatke i ishode, ne privatne ključeve.

**Gotovo kada:** dva stvarna procesa vide isti SAS; jedna odbijena potvrda i zamijenjen ključ prekidaju postupak. Fixture vektori i proof-of-possession negativni testovi prolaze.

## S09 — Neovisan protokol razgovora

**Ovisnosti:** S05–S08. **Izvori:** `peers/razgovor.go`, `ograde_razgovora.go`, `razmjena/exchange.go`.

- Implementiraj isti razgovor nad testnim duplex tokom i mrežnom vezom: Hello, granice/mogućnosti, delta, upis, sadržaj i potvrda.
- Bounded framing, ograničenje prije alokacije, očekivani redoslijed poruka, deadline, cancel i numerički kodovi odbijanja.
- Izbjegni deadlock kad oba partnera šalju veliku deltu: redoslijed slanja/primanja po ulozi i flush mora biti definiran i testiran.

**Gotovo kada:** obje uloge prolaze isti skup ugovornih testova; pogrešan redoslijed/oversize/truncated poruka daje kontrolirani prekid bez pogrešnog ACK-a.

## S10 — LAN discovery i automatska lokalna razmjena

**Ovisnosti:** S08–S09 i F1. **Izvori:** goCOP discovery simboli, `exchange.go`, `peers/redoslijed.go`.

- Portiraj otkrivanje adresa s vlastitim protokolom. Beacon daje adresu, ne povjerenje. Omogući ručni unos adrese i kada broadcast ne prolazi.
- Pokreni autentificirani listener/dialer, ograniči rukovanja i razgovore. Jedan razgovor s istim partnerom, kontrolirano gašenje.
- Lokalna promjena pokreće objedinjenu obavijest za razmjenu; ostavi periodičku provjeru kao sigurnosnu mrežu. Ne šalji cijelu bazu pri svakom kliku.

**Gotovo kada:** dva računala u LAN-u se uparuju i automatski izmjenjuju dopuštene promjene; lažni beacon ne prolazi TLS. Za oba računala zabilježeni stvarni ishodi.

## S11 — Fotografije i nedostajući sadržaj

**Ovisnosti:** S07, S09. **Izvori:** goCOP `internal/sadrzaj`, razgovor želje/sadržaj.

- Uvedi content hash, sigurnu lokalnu pohranu i migraciju sadašnjih data URL-ova. Slati samo traženi sadržaj vezan uz dopuštene zapise.
- Ograniči veličinu, provjeri hash i format; privremeni sadržaj potvrdi atomskim preimenovanjem. Prekid ponavlja komad ili cijeli ograničeni objekt prema ugovoru.
- Garbage collection čuva sadržaj na koji upućuju važeći zapisi/potrebna povijest; ne radi ga u mrežnom receive loopu.

**Gotovo kada:** isti sadržaj se ne šalje dvaput; korupcija i neovlašteni hash zahtjev odbijeni; prekid ne ostavlja datoteku koja se smatra dovršenom.

## S12 — Sastajalište i klijent signala

**Ovisnosti:** S01–S02, S07–S10. **Izvori:** `signal*.go`, `sastajaliste.go`, `poziv.go`, `okvir.go`.

- Implementiraj Rust klijent i minimalni self-hosted servis: dostupnost, potpisani poziv/odziv/sinkro, ograničenja, heartbeat i ponovno povezivanje.
- Očuvaj izvorne potpisane bajtove; provjeri potpis, članstvo, cilj, sastajalište, vrijeme i replay cache. Na novom pozivu druga strana daje novu tajnu i nakon restarta.
- Ako se zadržava goCOP oblik, TLS signala unutar WebSocketa zahtijeva byte-stream adapter i vlastiti ALPN. Ako se mijenja, izmjenu specificiraj i testiraj; WSS sam nije jednak ugovor.
- Servis ne pohranjuje poslovne delte niti omogućuje upit svih tuđih privatnih mreža. Klijenti provjeravaju očekivani ključ sastajališta.

**Gotovo kada:** dva izolirana klijenta razmijene signalne poruke preko servisa, ali krivotvorena, ponovljena, pogrešno adresirana ili prevelika poruka bude odbijena. Postoje lokalne upute i čisto gašenje.

## S13 — NAT probijanje i QUIC sesije

**Ovisnosti:** S01, S08–S12. **Izvori:** `netcheck.go`, `kandidati*.go`, `udarac.go`, `udaranje.go`, `probijanje.go`, `web_sync.go`, `sesija.go`.

- Portiraj čisti stroj stanja i fake-clock testove prije async efekata. Dvije tajne, HKDF/HMAC, smjerovi i prozor brojača moraju proći vektore.
- Integriraj dokazani UDP demux, ograničene STUN transakcije i validaciju kandidata. Ne udaraj proizvoljne adrese iz neprovjerenih poruka.
- Dodaj QUIC kontrolni tok/pozdrav vezan uz signalnu sesiju, tok razgovora i ponovnu provjeru ovlasti za svaki razgovor. Opoziv zatvara postojeću sesiju.
- Obradi istodobne pozive po determinističkom ključu, stare rezultate pripreme, promjenu mreže, ograničenja broja sesija i prekid svih child taskova.

**Gotovo kada:** deterministička NAT simulacija i stvarna razmjena između dviju različitih mreža prolaze. UDP blokada završava jasnim rezultatom i dopušta S14; ne visi u beskonačnim pokušajima.

## S14 — Rezervni put i izbor veze

**Ovisnosti:** S10, S12–S13. **Izvori:** `peers/redoslijed.go`, `priprema_weba.go`, `razmjena/tunel.go`, `put.go`.

- Implementiraj LAN prvo, živu WebSync sesiju potom, zatim konfiguriranu javnu/server/VPN adresu ili TLS-over-WebSocket tunel. VPN je put preko OS mreže, ne ugradnja vlastitog VPN proizvoda.
- Jedan pobjednik prolazi autentifikaciju, ostale veze/pripreme se zatvaraju. Ukupni rok i backoff ne smiju se resetirati svakim internim korakom.
- U tunelu ostaje end-to-end provjera ključa uređaja; server ne postaje autoritativna baza i ne dobiva poslovni plaintext.

**Gotovo kada:** s blokiranim UDP-om dvije mreže razmjenjuju preko konfiguriranog rezervnog puta; povratak kući vraća LAN. Dokaz razlikuje korišteni put od samog uspjeha.

## S15 — Tauri servis i korisnički tok

**Ovisnosti:** S10–S14. **Mjesta:** `src-tauri/src/lib.rs`, `src/api.ts`, `src/types.ts`, sync UI.

- Uvedi servis s jasnim start/stop/pause i lifecycle signalima. Događaji nose tipizirano stanje; tajne ostaju u Rustu.
- UI: stvarno uparivanje, konkretnog partnera, odabir popisa/read-write prava, opoziv, zadnji uspjeh, pending, put i razlog pogreške. Ne prikazuj uspjeh iz samog dovršenog HTTP zahtjeva.
- Zaštićeno renderiraj primljene nazive/atribute i postavi primjenjiv CSP. Ukloni reset relevantnih formi pri toast/render događajima. Ovo je sigurnosni preduvjet prikaza tuđih podataka, ne opći redizajn.
- Legacy PocketBase put dokumentiraj/ograniči tako da ne zaobilazi novi zapis promjena i ovlasti; promjenu ponašanja prikaži u migracijskim uputama.

**Gotovo kada:** stvarni tok uparivanje → uređivanje → razmjena → opoziv provjeren u aplikaciji, uz vidljiv točan status i bez privatnih ključeva u IPC-u/logu.

## S16 — Mobilne platforme i oporavak

**Ovisnosti:** prethodni izvršivi slojevi. **Mjesta:** Tauri konfiguracija, mobilni projekti i service lifecycle.

- Provjeri Android/iOS mrežne dozvole i lokalnu mrežu, fizičke uređaje, privatnu pohranu ključa, suspend/resume i promjenu mreže. Ne pretpostavljaj desktop listener ponašanje.
- Foreground i ručni sync su osnovni dokaz. Pozadinske mogućnosti dokumentiraj po platformi na temelju provjere aktualnih službenih pravila; ne obećavaj stalnu dostupnost ugašene/suspendirane aplikacije.
- Restart procesa, nestanak struje, zauzeta baza i nedovršen download imaju oporavak bez gubitka potvrđenih promjena.

**Gotovo kada:** postoje matrica uređaja/OS-a i stvarni rezultati; nedostupan uređaj nije označen zelenim testom. Ključ ne putuje u sync paket ni obični DB backup.

## S17 — Završna regresija i isporuka

**Ovisnosti:** S03–S16. **Izvori:** izlazni uvjeti PLAN-a, Go rubni testovi.

- Automatiziraj tri izolirana čvora: offline konflikti, duplikati, prekinuti prijenos, rupe, nove pretplate, sažimanje, opozivi, pogrešan ključ, pokvaren sadržaj i migracija stare baze.
- Posebno testiraj stvarni transport; prolaz baze u memoriji nije dokaz WebSynca. Javni STUN i stvarni NAT testovi odvojeni su od hermetičkih CI testova.
- Proširi `cargo xtask provjera` na novi paket i Tauri relevantne provjere; dodaj frontend i integracijsku provjeru gdje pripadaju. Glavni CI koristi isti postupak. Pregled licenci, ovisnosti i sigurnosti obuhvaća novo dodane biblioteke.
- Dopuni README, PROTOCOL, postupak sastajališta/tunela, ograničenja mobilnog rada i kratki runbook pogrešaka. Zabilježi izmjerene veličine/potrošnju/vrijeme samo gdje su stvarno mjerene.

**Gotovo kada:** svaki izlazni uvjet PLAN-a ima povezan test/opažanje, poznata ograničenja su vidljiva, nema preostalih testnih procesa i javnih objava izvan ovlasti.

## Predložak izvještaja po zadatku

```text
Zadatak: Sxx — naziv
Promijenjeno: datoteke i novo ponašanje
Referenca: goCOP commit + datoteka/simbol; namjerne razlike
Dokaz: konkretna naredba ili opažanje + rezultat
Regresije i ograničenja: što pada / što nije pokrenuto
Sljedeći zadatak: ID i ispunjene ovisnosti
```

## Početni prompt za agenta

> Implementiraj List2Cart sinkronizaciju prema docs/sync/PLAN.md, AGENT_TASKS.md i GOCOP_RUST_CHEATSHEET.md. Počni sa S00, zatim S01–S02; dalje radi po ovisnostima. Koristi lokalni goCOP kao referencu ugovora i testova, a proizvod izvedi u Rustu s vlastitim List2Cart protokolom. Sačuvaj postojeće izmjene. Prije implementacije provjeri aktualno stanje: popravci paketa možda već postoje. Ne mijenjaj goCOP, ne koristi produkcijske baze/ključeve, ne objavljuj servis. Ne preskači podatkovnu konvergenciju, prava po primatelju, rupe, opoziv i rezervni put radi bržeg mrežnog demoa. Napreduj malim provjerljivim zahvatima; stanje izvještavaj prema opaženim rezultatima.
