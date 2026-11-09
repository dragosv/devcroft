//! Private raw-state bootstrap, separate from the immutable/shared initramfs.
use std::fs::File;
use std::io;
use std::os::unix::fs::FileExt;

const OFFSET: u64 = 8192;
const MAGIC: &[u8; 8] = b"DCSHKEY1";
const MAX_RECORD: usize = 16384;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Seed {
    pub schema: u32,
    pub instance_id: String,
    pub host_key: String,
    pub authorized_key: String,
}

fn length(header: &[u8; 12]) -> io::Result<Option<usize>> {
    if header == &[0; 12] {
        return Ok(None); // Stage-B keeper-only fixture has no SSH bootstrap.
    }
    let len = u32::from_be_bytes(header[8..12].try_into().unwrap()) as usize;
    if &header[..8] != MAGIC || !(1..=MAX_RECORD).contains(&len) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid SSH bootstrap header",
        ));
    }
    Ok(Some(len))
}

pub fn read(path: &str) -> io::Result<Option<Seed>> {
    let disk = File::open(path)?;
    let mut header = [0; 12];
    disk.read_exact_at(&mut header, OFFSET)?;
    let Some(len) = length(&header)? else {
        return Ok(None);
    };
    let mut payload = vec![0; len];
    disk.read_exact_at(&mut payload, OFFSET + 12)?;
    let seed: Seed = serde_json::from_slice(&payload)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid SSH bootstrap record"))?;
    if seed.schema != 1
        || seed.instance_id.len() != 32
        || !seed.instance_id.bytes().all(|b| b.is_ascii_hexdigit())
        || seed.host_key.is_empty()
        || seed.authorized_key.is_empty()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid SSH bootstrap fields",
        ));
    }
    Ok(Some(seed))
} // No raw disk descriptor survives into the keeper/SSH threads or sessions.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_disk_keeps_ssh_disabled() {
        assert_eq!(length(&[0; 12]).unwrap(), None);
    }

    #[test]
    fn unknown_magic_and_invalid_lengths_fail_closed() {
        for len in [0_u32, (MAX_RECORD + 1) as u32, u32::MAX] {
            let mut header = [0; 12];
            header[..8].copy_from_slice(MAGIC);
            header[8..].copy_from_slice(&len.to_be_bytes());
            assert_eq!(
                length(&header).unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
        }
        assert!(length(b"UNKNOWN!\0\0\0\x01").is_err());
    }

    #[test]
    fn a_valid_header_has_a_bounded_exact_length() {
        let mut header = [0; 12];
        header[..8].copy_from_slice(MAGIC);
        header[8..].copy_from_slice(&(MAX_RECORD as u32).to_be_bytes());
        assert_eq!(length(&header).unwrap(), Some(MAX_RECORD));
    }

    #[test]
    fn private_disk_roundtrips_and_truncated_records_are_not_accepted() {
        use std::io::{Seek, SeekFrom, Write};
        let mut disk = tempfile::NamedTempFile::new().unwrap();
        let payload = br#"{"schema":1,"instance_id":"0123456789abcdef0123456789abcdef","host_key":"test-private","authorized_key":"test-public"}"#;
        disk.seek(SeekFrom::Start(OFFSET)).unwrap();
        disk.write_all(MAGIC).unwrap();
        disk.write_all(&(payload.len() as u32).to_be_bytes())
            .unwrap();
        disk.write_all(payload).unwrap();
        let seed = read(disk.path().to_str().unwrap()).unwrap().unwrap();
        assert_eq!(seed.host_key, "test-private");
        assert_eq!(seed.authorized_key, "test-public");
        disk.as_file().set_len(OFFSET + 13).unwrap();
        assert_eq!(
            read(disk.path().to_str().unwrap()).err().unwrap().kind(),
            io::ErrorKind::UnexpectedEof
        );
    }
}
