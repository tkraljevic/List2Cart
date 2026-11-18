# List2Cart

**List2Cart** je aplikacija za pametnu kupnju, skeniranje barkodova, kontrolu računa na blagajni i planiranje obiteljske nabavke za **macOS, Windows, Linux, iOS i Android**.

## Privatnost i sinkronizacija

- **100% lokalna pohrana:** Svi podaci (računi, cijene, trgovine, slike artikala i popisi za kupnju) spremaju se lokalno u SQLite bazu na vašem uređaju, bez obveznih vanjskih poslužitelja u oblaku.
- **Razmjena `.list2cart` paketa i provjera uloga (0.0.1-alfa):**
  - **Vlastiti uređaji (`own_device`):** Paket sadrži sve podatke — spremljene račune, katalog proizvoda, trgovine, popise za kupnju i knjigu verzija (`record_versions`).
  - **Članovi obitelji (`family_member`):** Paket sadrži isključivo odabrane popise za kupnju (`shared_lists`) te trgovine i artikle povezane s tim popisima, dok osobni računi ostaju privatni.
  - **Evidencija članova i SAS kod:** Lokalno registrirani uređaji i članovi obitelji definiraju dopuštene uloge, SAS kod za provjeru paketa pri uvozu te popis dijeljenih lista. Izravni mrežni P2P transport između uređaja predviđen je u nadolazećim izdanjima.

Razvoj Rust sinkronizacije i provjereni rezultati prate se u [docs/sync/STATUS.md](docs/sync/STATUS.md). Lokalni upisi sada imaju potpune potpisane verzije i trajni identitet uređaja. Privatni ključevi čuvaju se u zasebnim `*.device.key`/`*.network.key` datotekama uz bazu; sama kopija baze ne sadrži ključ za oporavak identiteta. Automatski LAN/WebSync još nije dovršen.

## Što aplikacija omogućuje

- **Skeniranje barkoda i fotografija artikla:** Skenirajte barkod kamerom (putem izvornog Tauri mobilnog skenera na Android/iOS uređajima ili `BarcodeDetector` dekodiranja uživo iz video prijenosa / slike) i uslikajte proizvod kako bi fotografija postala ikona artikla.
- **Kontrola računa u cent (`kom`, `kg`, `g`, `L`, `ml`):** Unesite količinu i cijenu dok kupujete te usporedite zbroj košarice s iznosom na blagajni prije plaćanja. Spremanje računa odvija se atomski unutar SQLite transakcije uz jedinstveni izračun stavki i ukupnog iznosa.
- **Usporedba cijena i trgovine:** Pratite u kojoj je trgovini pojedini artikl najpovoljniji.
- **Excel (`.xlsx`) i ispis:** Izvezite spremljeni račun u Excel tablicu ili ga ispišite.

## Plan izdanja

- **0.0.1 — Alfa** *(trenutno izdanje)*: Temeljna aplikacija, košarica, atomska kontrola i spremanje računa u cent (`kom`, `kg`, `g`, `L`, `ml`), dekodiranje barkoda, katalog, trgovine, planer, knjiga verzija (`record_versions` + `VACUUM`) i sinkronizacija putem `.list2cart` paketa.
- **0.1.0 — Beta**: Izravni mrežni P2P prijenos između uparenih uređaja i prošireno testiranje na svim platformama.
- **1.0.0 — Stabilno**: Potpuno stabilno izdanje za sve podržane platforme.

## Licenca

Otvoreni kod pod **Europskom javnom licencom (EUPL-1.2)** — pogledajte [`LICENSE`](LICENSE) i [`LICENSE_hr.txt`](LICENSE_hr.txt).
