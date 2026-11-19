# Postupci izrade

Aktivni postupak je `.github/workflows/build-alpha.yml`: Windows, Linux i Android paketi spremaju se kao GitHub Actions artefakti. Dostupan je ručno i za build grane. Ne objavljuje GitHub Release i ne uključuje iOS.

`izdanje-legacy.yml` arhiva je prethodnog postupka, izvan `.github/workflows` pa se ne izvršava. Arhiviran je jer je svaki tag pokretao iOS i koristio nazive paketa 0.0.1. Ne vraćati ga bez pregleda verzija, potpisivanja i eksplicitne odluke o iOS-u.

Slanje taga samo označava verziju izvornog koda; ne pokreće ovu arhiviranu automatizaciju niti automatski objavljuje instalacije.
