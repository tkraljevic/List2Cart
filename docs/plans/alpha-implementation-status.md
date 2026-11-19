# 0.0.2-alpha — stanje integracije dizajna

Prvi integrirani prolaz. Nije završna potvrda spremnosti izdanja.

## Povezano u aplikaciji

- Pet glavnih odjeljaka na desktopu i mobitelu; Popisi je početni ekran.
- Katalog, Cijene i Trgovine kao kartice odjeljka Proizvodi.
- Izvorna ikona iz FigCraft prve verzije, lokalni PNG.
- Kartice postojećih popisa s procjenom iz stvarnih stavki i odabirom popisa.
- Uređivanje količine/jedinice/cijene stavke u košarici, pregled novog iznosa, validacija i odustajanje.
- Novi moduli obitelji, člana, uparivanja, sinkronizacije, duplikata i O aplikaciji povezani s postojećim modelom i događajima.
- Zaseban pregled sinkronizacije dostupan preko Obitelj → Tijek sinkronizacije.
- Verzija O aplikaciji iz package.json, licenca iz Cargo.toml kroz Vite define.
- Testovi drugog agenta src/**/*.test.ts izvršavaju se Bunom; izuzeti iz produkcijskog tsc zbog bun:test tipova.

## Provjereno

- npm run build prolazi nakon integracije.
- bun test tools src/ui: 21 test prolazi, bez pogrešaka.
- git diff --check prolazi.
- U pregledniku: početni Popisi, pet odjeljaka, kartice proizvoda → Trgovine.
- Uređivanje testne stavke: 1 × 1,29 € → 2 × 1,29 € = 2,58 €; količina nula odbijena; odustajanje čuva zadnju spremljenu količinu.
- Obitelj → O aplikaciji → povratak; stvarna verzija 0.0.2-alpha i licenca EUPL-1.2.
- Obitelj → Tijek sinkronizacije → povratak; bez uređaja prikazano prazno stanje, bez lažne potvrde.
- Pridruži se drugoj mreži otvara postojeći LAN dijalog; zatvaranje bez aktivne sesije radi.
- Mobilni prikaz 390 × 844: Kupnja, uređivač stavke i O aplikaciji; za O aplikaciji širina dokumenta 390, bez vodoravnog preljeva. Desktop prikaz pregledan.
- Snimke: docs/design/implementation-alpha/.

## Još potrebno prije izdanja

- Provjera glavnog toka s pravom Tauri bazom: više popisa → kupnja → spremanje računa → povijest, javni cjenici i CSV uvoz. Web prikaz koristi već postojeće fallback podatke i nema sve native naredbe; to nije dokaz native spremanja.
- Uparivanje i sinkronizacija na dva stvarna procesa/uređaja, uključujući pogrešan kod, prekid veze i odustajanje tijekom aktivne sesije.
- Otkazivanje sesije sada je povezano s backendom i pokriveno lokalnim TLS testovima; još slijedi ručna provjera na fizičkim uređajima. Prekid ne povlači već poslanu potvrdu.
- Usporedba svih popunjenih trgovačkih ekrana s dizajnom; još nije dovršen puni prijenos svih 60 stanja.
- Završni pregled drugog agenta i njegovih uputa za integraciju, jer je radio paralelno.
- Prava read/write po popisu i granularne faze LAN sesije ne smiju se izmišljati: trenutačni API ih ne izlaže.

Zatečeni diff očuvan; početni snapshot u /private/tmp/list2cart-alpha-baseline/pre-implementation.diff. Nema commita, pusha ili releasea.


Dopuna nakon kontrole: sva tri nalaza iz alpha-agent-review.md popravljena. Pauza je jasno označena kao pauza automatskog pokretanja; odustajanje stvarno prekida listener/pending/confirm; poruke confirming/cancelling su razdvojene. Zakašnjeli IPC odgovori ne vraćaju zatvorenu sesiju. Završno: 23 frontend testa i 119 Rust testova prolaze, uz build i Tauri cargo check. Detalji i ograničenja u alpha-agent-review.md.
