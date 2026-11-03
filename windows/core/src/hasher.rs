//! Content fingerprints (BLAKE3). Same hash = same bytes = same file.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use anyhow::Result;

use crate::progress::Cancel;

/// Bytes read from each end for the quick hash.
pub const SAMPLE: u64 = 128 * 1024;

/// Size + first 128 KiB + last 128 KiB. Only a pre-filter: equal quick hashes can still
/// differ in the middle, so the verdict always comes from [`full_hash`].
pub fn quick_hash(path: &Path, size: u64) -> std::io::Result<String> {
    let mut f = File::open(path)?;
    let mut h = blake3::Hasher::new();
    h.update(&size.to_le_bytes());
    let mut buf = vec![0u8; SAMPLE as usize];
    let n = read_up_to(&mut f, &mut buf)?;
    h.update(&buf[..n]);
    if size > SAMPLE * 2 {
        f.seek(SeekFrom::Start(size - SAMPLE))?;
        let n = read_up_to(&mut f, &mut buf)?;
        h.update(&buf[..n]);
    }
    Ok(h.finalize().to_hex().to_string())
}

/// BLAKE3 of the whole content (hex). Checks `cancel` every 8 MiB.
pub fn full_hash(path: &Path, cancel: &Cancel) -> Result<String> {
    let mut f = File::open(path)?;
    let mut h = blake3::Hasher::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut since_check = 0usize;
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        since_check += n;
        if since_check >= 8 << 20 {
            since_check = 0;
            cancel.check()?;
        }
    }
    Ok(h.finalize().to_hex().to_string())
}

fn read_up_to(f: &mut File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut total = 0;
    while total < buf.len() {
        match f.read(&mut buf[total..]) {
            Ok(0) => break,
            Ok(n) => total += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_content_equal_hash() {
        let d = tempfile::Builder::new()
            .prefix("mf-hash")
            .tempdir()
            .unwrap();
        let a = d.path().join("a.txt");
        let b = d.path().join("b.txt");
        let c = d.path().join("c.txt");
        std::fs::write(&a, "hello world").unwrap();
        std::fs::write(&b, "hello world").unwrap();
        std::fs::write(&c, "hello worlD").unwrap();
        let none = Cancel::new();
        assert_eq!(full_hash(&a, &none).unwrap(), full_hash(&b, &none).unwrap());
        assert_ne!(full_hash(&a, &none).unwrap(), full_hash(&c, &none).unwrap());
        assert_eq!(quick_hash(&a, 11).unwrap(), quick_hash(&b, 11).unwrap());
        assert_eq!(full_hash(&a, &none).unwrap().len(), 64);
    }

    #[test]
    fn quick_hash_misses_the_middle_full_hash_does_not() {
        let d = tempfile::Builder::new()
            .prefix("mf-hash")
            .tempdir()
            .unwrap();
        let mut data = vec![0x41u8; 600_000];
        let a = d.path().join("one.bin");
        let b = d.path().join("two.bin");
        std::fs::write(&a, &data).unwrap();
        data[300_000] = 0x42;
        std::fs::write(&b, &data).unwrap();
        assert_eq!(
            quick_hash(&a, 600_000).unwrap(),
            quick_hash(&b, 600_000).unwrap()
        );
        let none = Cancel::new();
        assert_ne!(full_hash(&a, &none).unwrap(), full_hash(&b, &none).unwrap());
    }
}
