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
