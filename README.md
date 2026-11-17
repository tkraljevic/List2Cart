# List2Cart

**List2Cart** je višeplatformska aplikacija za pametnu kupnju, skeniranje barkodova, kontrolu računa u cent na blagajni, usporedbu cijena po trgovinama i obiteljsko planiranje kupnje za **macOS, Windows, Linux, iOS i Android** (Tauri v2 + Rust).

## Ključne značajke

- **Skeniranje barkoda i fotografija proizvoda:** Skenirajte EAN-13 barkod kamerom ili ručno, uslikajte proizvod kako bi fotografija postala ikona artikla te dohvatite naziv i sliku online putem **Open Food Facts API-ja**.
- **Kontrola računa u cent (`kom`, `kg`, `g`, `L`, `ml`):** Unos količine u komadima, gramima, kilogramima, litrama ili mililitrima uz automatsku usporedbu zbroja košarice s iznosom na blagajni (`0,00 €` podudaranje ili izračun razlike).
- **Izvoz u Excel (`.xlsx`) i ispis:** Izvoz računa u formatiranu Excel tablicu (`rust_xlsxwriter`) i izravan ispis.
- **Trgovine i Planer kupnje:** Upravljanje vlastitim trgovinama, praćenje povijesti cijena po dućanima (`★ NAJJEFTINIJE`) i kreiranje zajedničkih popisa za kupnju s prebacivanjem u košaricu.
- **goCOP-stil sinkronizacija i automatsko čišćenje (`record_versions` + `VACUUM`):**
  - Monotona `UUIDv7` knjiga verzija (`record_versions`) s `Frontier` granicom po čvoru i kanalu.
  - Automatsko čišćenje zastarjelih verzija (`auto_compact`) uz `VACUUM; PRAGMA optimize;` za fizičko oslobađanje prostora u SQLite bazi.
  - Uparivanje pomoću **6-znamenkastog SAS koda** s dvije uloge:
    - **Moji osobni uređaji (`osobno + obitelj`)** — dijele **SVE** (račune, katalog, slike, trgovine i sve liste).
    - **Članovi obitelji (`obitelj`)** — dijele **samo zajedničke liste za kupnju**, dok osobni računi ostaju privatni.

## Provjera kvalitete i razvoj

```bash
# Provjera formata, clippy (-D warnings), jedinični testovi i cargo-deny (EUPL-1.2)
cargo xtask provjera

# Pokretanje web sučelja u razvoju
bun run dev

# Pokretanje nativne Tauri v2 aplikacije
bunx tauri dev
```

## Licenca

Ovaj projekt objavljen je pod **Europskom javnom licencom za otvoreni kod (EUPL-1.2)** — pogledajte [`LICENSE`](LICENSE) i hrvatski tekst u [`LICENSE_hr.txt`](LICENSE_hr.txt).
