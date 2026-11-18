# Go → Rust fixtures

Generator koristi samo Go standardnu biblioteku (Go 1.24+ za `crypto/hkdf`). Ne uvozi niti mijenja produkcijski goCOP. Algoritmi za SAS i udarce prate `internal/razmjena` referentnog commita navedenog u izvoru; omot verzije zaseban je List2Cart ugovor. Kod je pod EUPL-1.2, vidi korijenski LICENSE. Ključevi su javni sintetički fixturei.

Iz korijena List2Cart projekta:

```sh
go run ./tools/sync-fixtures/main.go > /tmp/list2cart-go-vectors.json
cmp /tmp/list2cart-go-vectors.json crates/list2cart-sync/testdata/go-vectors.json
cargo test -p list2cart-sync wire::vectors
```

Rust testovi čitaju spremljeni JSON; Go toolchain nije runtime ni uvjet običnog Rust testa. Kod namjerne promjene ugovora regeneriraj fixture, pregledaj razliku i dopuni PROTOCOL.md. Nemoj samo zamijeniti očekivane vrijednosti kako bi test postao zelen.
