//! BLAKE3 hashes of checkpoint files, computed by the server as their parts arrive, in any
//! order, without reading anything back from storage: each part's hash is the BLAKE3 chaining
//! value of its subtree (parts are 16 MiB, a power of two: a subtree of the file's BLAKE3 tree),
//! and the file's hash merges them as BLAKE3 would. The same as `blake3::hash` (or `b3sum`) of
//! the whole file.

use blake3::hazmat::{
    ChainingValue, HasherExt, Mode, left_subtree_len, merge_subtrees_non_root, merge_subtrees_root,
};

pub type Hash = [u8; 32];

/// The hash a part contributes: for a file of one part, the file's hash itself; otherwise the
/// chaining value of its subtree. `part_len` is a power of two, at least 1 KiB.
pub fn part_hash(content: &[u8], part: u64, parts: u64, part_len: u64) -> Hash {
    if parts <= 1 {
        return *blake3::hash(content).as_bytes();
    }
    blake3::Hasher::new()
        .set_input_offset(part * part_len)
        .update(content)
        .finalize_non_root()
}

/// A file's BLAKE3 hash from its parts' (in order), for a file of `size` bytes.
pub fn file_hash(parts: &[Hash], size: u64, part_len: u64) -> Hash {
    match parts {
        [] => *blake3::hash(b"").as_bytes(),
        [only] => *only,
        _ => subtree(parts, size, part_len, true),
    }
}

/// BLAKE3's tree is left-full: the left subtree holds the largest power of two of chunks below
/// the length (here, whole parts), the right one the rest.
fn subtree(parts: &[ChainingValue], len: u64, part_len: u64, root: bool) -> Hash {
    if let [only] = parts {
        return *only;
    }
    let left_len = left_subtree_len(len);
    let split = (left_len / part_len) as usize;
    let left = subtree(&parts[..split], left_len, part_len, false);
    let right = subtree(&parts[split..], len - left_len, part_len, false);
    if root {
        *merge_subtrees_root(&left, &right, Mode::Hash).as_bytes()
    } else {
        merge_subtrees_non_root(&left, &right, Mode::Hash)
    }
}

/// A checkpoint's digest: BLAKE3 of its files listed as `b3sum` lists them, `<hash>  <path>`, a
/// line each, by path. Changes with any file's content, path or presence.
pub fn digest<'a>(files: impl IntoIterator<Item = (&'a str, &'a [u8])>) -> Hash {
    let mut lines: Vec<String> = files
        .into_iter()
        .map(|(path, hash)| format!("{}  {path}\n", hex(hash)))
        .collect();
    lines.sort_by(|a, b| a[66..].cmp(&b[66..]));
    *blake3::hash(lines.concat().as_bytes()).as_bytes()
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hashes `data` in parts of `part_len`, out of order, as the server receives them.
    fn by_parts(data: &[u8], part_len: u64) -> Hash {
        let chunks: Vec<&[u8]> = data.chunks(part_len as usize).collect();
        let parts = chunks.len() as u64;
        let mut hashes: Vec<(usize, Hash)> = chunks
            .iter()
            .enumerate()
            .rev()
            .map(|(i, c)| (i, part_hash(c, i as u64, parts, part_len)))
            .collect();
        hashes.sort_by_key(|(i, _)| *i);
        let in_order: Vec<Hash> = hashes.into_iter().map(|(_, h)| h).collect();
        file_hash(&in_order, data.len() as u64, part_len)
    }

    #[test]
    fn the_same_as_hashing_the_whole_file() {
        let data: Vec<u8> = (0..100_000u32).map(|i| (i * 7 % 251) as u8).collect();
        for part_len in [1024u64, 4096] {
            let mut sizes = vec![0, 1, 1023, 1024, 1025];
            for k in [1u64, 2, 3, 4, 5, 7, 8, 9, 16, 17, 24] {
                sizes.extend([k * part_len - 1, k * part_len, k * part_len + 1]);
            }
            for size in sizes {
                let size = size as usize;
                let want = blake3::hash(&data[..size]);
                assert_eq!(
                    by_parts(&data[..size], part_len),
                    *want.as_bytes(),
                    "{size} bytes in parts of {part_len}"
                );
            }
        }
    }

    #[test]
    fn real_parts() {
        // Three parts of 16 MiB, the last one short.
        let part_len = losscape_core::CHECKPOINT_PART_SIZE;
        let data: Vec<u8> = (0..(2 * part_len + 12_345))
            .map(|i| (i % 253) as u8)
            .collect();
        assert_eq!(by_parts(&data, part_len), *blake3::hash(&data).as_bytes());
    }

    #[test]
    fn digests_list_files_by_path() {
        let a = [1u8; 32];
        let b = [2u8; 32];
        let digest_ab = digest([("a", &a[..]), ("b/c", &b[..])]);
        assert_eq!(
            digest_ab,
            digest([("b/c", &b[..]), ("a", &a[..])]),
            "any order"
        );
        let listing = format!("{}  a\n{}  b/c\n", hex(&a), hex(&b));
        assert_eq!(digest_ab, *blake3::hash(listing.as_bytes()).as_bytes());
        assert_ne!(digest_ab, digest([("a", &b[..]), ("b/c", &a[..])]));
    }
}
