//! Bounded format-aware mutation corpus, runnable without a fuzzing toolchain.
#[test]
fn parser_mutation_corpus_returns_errors_without_panicking() {
    use crate::{
        frame::FrameReader,
        manifest::Manifest,
        sstable::VerifiedBlock,
        sstable::{Kind, TableBuilder},
    };
    let manifest = Manifest::fresh().encode();
    let mut builder = TableBuilder::new();
    builder.add(Kind::Put, b"key", b"value", 1).unwrap();
    let table = builder.finish().unwrap();
    let footer = table.len() - crate::sstable::builder::FOOTER_LEN;
    let data_len = u64::from_le_bytes(table[footer + 20..footer + 28].try_into().unwrap()) as usize;
    let block = &table[..data_len];
    // A shortened first entry followed by a valid kind byte but an
    // incomplete second header must fail rather than index past the buffer.
    let mut short_header = block.to_vec();
    short_header[9..13].copy_from_slice(&0u32.to_le_bytes());
    short_header[24] = 1;
    let n = short_header.len();
    let crc = crate::crc32::crc32(&short_header[..n - 4]);
    short_header[n - 4..].copy_from_slice(&crc.to_le_bytes());
    let block_iter = VerifiedBlock::from_raw(short_header).unwrap();
    assert!(block_iter.iter().collect::<Result<Vec<_>, _>>().is_err());
    for seed in [&manifest[..], block] {
        for cut in 0..=seed.len() {
            let _ = Manifest::decode(&seed[..cut]);
            let _ = VerifiedBlock::from_raw(seed[..cut].to_vec());
        }
        for i in 0..seed.len() {
            for bit in 0..8 {
                let mut mutated = seed.to_vec();
                mutated[i] ^= 1 << bit;
                let _ = Manifest::decode(&mutated);
                let _ = VerifiedBlock::from_raw(mutated.clone());
                // Exercise block semantics beyond checksum rejection.
                if mutated.len() >= 5 {
                    let n = mutated.len();
                    let crc = crate::crc32::crc32(&mutated[..n - 4]);
                    mutated[n - 4..].copy_from_slice(&crc.to_le_bytes());
                    if let Ok(block) = VerifiedBlock::from_raw(mutated) {
                        for key in [b"".as_slice(), b"key", b"z"] {
                            let _ = block.get(key, None);
                        }
                        let _: Vec<_> = block.iter().collect();
                    }
                }
            }
        }
    }
    for len in [0u32, 1, crate::frame::MAX_RECORD_LEN, u32::MAX] {
        let mut header = len.to_le_bytes().to_vec();
        header.extend_from_slice(&0u32.to_le_bytes());
        let _ = FrameReader::new(header.as_slice()).read_record();
    }
}
