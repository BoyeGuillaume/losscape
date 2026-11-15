//! Headers of a tar archive (POSIX pax), for streaming a checkpoint's files as one download.
//! Every file gets a pax header with its full path and size, so paths of any length and files
//! over 8 GiB come out whole; the ustar header after it holds what fits, for old readers.

const BLOCK: usize = 512;

/// Bytes before a file's content: its pax header, then its ustar header.
pub fn file_header(index: usize, path: &str, size: u64, mtime_s: u64) -> Vec<u8> {
    let records = [
        pax_record("path", path),
        pax_record("size", &size.to_string()),
    ]
    .concat();
    let mut out = header(
        &format!("PaxHeaders/{index}"),
        records.len() as u64,
        mtime_s,
        b'x',
    );
    out.extend_from_slice(&records);
    out.resize(out.len() + padding(records.len() as u64), 0);
    out.extend(header(path, size, mtime_s, b'0'));
    out
}

/// Zeros after a file's content, to the end of its last block.
pub fn padding(size: u64) -> usize {
    (BLOCK - (size % BLOCK as u64) as usize) % BLOCK
}

/// The end of the archive: two empty blocks.
pub const END: [u8; 2 * BLOCK] = [0; 2 * BLOCK];

/// The archive's length, from its files' paths and sizes.
pub fn archive_len<'a>(files: impl IntoIterator<Item = (&'a str, u64)>) -> u64 {
    files
        .into_iter()
        .enumerate()
        .map(|(i, (path, size))| {
            file_header(i, path, size, 0).len() as u64 + size + padding(size) as u64
        })
        .sum::<u64>()
        + END.len() as u64
}

/// `"<length> <key>=<value>\n"`, the length counting itself.
fn pax_record(key: &str, value: &str) -> Vec<u8> {
    let rest = key.len() + value.len() + 3; // space, '=', newline
    let mut len = rest + 1;
    while len.to_string().len() + rest != len {
        len = len.to_string().len() + rest;
    }
    format!("{len} {key}={value}\n").into_bytes()
}

fn header(path: &str, size: u64, mtime_s: u64, kind: u8) -> Vec<u8> {
    let mut h = vec![0u8; BLOCK];
    // What fits of the name: readers that know pax take the full path from it.
    let name = path.as_bytes();
    let cut = name.len().min(100);
    let cut = (0..=cut)
        .rev()
        .find(|&i| path.is_char_boundary(i))
        .unwrap_or(0);
    h[..cut].copy_from_slice(&name[..cut]);
    octal(&mut h[100..108], 0o644);
    octal(&mut h[108..116], 0);
    octal(&mut h[116..124], 0);
    // Too large for 11 octal digits: the pax size holds it.
    octal(&mut h[124..136], if size < 8u64.pow(11) { size } else { 0 });
    octal(&mut h[136..148], mtime_s);
    h[156] = kind;
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    h[148..156].fill(b' ');
    let sum: u64 = h.iter().map(|&b| u64::from(b)).sum();
    h[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
    h
}

/// Octal digits filling the field but its last byte (a NUL).
fn octal(field: &mut [u8], value: u64) {
    let digits = field.len() - 1;
    field[..digits].copy_from_slice(format!("{value:0digits$o}").as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn archive(files: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let mut out = Vec::new();
        for (i, (path, data)) in files.iter().enumerate() {
            out.extend(file_header(i, path, data.len() as u64, 1_700_000_000));
            out.extend(data);
            out.resize(out.len() + padding(data.len() as u64), 0);
        }
        out.extend(END);
        out
    }

    #[test]
    fn standard_readers_get_the_files_back() {
        let long = format!("{}/weights.safetensors", "nested/".repeat(40));
        let files = [
            ("model.pt", b"weights".repeat(100)),
            ("tok/vocab.json", b"{}".to_vec()),
            ("empty", Vec::new()),
            (long.as_str(), vec![7; BLOCK]),
            ("données/é.bin", vec![1; 3]),
        ];
        let bytes = archive(&files);
        let paths_sizes = files.iter().map(|(p, d)| (*p, d.len() as u64));
        assert_eq!(archive_len(paths_sizes), bytes.len() as u64);

        let mut read = ::tar::Archive::new(bytes.as_slice());
        let entries: Vec<(String, Vec<u8>, u64)> = read
            .entries()
            .unwrap()
            .map(|e| {
                let mut e = e.unwrap();
                let path = e.path().unwrap().to_string_lossy().into_owned();
                let mtime = e.header().mtime().unwrap();
                let mut data = Vec::new();
                std::io::Read::read_to_end(&mut e, &mut data).unwrap();
                (path, data, mtime)
            })
            .collect();
        assert_eq!(entries.len(), files.len());
        for ((path, data, mtime), (want_path, want_data)) in entries.iter().zip(&files) {
            assert_eq!((path.as_str(), data), (*want_path, want_data));
            assert_eq!(*mtime, 1_700_000_000);
        }
    }

    #[test]
    fn sizes_over_8_gib_are_in_the_pax_header() {
        let size = 9 << 30;
        let header = file_header(0, "big.bin", size, 0);
        assert!(String::from_utf8_lossy(&header).contains(&format!("size={size}\n")));
        assert_eq!(header.len() % BLOCK, 0);
    }

    #[test]
    fn pax_records_count_their_own_length() {
        for value in ["", "a", &"x".repeat(85), &"y".repeat(994)] {
            let record = pax_record("path", value);
            let (len, _) = std::str::from_utf8(&record)
                .unwrap()
                .split_once(' ')
                .unwrap();
            assert_eq!(len.parse::<usize>().unwrap(), record.len(), "{value:?}");
        }
    }
}
