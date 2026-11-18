# ADR-001: Jedinstveni UDP socket adapter (`AsyncUdpSocket`) za STUN, udarce i QUIC

Datum: 2026-11-17\
Status: Prihvaćeno (`S01`)

## Kontekst

goCOP koristi `quic-go` i `quic.Transport.ReadNonQUICPacket` kako bi isti UDP port i isti NAT mapping nosili:
1. STUN Binding zahtjeve i odgovore (`0x0001` / `0x0101`, gornja dva bita `00`, magic cookie `0x2112A442`).
2. Autentificirane 60-bajtne NAT punch pakete (`0x10` / `0x11`, gornja dva bita `00`, HMAC-SHA256).
3. QUIC datagrame (gornja dva bita prvog bajta različita od `00`, odnosno za duge/kratke zaglavlje pakete koje očekuje Quinn).

U Rustu ne smijemo otvarati zasebne `UdpSocket` instance ni čitati isti socket iz više nepovezanih taskova.

## Odluka i zaključane verzije

Zaključane su sljedeće kompatibilne verzije u `Cargo.lock` (uz `rustc 1.99.1`, `edition = "2024"`):
- `quinn = "0.11.12"` (features: `runtime-tokio`, `rustls-ring`, bez default featurea)
- `rustls = "0.23.46"` (crypto provider: `ring 0.17.14`)
- `tokio = "1.54.1"`, `tokio-util = "0.7.20"`, `tokio-rustls = "0.26.6"`
- `ed25519-dalek = "2.2.0"`, `rcgen = "0.13.2"`, `x509-parser = "0.16.0"`
- `sha2 = "0.10.9"`, `hmac = "0.12.1"`, `hkdf = "0.12.4"`, `chacha20poly1305 = "0.10.1"`

### Odabrani UDP adapter (`crates/list2cart-sync/src/transport/udp_mux.rs`)

Implementiran je `MuxUdpSocket` koji implementira `quinn::AsyncUdpSocket` i predaje se metodi `quinn::Endpoint::new_with_abstract_socket`:
1. **Jedan vlasnik čitanja (`poll_recv`)**: Quinn-ov `EndpointDriver` jedini poziva `MuxUdpSocket::poll_recv`. Unutar petlje se čita datagram preko `tokio::net::UdpSocket::try_recv_from`:
   - ako `is_punch_packet(pkt)` vrijedi, datagram se usmjerava preko ograničenog `mpsc::Sender::try_send` kanala registriranoj sesiji po 16-bajtnom `session_id` (ili povećava `discarded` ako je kanal pun ili sesija nepoznata);
   - ako `is_stun(pkt)` vrijedi, datagram se usmjerava registriranom čekatelju po 12-bajtnom `transaction_id` (ili povećava `discarded`);
   - ako `(pkt[0] & 0xC0) != 0`, datagram se predaje Quinnu kroz `IoSliceMut` i `RecvMeta`;
   - ostali paketi povećavaju `discarded` brojač i odbacuju se bez zauzimanja memorije.
2. **Slanje preko iste utičnice**: QUIC šalje kroz `AsyncUdpSocket::try_send`, a STUN/udarci kroz `UdpMuxHandle::send_non_quic` nad istim `Arc<tokio::net::UdpSocket>`.
3. **Segmentacija i MTU**: Kako bismo osigurali predvidljivo ponašanje na macOS, Windows, Linux, iOS i Android mrežama bez ovisnosti o platformskom GSO/GRO-u pri demuxu, eksplicitno su postavljeni `max_transmit_segments() = 1`, `max_receive_segments() = 1`, `may_fragment() = true`.
4. **Otkazivanje i ponovno otvaranje**: `UdpMuxHandle::close()` odbija nove registracije i aplikacijska slanja, zatvara endpoint i čisti čekatelje. QUIC I/O ostaje dostupan dok se ne isprazne postojeće veze; prerano vraćanje I/O greške ubilo bi driver prije obrade završnih događaja. Tek nakon završetka tokova, `wait_idle()` i otpuštanja svih referenci endpointa/handlea socket se oslobađa. Test provjerava ponovno vezanje baš istog porta; `bind()` ne prikriva zauzet port prelaskom na nasumičan.
5. **Ograde**: najviše 64 registracije po registryju, 256 poruka po redu, najviše 64 datagrama po pollu prije predaje vremena ostalim taskovima. Ponovna registracija istog ID-a odbija se i ne zamjenjuje aktivnog čekatelja.
6. **QUIC fixed bit**: endpoint eksplicitno koristi `grease_quic_bit(false)`. Quinn-proto 0.11.19 zadano oglašava prihvat promjenjivog fixed bita, a `connection/packet_builder.rs` tada nasumično uklanja bit iz poslanih paketa. Kratko zaglavlje s oba gornja bita `00` naš demux bi odbacio. Ova postavka sprječava pregovaranje te mogućnosti; TLS autentikacija i šifriranje time se ne mijenjaju. Nalaz je otkriven padom testa na čekanju zatvaranja server veze, bez promjena UDP koda u prethodnom S06 zahvatu.

## Mobilni / target rizik

`rustup target list --installed` na ovom računalu trenutno sadrži samo `aarch64-apple-darwin`. Izravna provjera kompajliranja za Android/iOS targete (`aarch64-apple-ios`, `aarch64-linux-android`) odgođena je u skladu sa `S01` planom i označena kao otvoreni rizik do `S16` kada budu instalirani mobilni toolchainovi i NDK/Xcode SDK-ovi.

## Dokaz prototipa

Test `transport::udp_mux::tests::single_udp_port_demuxes_stun_punch_quic_and_discards_unknown` na `127.0.0.1` dokazuje na **istom lokalnom UDP portu po čvoru**:
- STUN Binding zahtjev i XOR-MAPPED-ADDRESS odgovor (`203.0.113.7`),
- 60-bajtni HMAC-SHA256 zaštićen udarac (`0x10`) i odgovor (`0x11`) s kliznim prozorom,
- odbacivanje i brojanje nepoznatih ne-QUIC datagrama (`discarded_count >= 1`),
- dvosmjerni QUIC tok (`ping-list2cart` → `pong-list2cart`),
- uredno zatvaranje (`close()`) i ponovno vezanje istog porta.

Naknadno potvrđeno u preuzimanju rada: lokalni scenarij ima ukupni rok 10 s i prolazi nakon ispravka gašenja. To je sintetički QUIC prototip s testnom TLS konfiguracijom, ne dokaz produkcijskog mTLS uparivanja, stvarnog NAT probijanja ili mobilne podrške. Test ne smije biti zamjena za S08/S13/S16.
