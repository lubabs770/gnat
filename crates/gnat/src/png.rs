//! A minimal PNG writer.
//!
//! Only needed so `--snapshot` can produce something viewable, and a whole
//! image crate for one function is not worth the dependency. Deflate is used in
//! "stored" mode — legal, trivially correct, and the files are throwaway
//! diagnostics rather than assets.

use anyhow::Result;
use std::io::Write;
use std::path::Path;

/// Write RGBA8 pixels as a PNG.
pub fn write_rgba(path: impl AsRef<Path>, width: u32, height: u32, rgba: &[u8]) -> Result<()> {
    anyhow::ensure!(
        rgba.len() == (width as usize * height as usize * 4),
        "pixel buffer is {} bytes, expected {}",
        rgba.len(),
        width as usize * height as usize * 4
    );

    let mut out = Vec::new();
    out.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);

    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA, no interlace
    chunk(&mut out, b"IHDR", &ihdr);

    // Each scanline is prefixed with its filter type; 0 means "none".
    let mut raw = Vec::with_capacity((width as usize * 4 + 1) * height as usize);
    for y in 0..height as usize {
        raw.push(0);
        let start = y * width as usize * 4;
        raw.extend_from_slice(&rgba[start..start + width as usize * 4]);
    }
    chunk(&mut out, b"IDAT", &zlib_stored(&raw));
    chunk(&mut out, b"IEND", &[]);

    let mut f = std::fs::File::create(path)?;
    f.write_all(&out)?;
    Ok(())
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// A zlib stream whose deflate payload is entirely uncompressed blocks.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01]; // deflate, 32K window, no preset dict
    // Stored blocks carry a 16-bit length, so the payload is chunked.
    const MAX: usize = 0xFFFF;
    let mut offset = 0;
    loop {
        let end = (offset + MAX).min(data.len());
        let block = &data[offset..end];
        let last = end == data.len();
        out.push(last as u8);
        out.extend_from_slice(&(block.len() as u16).to_le_bytes());
        out.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        out.extend_from_slice(block);
        offset = end;
        if last {
            break;
        }
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_matches_the_known_check_value() {
        // The standard CRC-32 check value for "123456789".
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn adler_matches_the_known_check_value() {
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn a_written_png_has_the_right_signature_and_chunks() {
        let dir = std::env::temp_dir().join("gnat-png-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.png");
        write_rgba(&path, 2, 2, &[255; 16]).unwrap();

        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(
            &bytes[..8],
            &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]
        );
        let has = |tag: &[u8]| bytes.windows(4).any(|w| w == tag);
        assert!(has(b"IHDR") && has(b"IDAT") && has(b"IEND"));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn a_mismatched_buffer_is_refused() {
        let path = std::env::temp_dir().join("gnat-png-bad.png");
        assert!(write_rgba(&path, 4, 4, &[0; 8]).is_err());
    }

    #[test]
    fn large_images_span_multiple_stored_blocks() {
        // One stored block holds 65535 bytes; this needs several.
        let data = vec![7u8; 200_000];
        let z = zlib_stored(&data);
        assert!(z.len() > data.len(), "stored blocks add framing");
        assert_eq!(&z[..2], &[0x78, 0x01]);
    }
}
