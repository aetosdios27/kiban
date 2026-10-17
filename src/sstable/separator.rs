//! Parsed separators that keep common short keys in the index cache line.
pub(super) enum Separator {
    Inline { len: u8, bytes: [u8; 46] },
    Long(Vec<u8>),
}

impl Separator {
    pub(super) fn new(bytes: &[u8]) -> Self {
        if bytes.len() > 46 {
            return Self::Long(bytes.to_vec());
        }
        let mut inline = [0; 46];
        inline[..bytes.len()].copy_from_slice(bytes);
        Self::Inline {
            len: bytes.len() as u8,
            bytes: inline,
        }
    }

    pub(super) fn as_slice(&self) -> &[u8] {
        match self {
            Self::Inline { len, bytes } => &bytes[..usize::from(*len)],
            Self::Long(bytes) => bytes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn separators_preserve_short_long_and_binary_keys() {
        for len in [0, 1, 37, 38, 45, 46, 47, 100, 4096] {
            let key: Vec<_> = (0..len).map(|i| (i % 256) as u8).collect();
            assert_eq!(Separator::new(&key).as_slice(), key);
        }
    }
}
