//! The vault: every shipped data file, deflated at build
//! time by build.rs into one `include_bytes!` archive. Inflate on first use,
//! cached for the life of the process. Callers never see a path; the vault is
//! the only data source in the binary.
use std::collections::HashMap;
use std::io::Read;
use std::sync::OnceLock;

/// One `pub const` per manifest entry — a mistyped name is a compile error.
#[allow(dead_code)] // every entry gets a const; not every const has a caller yet
pub mod names {
    include!(concat!(env!("OUT_DIR"), "/vault_names.rs"));
}

static VAULT_BIN: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/vault.bin"));

struct Entry {
    offset: usize,
    raw_len: usize,
    packed_len: usize,
    crc: u32,
    inflated: OnceLock<Box<[u8]>>,
}

struct Index {
    entries: HashMap<&'static str, Entry>,
    blobs: &'static [u8],
}

fn index() -> &'static Index {
    static INDEX: OnceLock<Index> = OnceLock::new();
    INDEX.get_or_init(|| {
        let b = VAULT_BIN;
        assert_eq!(&b[..4], b"OMAV", "vault.bin: bad magic");
        let count = u32::from_le_bytes(b[4..8].try_into().unwrap()) as usize;
        let mut pos = 8;
        let mut entries = HashMap::with_capacity(count);
        for _ in 0..count {
            let name_len = u16::from_le_bytes(b[pos..pos + 2].try_into().unwrap()) as usize;
            pos += 2;
            let name = std::str::from_utf8(&b[pos..pos + name_len]).expect("vault name utf8");
            pos += name_len;
            let offset = u64::from_le_bytes(b[pos..pos + 8].try_into().unwrap()) as usize;
            let raw_len = u32::from_le_bytes(b[pos + 8..pos + 12].try_into().unwrap()) as usize;
            let packed_len = u32::from_le_bytes(b[pos + 12..pos + 16].try_into().unwrap()) as usize;
            let crc = u32::from_le_bytes(b[pos + 16..pos + 20].try_into().unwrap());
            pos += 20;
            entries.insert(name, Entry { offset, raw_len, packed_len, crc, inflated: OnceLock::new() });
        }
        Index { entries, blobs: &b[pos..] }
    })
}

/// The inflated bytes of a vault entry. Panics on an unknown name (names come
/// from [`names`], so that is a programming error, not a runtime condition).
pub fn bytes(name: &str) -> &'static [u8] {
    let idx = index();
    let e = idx
        .entries
        .get(name)
        .unwrap_or_else(|| panic!("vault: no entry named {name:?} (see data/manifest.toml)"));
    e.inflated.get_or_init(|| {
        let packed = &idx.blobs[e.offset..e.offset + e.packed_len];
        let mut out = Vec::with_capacity(e.raw_len);
        flate2::read::DeflateDecoder::new(packed)
            .read_to_end(&mut out)
            .unwrap_or_else(|err| panic!("vault: inflate {name}: {err}"));
        assert_eq!(out.len(), e.raw_len, "vault: {name}: length mismatch");
        assert_eq!(crc32fast::hash(&out), e.crc, "vault: {name}: crc mismatch");
        out.into_boxed_slice()
    })
}

/// A vault entry parsed as JSON.
pub fn json<T: serde::de::DeserializeOwned>(name: &str) -> T {
    serde_json::from_slice(bytes(name)).unwrap_or_else(|e| panic!("vault: {name}: not valid JSON for the requested type: {e}"))
}

/// Every entry name, sorted.
pub fn names_list() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = index().entries.keys().copied().collect();
    v.sort_unstable();
    v
}
