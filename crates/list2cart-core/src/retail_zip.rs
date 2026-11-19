//! Reads single files out of a large ZIP on a web server without downloading it.
//!
//! Plodine and Studenac publish every branch's price list in one daily ZIP of
//! 180–250 MB. Only the archive's directory and the one branch's file are fetched,
//! with HTTP range requests. A stored (uncompressed) ZIP inside the ZIP is read the
//! same way, from its offset in the outer file (Studenac).
use reqwest::{Client, StatusCode, header};
use std::io::Read;

/// Longest a ZIP's end record can be: 22 bytes plus a comment of up to 64 KiB.
const EOCD_SEARCH: u64 = 22 + 65_535;
const MAX_DIRECTORY: u64 = 8_000_000;
pub(crate) const MAX_ENTRY: u64 = 12_000_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ZipEntry {
    pub name: String,
    pub method: u16,
    pub compressed_size: u64,
    pub uncompressed_size: u64,
    /// Offset of the entry's local header, from the start of its own archive.
    pub header_offset: u64,
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// Finds the end-of-central-directory record in the archive's last bytes and
/// returns (directory offset, directory size). ZIP64 archives are refused.
pub(crate) fn find_directory(tail: &[u8]) -> Result<(u64, u64), String> {
    let at = (0..tail.len().saturating_sub(21))
        .rev()
        .find(|&i| tail[i..].starts_with(&[0x50, 0x4b, 0x05, 0x06]))
        .ok_or("Arhiva cjenika nema ispravan zavrsetak")?;
    let size = u32_at(tail, at + 12).ok_or("Neispravna arhiva cjenika")?;
    let offset = u32_at(tail, at + 16).ok_or("Neispravna arhiva cjenika")?;
    if size == u32::MAX || offset == u32::MAX {
        return Err("Arhiva cjenika je prevelika (ZIP64)".into());
    }
    Ok((offset.into(), size.into()))
}

/// Names written without the UTF-8 flag are tried as UTF-8 first (most tools write
/// it), then as the DOS code page 852 used for Croatian letters.
fn entry_name(raw: &[u8], utf8_flag: bool) -> String {
    match std::str::from_utf8(raw) {
        Ok(s) => s.to_owned(),
        Err(_) if utf8_flag => String::from_utf8_lossy(raw).into_owned(),
        Err(_) => raw.iter().map(|&b| cp852(b)).collect(),
    }
}
fn cp852(b: u8) -> char {
    const HIGH: &str = "ÇüéâäůćçłëŐőîŹÄĆÉĹĺôöĽľŚśÖÜŤťŁ×čáíóúĄąŽžĘę¬źČş«»░▒▓│┤ÁÂĚŞ╣║╗╝Żż┐└┴┬├─┼Ăă╚╔╩╦╠═╬¤đĐĎËďŇÍÎě┘┌█▄ŢŮ▀ÓßÔŃńňŠšŔÚŕŰýÝţ´\u{ad}˝˛ˇ˘§÷¸°¨˙űŘř■\u{a0}";
    if b < 0x80 {
        b as char
    } else {
        HIGH.chars().nth(usize::from(b - 0x80)).unwrap_or('?')
    }
}

pub(crate) fn parse_directory(directory: &[u8]) -> Result<Vec<ZipEntry>, String> {
    let mut entries = Vec::new();
    let mut at = 0;
    while directory.get(at..at + 4) == Some(&[0x50, 0x4b, 0x01, 0x02]) {
        let bad = || "Neispravan popis datoteka u arhivi cjenika".to_string();
        let flags = u16_at(directory, at + 8).ok_or_else(bad)?;
        let method = u16_at(directory, at + 10).ok_or_else(bad)?;
        let compressed_size = u32_at(directory, at + 20).ok_or_else(bad)?;
        let uncompressed_size = u32_at(directory, at + 24).ok_or_else(bad)?;
        let name_len = usize::from(u16_at(directory, at + 28).ok_or_else(bad)?);
        let extra_len = usize::from(u16_at(directory, at + 30).ok_or_else(bad)?);
        let comment_len = usize::from(u16_at(directory, at + 32).ok_or_else(bad)?);
        let header_offset = u32_at(directory, at + 42).ok_or_else(bad)?;
        let name = directory.get(at + 46..at + 46 + name_len).ok_or_else(bad)?;
        entries.push(ZipEntry {
            name: entry_name(name, flags & 0x0800 != 0),
            method,
            compressed_size: compressed_size.into(),
            uncompressed_size: uncompressed_size.into(),
            header_offset: header_offset.into(),
        });
        at += 46 + name_len + extra_len + comment_len;
    }
    if entries.is_empty() {
        return Err("Arhiva cjenika je prazna".into());
    }
    Ok(entries)
}

/// Where an entry's data starts, read from its local header.
pub(crate) fn data_start(local_header: &[u8]) -> Result<u64, String> {
    if !local_header.starts_with(&[0x50, 0x4b, 0x03, 0x04]) {
        return Err("Neispravna datoteka u arhivi cjenika".into());
    }
    let name_len = u16_at(local_header, 26).ok_or("Neispravna arhiva")?;
    let extra_len = u16_at(local_header, 28).ok_or("Neispravna arhiva")?;
    Ok(30 + u64::from(name_len) + u64::from(extra_len))
}

pub(crate) fn unpack(entry: &ZipEntry, data: &[u8]) -> Result<Vec<u8>, String> {
    if entry.uncompressed_size > MAX_ENTRY {
        return Err("Cjenik poslovnice je prevelik".into());
    }
    match entry.method {
        0 => Ok(data.to_vec()),
        8 => {
            let mut out = Vec::with_capacity(entry.uncompressed_size as usize);
            flate2::read::DeflateDecoder::new(data)
                .take(MAX_ENTRY + 1)
                .read_to_end(&mut out)
                .map_err(|e| format!("Cjenik se ne moze raspakirati: {e}"))?;
            if out.len() as u64 > MAX_ENTRY {
                return Err("Cjenik poslovnice je prevelik".into());
            }
            Ok(out)
        }
        _ => Err("Nepodrzana kompresija u arhivi cjenika".into()),
    }
}

/// One byte range of a remote file; returns the bytes and the file's total size.
async fn range(
    client: &Client,
    url: &str,
    spec: String,
    limit: u64,
) -> Result<(Vec<u8>, u64), String> {
    let response = client
        .get(url)
        .header(header::RANGE, format!("bytes={spec}"))
        .send()
        .await
        .map_err(|e| format!("Dohvat cjenika nije uspio: {e}"))?;
    if response.status() != StatusCode::PARTIAL_CONTENT {
        return Err(format!(
            "Trgovina ne dopusta citanje dijela arhive ({})",
            response.status()
        ));
    }
    let total = response
        .headers()
        .get(header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.rsplit('/').next())
        .and_then(|v| v.parse::<u64>().ok())
        .ok_or("Trgovina nije javila velicinu arhive")?;
    let bytes = response.bytes().await.map_err(|e| e.to_string())?;
    if bytes.len() as u64 > limit {
        return Err("Dio arhive je veci od ocekivanog".into());
    }
    Ok((bytes.to_vec(), total))
}

/// A ZIP inside a remote file: the whole file, or a stored ZIP within another one.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Archive {
    pub base: u64,
    pub size: u64,
}

pub(crate) async fn open(client: &Client, url: &str) -> Result<Archive, String> {
    let (_, size) = range(client, url, "0-0".into(), 1).await?;
    Ok(Archive { base: 0, size })
}

pub(crate) async fn list(
    client: &Client,
    url: &str,
    archive: Archive,
) -> Result<Vec<ZipEntry>, String> {
    let tail_len = archive.size.min(EOCD_SEARCH);
    let tail_start = archive.base + archive.size - tail_len;
    let (tail, _) = range(
        client,
        url,
        format!("{tail_start}-{}", archive.base + archive.size - 1),
        tail_len,
    )
    .await?;
    let (offset, size) = find_directory(&tail)?;
    if size == 0 || size > MAX_DIRECTORY || offset + size > archive.size {
        return Err("Neispravan popis datoteka u arhivi cjenika".into());
    }
    let start = archive.base + offset;
    let (directory, _) = range(client, url, format!("{start}-{}", start + size - 1), size).await?;
    parse_directory(&directory)
}

/// Where an entry's (possibly compressed) bytes lie in the remote file.
pub(crate) async fn locate(
    client: &Client,
    url: &str,
    archive: Archive,
    entry: &ZipEntry,
) -> Result<Archive, String> {
    let header_at = archive.base + entry.header_offset;
    // A local header is 30 bytes plus a name and extra field of at most 64 KiB each.
    let (local, _) = range(
        client,
        url,
        format!("{header_at}-{}", header_at + 29 + 1024),
        30 + 1024 + 1,
    )
    .await?;
    let start = header_at + data_start(&local)?;
    if start + entry.compressed_size > archive.base + archive.size {
        return Err("Datoteka izlazi izvan arhive cjenika".into());
    }
    Ok(Archive {
        base: start,
        size: entry.compressed_size,
    })
}

pub(crate) async fn read(
    client: &Client,
    url: &str,
    data: Archive,
    entry: &ZipEntry,
) -> Result<Vec<u8>, String> {
    if data.size > MAX_ENTRY {
        return Err("Cjenik poslovnice je prevelik".into());
    }
    if data.size == 0 {
        return unpack(entry, &[]);
    }
    let (bytes, _) = range(
        client,
        url,
        format!("{}-{}", data.base, data.base + data.size - 1),
        data.size,
    )
    .await?;
    unpack(entry, &bytes)
}

/// The entry's bytes as stored, still compressed, for a file too big to unpack whole
/// (cijene.dev's prices of every branch of a chain run to tens of megabytes).
pub(crate) async fn read_raw(
    client: &Client,
    url: &str,
    data: Archive,
    limit: u64,
) -> Result<Vec<u8>, String> {
    if data.size > limit {
        return Err("Cjenik trgovackog lanca je prevelik za preuzimanje".into());
    }
    if data.size == 0 {
        return Ok(Vec::new());
    }
    let (bytes, _) = range(
        client,
        url,
        format!("{}-{}", data.base, data.base + data.size - 1),
        data.size,
    )
    .await?;
    Ok(bytes)
}

/// Reads an entry's stored bytes as a stream, unpacking as it goes.
pub(crate) fn stream<'a>(entry: &ZipEntry, raw: &'a [u8]) -> Result<Box<dyn Read + 'a>, String> {
    match entry.method {
        0 => Ok(Box::new(raw)),
        8 => Ok(Box::new(flate2::read::DeflateDecoder::new(raw))),
        _ => Err("Nepodrzana kompresija u arhivi cjenika".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn archive(files: &[(&str, &[u8], zip::CompressionMethod)]) -> Vec<u8> {
        let mut out = std::io::Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(&mut out);
        for (name, body, method) in files {
            writer
                .start_file(
                    *name,
                    zip::write::SimpleFileOptions::default().compression_method(*method),
                )
                .unwrap();
            writer.write_all(body).unwrap();
        }
        writer.finish().unwrap();
        out.into_inner()
    }

    fn read_local(bytes: &[u8], base: usize, entry: &ZipEntry) -> Vec<u8> {
        let header = base + entry.header_offset as usize;
        let start = header + data_start(&bytes[header..]).unwrap() as usize;
        unpack(entry, &bytes[start..start + entry.compressed_size as usize]).unwrap()
    }

    #[test]
    fn reads_one_file_and_a_file_inside_a_stored_inner_archive() {
        let csv = "naziv;MPC\nMlijeko;,79\n".repeat(50);
        let inner = archive(&[(
            "Prodavaonica-Split.xml",
            b"<Proizvodi/>",
            zip::CompressionMethod::Deflated,
        )]);
        let outer = archive(&[
            (
                "SUPER_10000_ZAGREB_18112026.csv",
                csv.as_bytes(),
                zip::CompressionMethod::Deflated,
            ),
            ("PROIZVODI_1.zip", &inner, zip::CompressionMethod::Stored),
        ]);
        let (offset, size) = find_directory(&outer).unwrap();
        let entries = parse_directory(&outer[offset as usize..(offset + size) as usize]).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(read_local(&outer, 0, &entries[0]), csv.as_bytes());

        // The inner archive is read from its place in the outer one, like over HTTP.
        let header = entries[1].header_offset as usize;
        let base = header + data_start(&outer[header..]).unwrap() as usize;
        let inner_bytes = &outer[base..base + entries[1].compressed_size as usize];
        let (offset, size) = find_directory(inner_bytes).unwrap();
        let inner_entries =
            parse_directory(&inner_bytes[offset as usize..(offset + size) as usize]).unwrap();
        assert_eq!(inner_entries[0].name, "Prodavaonica-Split.xml");
        assert_eq!(read_local(&outer, base, &inner_entries[0]), b"<Proizvodi/>");
    }

    #[test]
    fn names_without_the_utf8_flag_fall_back_to_code_page_852() {
        assert_eq!(entry_name("Požega".as_bytes(), false), "Požega");
        // "Požega" in code page 852: ž is 0xA7.
        assert_eq!(
            entry_name(&[b'P', b'o', 0xa7, b'e', b'g', b'a'], false),
            "Požega"
        );
        assert!(find_directory(b"not a zip").is_err());
        assert!(data_start(b"PK\x01\x02").is_err());
    }
}
