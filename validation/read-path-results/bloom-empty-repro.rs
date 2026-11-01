use kiban::bloom::BloomFilter;

fn main() {
    let empty = BloomFilter::empty();
    let encoded = empty.encode();
    let built = BloomFilter::build(std::iter::empty::<&[u8]>());
    println!("empty(): {} encoded bytes; decode succeeds: {}", encoded.len(),
        BloomFilter::decode(&encoded).is_some());
    println!("build(empty): {} encoded bytes; decode succeeds: {}", built.encode().len(),
        BloomFilter::decode(&built.encode()).is_some());
    assert_eq!(BloomFilter::decode(&encoded), Some(empty),
        "a valid empty filter must round-trip through its public codec");
}
