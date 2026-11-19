# ADR-002: Model povjerenja, principal-uređaji i prava po popisu (ACL)

Datum: 2026-11-17\
Status: smjer modela prihvaćen (`S02`); izvedba ovlasti/oporavka (`S07`) djelomična i nije spremna za mrežnu produkciju.

## Kontekst

goCOP modelira povjerenje preko `NetworkKey`, delegirane `Ovlast` uloge, `Membership` certifikata i `Opoziv` zapisa, pri čemu su svi čvorovi jedne mreže ravnopravni ili podijeljeni po organizacijskim sektorima. List2Cart ima drukčiji zahtjev privatnosti:
1. Korisnik ima **više vlastitih uređaja** koji dijele osobne račune, cijene i privatne popise (`private:<principal_id>`).
2. Korisnik dijeli odabrane **zajedničke popise** s članovima obitelji (`list:<list_sync_id>`), pri čemu član obitelji nikada ne smije čitati osobne račune niti privatnu povijest cijena, niti sam sebi dodijeliti ulogu `own_device`.
3. Dva popisa s istim nazivom (npr. dva `"Subota"` popisa) imaju različite `sync_id` i ne smiju dijeliti ovlasti.

## Odluke

1. **Principal → uređaji**:
   - Svaka osoba ima stabilni `principal_id` (izvedba koristi prvih 16 B SHA-256 korijenskog javnog ključa kao hex za početnu osobu; vezanje dodatnih osoba određuje potpisano članstvo).
   - Novi uređaj stječe `OwnDevice` članstvo isključivo ako je `Membership` potpisan od strane `NetworkKey` ili ovlaštenog `Ovlast` administratora s eksplicitnom ulogom `OwnDevice` i pripadnim `principal_id`. Samoprijava iz JSON-a ili UI padajućeg izbornika odbija se.
2. **Izdavatelji grantova i delegacija**:
   - U v1 korijenski `NetworkKey` ili izričito ovlašteni administrator s valjanom `Ovlast` potvrdom izdaje `ListGrant` zapise.
   - Običan `FamilyMember` i običan `OwnDevice` bez `Ovlast` delegacije ne mogu izdavati nova članstva ni grantove.
3. **Granularni `ListGrant` po `list_sync_id`**:
   - Svaki grant veže točno: `network`, `grant_id`, `list_sync_id`, `recipient_pub_key`, `permission` (`Read` ili `Write`), `issued_at`, `expires_at`, `issued_by`, `signature` (i opcijski `admitter_ovlast`).
   - Pravo čitanja (`Read`) dopušta čvoru da preuzme verzije kanala `list:<list_sync_id>`, ali njegove vlastite mutacije na tom kanalu primatelji odbijaju.
   - Pravo pisanja (`Write`) dopušta dodavanje, označavanje (`checked`) i uređivanje stavki na tom konkretnom `list_sync_id`.
   - Sama uloga `OwnDevice` ne otvara sve zajedničke popise drugih osoba. Osim korijenskog administratora, svaki ključ treba konkretan grant i njegov `grant_ref` pri pisanju. Roditeljski ID stavke mora odgovarati kanalu, a identitet postojeće cjeline ne smije se preseliti u drugi kanal.
4. **Prosljeđivanje (A → B → C)**:
   - Verzije nose potpis izvornog autora (`author_pub_key`) i referencu ovlasti (`grant_ref`).
   - Posrednik B može proslijediti A-ove verzije kanala `list:<id>` čvoru C samo ako C ima pravo na `list:<id>`, a C sam verificira A-ov potpis i A-ovu ovlast u trenutku primjene. B nikada ne može podići svoja prava ni proslijediti A-ov `private:` kanal.
   - Obiteljske delta stranice nose odabrane potpisane kontrolne dokaze za dopuštene kanale i njihove autore. Skup se računa prije budžeta i ponavlja uz svaku stranicu, bez ACK-a globalnog kontrolnog kanala. Grant/opoziv primatelja ne preskače se njegovim starim kontrolnim frontierom. Vlastiti uređaj drugog principala također koristi ograničen pogled. Veliki skup može prijeći na izričiti trajni snapshot formata 2: dokazi i podaci dijele se na staging stranice i primjenjuju zajedno tek nakon potpune provjere. Relevantna promjena prava blokira stari izlaz, uključujući novi grant za isti popis. Automatski obiteljski dispatcher koristi potpisani zahtjev v3: promjene prenosi ograničenim snapshotom, nepromijenjeno potvrđeno stanje praznim odgovorom, a izvedeni izlaz briše tek nakon potpisanog completion ACK-a.
5. **Opoziv i granice opoziva**:
   - `Revocation` (`opoziv/1`) s potpisom `NetworkKey` ili ovlaštenog izdavatelja poništava članstvo ili pojedini `ListGrant` za sve buduće razmjene čim čvor primi taj kontrolni zapis.
   - Kontrolni zapisi (`channel == "control"`) uvijek se primjenjuju **prije** domenskih zapisa unutar istog paketa ili razgovora.
   - Opoziv ne briše podatke koji su već ranije preuzeti na tuđi uređaj prije nego što je opoziv stigao (poznata fizikalna granica offline P2P sustava, jasno dokumentirana korisniku).
   - Opseg opoziva je eksplicitan: `clanstvo`/`membership` blokira ključ uređaja, `list_grant` samo točan grant i ključ, a `ovlast` delegaciju administratora i prava izdana preko nje. Opoziv jednog popisa ne ukida drugi popis istom uređaju. Korijenski mrežni ključ nije uređajsko članstvo i ne može se opozvati tim putem.
   - Za povijesnu verziju provjerava se potpisano članstvo i grant u trenutku nastanka verzije, a administratorski omot u trenutku njegova potpisa. Dokazi se traže u trenutnom cacheu i očuvanoj potpisanoj kontrolnoj knjizi. Najnovije izdavanje koje prethodi promjeni ima prednost; njegov istek ili niža uloga ne vraćaju starije šire pravo. Isti trenutak izdavanja s različitim sadržajem je dvosmislen i odbija se.
   - Konzervativno pravilo nakon opoziva: opozvani ključ, grant ili delegacija ne prihvaćaju se ni za staru verziju. Sam istek nije opoziv: istekla potvrda može dokazati staru promjenu, ali ne otvara današnji pristup. Potrebni dokazi prate delta stranice ili trajni snapshot; automatski obiteljski oporavak koristi isti put uz preostali pregled vremenskih promjena i opozvanih povijesnih autora.
   - Grant ID je nepromjenjiv; vraćanje pristupa zahtijeva novi grant ID. Opozvan uređaj zahtijeva novi ključ. Novije članstvo istog ključa ne može promijeniti principal, a staro članstvo ne može prepisati novije. Kontrolni cache i njegove promjene opoziva imaju atomski savepoint.
6. **Izolacija legacy v1 importa**:
   - Uvoz starog `.list2cart.json` formata (bez kriptografskog potpisa) tretira se isključivo kao lokalni uvoz za uređaj na kojem ga je korisnik ručno pokrenuo i nikada ne upisuje zapise u `sync_trust_records`.

## Dopuna 2026-11-19: član obitelji i zajednički popisi

Test s tri uređaja u istoj mreži (vlasnik, njegov drugi uređaj, član obitelji) pokazao je da član obitelji nije mogao stvoriti ni popis ni račun, a zajednički popis do njega nikad nije stigao. Odluke:

1. **Vlastiti privatni kanal piše svaki član.** `private:<principal_id>` smije pisati uređaj čija potpisana uloga nosi taj `principal_id`, bez obzira je li `OwnDevice` ili `FamilyMember`. Član obitelji ima vlastiti principal, nikad vlasnikov, a taj se kanal šalje samo uređajima istog principala, pa njegovi računi i popisi ostaju njegovi.
2. **Zajednički popis nastaje u kanalu `list:<list_sync_id>`.** Stvara ga uređaj s mrežnim ključem ili `Ovlast` delegacijom (`Database::create_shared_shopping_list`). Pri stvaranju potpisuje `Write` grant sebi i svakom aktivnom članu mreže. Postojeći privatni popis se ne premješta (točka 3 izvornih odluka); dijeli se novim zajedničkim popisom.
3. **Uparivanje predaje dokaze koje razgovor ne bi isporučio.** Razgovor šalje samo verzije novije od onih koje primatelj već ima, a paket dobrodošlice primatelju daje novije. Zato `OwnDevice` pri uparivanju dobiva cijelu potpisanu kontrolnu povijest, a svaki novi član grantove svih postojećih zajedničkih popisa (svoj i tuđe). Primatelj ih odmah upisuje i u kontrolni cache.
4. **Obiteljski prijenos smije donijeti grant koji primatelj još nije vidio.** Početna provjera prijenosa (`only_unseen_grants`) propušta kanal `list:<id>` samo ako za tu stranu i taj popis lokalno ne postoji nikakav zapis o grantu. Grant koji je primatelj već vidio pa izgubio (opoziv, istek) i dalje odbija prijenos, a puna provjera ponavlja se nakon primjene dokaza.

Testovi: `owner_and_own_device_use_the_app_and_see_each_others_lists_and_receipts`, `family_member_uses_the_app_privately_and_shares_only_the_shared_list` i `a_list_shared_after_pairing_reaches_every_member_and_only_the_owner_can_share` u `session.rs`.
