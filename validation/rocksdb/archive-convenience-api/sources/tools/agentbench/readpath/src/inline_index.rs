//! Cache-line-sized separator prototype; existing file encoding is unchanged.
pub(crate) enum Separator {
    Inline { len: u8, bytes: [u8; 38] },
    Long(Vec<u8>),
}
impl Separator {
    pub(crate) fn new(bytes: &[u8]) -> Self {
        if bytes.len() > 38 {
            return Self::Long(bytes.to_vec());
        }
        let mut inline = [0; 38];
        inline[..bytes.len()].copy_from_slice(bytes);
        Self::Inline {
            len: bytes.len() as u8,
            bytes: inline,
        }
    }
    pub(crate) fn as_slice(&self) -> &[u8] {
        match self {
            Self::Inline { len, bytes } => &bytes[..usize::from(*len)],
            Self::Long(bytes) => bytes,
        }
    }
}
#[repr(align(64))]
pub(crate) struct InlineEntry {
    separator: Separator,
    pub(crate) offset: u64,
    pub(crate) len: u64,
}
impl InlineEntry {
    pub(crate) fn new(key: &[u8], offset: u64, len: u64) -> Self {
        Self {
            separator: Separator::new(key),
            offset,
            len,
        }
    }
}
impl AsRef<[u8]> for InlineEntry {
    fn as_ref(&self) -> &[u8] {
        self.separator.as_slice()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inline_separators_preserve_arbitrary_binary_keys_and_long_keys() {
        assert_eq!(std::mem::size_of::<InlineEntry>(), 64);
        for len in [0, 1, 37, 38, 39, 100, 4096] {
            let key: Vec<_> = (0..len).map(|i| (i % 256) as u8).collect();
            let e = InlineEntry::new(&key, 123, 456);
            assert_eq!(e.as_ref(), key);
            assert_eq!((e.offset, e.len), (123, 456));
        }
    }
}
