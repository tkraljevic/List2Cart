// SPDX-License-Identifier: EUPL-1.2
// Independent fixture generator for List2Cart's Rust port of goCOP contracts.
// Reference: goCOP fcca2c6bda5a2c43fa8b051994d4a5ad1a819e7e,
// internal/razmjena/{keys,pair,udarac,poziv,network}.go (EUPL-1.2).
// Uses public synthetic seeds. Never use these keys on a real device.
package main

import (
	"bytes"
	"crypto/ed25519"
	"crypto/hkdf"
	"crypto/hmac"
	"crypto/sha256"
	"encoding/base64"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
)

func must(err error) {
	if err != nil {
		panic(err)
	}
}
func b64(b []byte) string { return base64.StdEncoding.EncodeToString(b) }

func main() {
	const protocol = "list2cart"
	a := ed25519.NewKeyFromSeed(bytes.Repeat([]byte{1}, 32))
	b := ed25519.NewKeyFromSeed(bytes.Repeat([]byte{2}, 32))
	net := ed25519.NewKeyFromSeed(bytes.Repeat([]byte{16}, 32))
	pa, pb, pn := a.Public().(ed25519.PublicKey), b.Public().(ed25519.PublicKey), net.Public().(ed25519.PublicKey)
	caller, listener := make([]byte, 32), make([]byte, 32)
	for j := range caller {
		caller[j], listener[j] = byte(j), byte(0x80+j)
	}
	commit := sha256.Sum256(append([]byte(protocol+"-pair-commit-v2|"), caller...))
	x, y := pa, pb
	if bytes.Compare(x, y) > 0 {
		x, y = y, x
	}
	h := sha256.New()
	for _, part := range [][]byte{[]byte(protocol + "-pair-v2|"), x, y, caller, listener} {
		h.Write(part)
	}
	sas := fmt.Sprintf("%06d", binary.BigEndian.Uint32(h.Sum(nil)[:4])%1000000)

	session, err := hex.DecodeString("00112233445566778899aabbccddeeff")
	must(err)
	direction := byte(1)
	if bytes.Compare(pa, pb) < 0 {
		direction = 0
	}
	info := append([]byte(protocol+"-udarac-v1"), session...)
	info = append(info, direction)
	ikm := append(bytes.Repeat([]byte{0xaa}, 32), bytes.Repeat([]byte{0xbb}, 32)...)
	key, err := hkdf.Key(sha256.New, ikm, nil, string(info), 32)
	must(err)
	packet := make([]byte, 28, 60)
	packet[0], packet[1], packet[2] = 0x10, direction, 1
	copy(packet[4:20], session)
	binary.BigEndian.PutUint64(packet[20:28], 7)
	mac := hmac.New(sha256.New, key)
	mac.Write(packet)
	packet = append(packet, mac.Sum(nil)...)

	ovlast := struct {
		V         string `json:"v"`
		Network   string `json:"network"`
		DeviceID  string `json:"deviceId"`
		DeviceKey string `json:"deviceKey"`
		IssuedBy  string `json:"issuedBy"`
		IssuedAt  int64  `json:"issuedAt"`
		ExpiresAt int64  `json:"expiresAt"`
	}{"ovlast/1", b64(pn), "admin-1", b64(pa), "root", 1760000000, 1760003600}
	ovlastBytes, err := json.Marshal(ovlast)
	must(err)
	signal := []byte(fmt.Sprintf(`{"ver":1,"sesija":"00112233445566778899aabbccddeeff","od":"%s","za":"%s","sastajaliste":"%s","izdano":1760000000}`, b64(pa), b64(pb), b64(pn)))
	signalSignature := ed25519.Sign(a, append([]byte(protocol+"-poziv-v1|"), signal...))
	// List2Cart-specific version envelope, intentionally not goCOP's domain wire format.
	versionTuple := []any{1, "shopping_lists", "list-1", "0199c82c-0000-7000-8000-000000000001", b64(pa), "", false,
		"list:list-1", "2025-10-09T08:53:20Z", int64(1760000000), b64(pa), "grant-1", `{"name":"Subota"}`}
	tuple, err := json.Marshal(versionTuple)
	must(err)
	version := append([]byte("list2cart-ver-v1|"), tuple...)
	output := map[string]string{
		"reference_commit": "fcca2c6bda5a2c43fa8b051994d4a5ad1a819e7e",
		"public_a":         b64(pa), "public_b": b64(pb), "public_network": b64(pn),
		"sas_commit": hex.EncodeToString(commit[:]), "sas": sas,
		"punch_send_key": hex.EncodeToString(key), "punch_packet": hex.EncodeToString(packet),
		"ovlast_bytes": string(ovlastBytes), "ovlast_signature": b64(ed25519.Sign(net, ovlastBytes)),
		"signal_body": string(signal), "signal_signature": b64(signalSignature),
		"version_bytes": string(version), "version_signature": b64(ed25519.Sign(a, version)),
	}
	enc := json.NewEncoder(os.Stdout)
	enc.SetIndent("", "  ")
	enc.SetEscapeHTML(false)
	must(enc.Encode(output))
}
