//! Little-endian field access over a whole `.ez` file, recording which bytes
//! the decoder has accounted for.
//!
//! Every read claims its bytes. Once decoding is done, any non-zero byte that
//! nothing claimed is reported, so a field the decoder doesn't know about can't
//! pass unnoticed.

use std::fmt;

/// How a byte has been accounted for. Later claims only ever raise the level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Claim {
    /// Not read by anything.
    Free,
    /// Part of a table row past that table's count: EZNEC leaves old values there.
    Leftover,
    /// A field whose place is known but whose meaning is not.
    Unknown,
    /// A decoded field.
    Field,
}

/// A field whose position is known but whose meaning is not decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownField {
    /// Absolute file offset.
    pub offset: usize,
    pub label: String,
    pub bytes: Vec<u8>,
}

impl UnknownField {
    pub fn is_zero(&self) -> bool {
        self.bytes.iter().all(|&b| b == 0)
    }
}

/// A run of non-zero bytes that no decoder step claimed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unclaimed {
    pub offset: usize,
    pub bytes: Vec<u8>,
}

/// A decoding failure, with the file offset where it was found when there is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EzError {
    pub offset: Option<usize>,
    pub message: String,
}

impl EzError {
    pub fn new(message: impl Into<String>) -> Self {
        EzError {
            offset: None,
            message: message.into(),
        }
    }

    pub fn at(offset: usize, message: impl Into<String>) -> Self {
        EzError {
            offset: Some(offset),
            message: message.into(),
        }
    }
}

impl fmt::Display for EzError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.offset {
            Some(off) => write!(f, "at {off:#06x}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for EzError {}

pub(crate) struct Bytes<'a> {
    data: &'a [u8],
    claims: Vec<Claim>,
    unknowns: Vec<UnknownField>,
}

impl<'a> Bytes<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Bytes {
            data,
            claims: vec![Claim::Free; data.len()],
            unknowns: Vec::new(),
        }
    }

    fn take(&mut self, off: usize, len: usize, claim: Claim) -> Result<&'a [u8], EzError> {
        let end = off
            .checked_add(len)
            .filter(|&end| end <= self.data.len())
            .ok_or_else(|| {
                EzError::at(
                    off,
                    format!("needs {len} bytes, but the file ends at {:#06x}", self.data.len()),
                )
            })?;
        for c in &mut self.claims[off..end] {
            *c = (*c).max(claim);
        }
        Ok(&self.data[off..end])
    }

    pub fn slice(&mut self, off: usize, len: usize) -> Result<&'a [u8], EzError> {
        self.take(off, len, Claim::Field)
    }

    pub fn u8(&mut self, off: usize) -> Result<u8, EzError> {
        Ok(self.take(off, 1, Claim::Field)?[0])
    }

    pub fn i16(&mut self, off: usize) -> Result<i16, EzError> {
        let b = self.take(off, 2, Claim::Field)?;
        Ok(i16::from_le_bytes([b[0], b[1]]))
    }

    pub fn i32(&mut self, off: usize) -> Result<i32, EzError> {
        let b = self.take(off, 4, Claim::Field)?;
        Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn u32(&mut self, off: usize) -> Result<u32, EzError> {
        let b = self.take(off, 4, Claim::Field)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn f32(&mut self, off: usize) -> Result<f32, EzError> {
        let b = self.take(off, 4, Claim::Field)?;
        Ok(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn f32s<const N: usize>(&mut self, off: usize) -> Result<[f32; N], EzError> {
        let mut out = [0.0; N];
        for (i, v) in out.iter_mut().enumerate() {
            *v = self.f32(off + 4 * i)?;
        }
        Ok(out)
    }

    pub fn f64(&mut self, off: usize) -> Result<f64, EzError> {
        let b = self.take(off, 8, Claim::Field)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(f64::from_le_bytes(a))
    }

    /// Records a field whose meaning isn't decoded. It is listed in the report.
    pub fn unknown(&mut self, off: usize, len: usize, label: impl Into<String>) -> Result<(), EzError> {
        let bytes = self.take(off, len, Claim::Unknown)?.to_vec();
        self.unknowns.push(UnknownField {
            offset: off,
            label: label.into(),
            bytes,
        });
        Ok(())
    }

    /// Marks a table row past the table's count, which holds stale values.
    pub fn leftover(&mut self, off: usize, len: usize) -> Result<(), EzError> {
        self.take(off, len, Claim::Leftover).map(|_| ())
    }

    pub fn into_report(self) -> (Vec<UnknownField>, Vec<Unclaimed>) {
        // Split unclaimed spans at runs of 4 or more zero bytes, and trim zeros.
        const GAP: usize = 4;
        let mut runs = Vec::new();
        let mut start: Option<usize> = None;
        let mut last_nonzero = 0;
        for (i, (&b, &c)) in self.data.iter().zip(&self.claims).enumerate() {
            let hot = c == Claim::Free && b != 0;
            if hot {
                if let Some(s) = start {
                    if i - last_nonzero > GAP {
                        runs.push((s, last_nonzero + 1));
                        start = Some(i);
                    }
                } else {
                    start = Some(i);
                }
                last_nonzero = i;
            } else if c != Claim::Free {
                if let Some(s) = start.take() {
                    runs.push((s, last_nonzero + 1));
                }
            }
        }
        if let Some(s) = start {
            runs.push((s, last_nonzero + 1));
        }
        let unclaimed = runs
            .into_iter()
            .map(|(s, e)| Unclaimed {
                offset: s,
                bytes: self.data[s..e].to_vec(),
            })
            .collect();
        (self.unknowns, unclaimed)
    }
}

/// Decodes a byte string as Windows-1252/Latin-1 text, one char per byte.
pub(crate) fn latin1(b: &[u8]) -> String {
    b.iter().map(|&c| c as char).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unclaimed_runs_are_split_and_trimmed() {
        let data = [0, 1, 2, 0, 0, 0, 0, 0, 3, 0, 9, 9];
        let mut b = Bytes::new(&data);
        b.slice(10, 2).unwrap();
        let (_, un) = b.into_report();
        assert_eq!(
            un,
            vec![
                Unclaimed {
                    offset: 1,
                    bytes: vec![1, 2]
                },
                Unclaimed {
                    offset: 8,
                    bytes: vec![3]
                }
            ]
        );
    }

    #[test]
    fn reading_past_the_end_is_an_error() {
        let data = [0u8; 3];
        let mut b = Bytes::new(&data);
        let e = b.f32(0).unwrap_err();
        assert_eq!(e.offset, Some(0));
    }

    #[test]
    fn field_claim_wins_over_leftover() {
        let data = [7u8; 4];
        let mut b = Bytes::new(&data);
        b.leftover(0, 4).unwrap();
        b.u8(1).unwrap();
        let (_, un) = b.into_report();
        assert!(un.is_empty());
    }
}
