//! MS-OVBA §2.4.1: chunk-local copy tokens with overlapping copies.
use crate::{Error, MAX_STREAM};

pub(crate) fn decompress(input: &[u8], budget: &mut usize) -> Result<Vec<u8>, Error> {
    if input.first() != Some(&1) {
        return Err(Error::Invalid("compressed container signature"));
    }
    let mut output = Vec::new();
    let mut pos = 1usize;
    while pos < input.len() {
        let header = input.get(pos..pos + 2).ok_or(Error::Invalid("chunk header"))?;
        let header = u16::from_le_bytes([header.first().copied().unwrap_or(0), header.get(1).copied().unwrap_or(0)]);
        let size = usize::from(header & 0x0fff) + 3;
        if header & 0x7000 != 0x3000 {
            return Err(Error::Invalid("chunk signature"));
        }
        let chunk = input.get(pos + 2..pos + size).ok_or(Error::Invalid("truncated chunk"))?;
        let start = output.len();
        if start.saturating_add(4096) > MAX_STREAM || *budget < 4096.min(chunk.len()) {
            return Err(Error::Limit("decompressed bytes"));
        }
        if header & 0x8000 == 0 {
            if size != 4098 {
                return Err(Error::Invalid("raw chunk size"));
            }
            output.extend_from_slice(chunk);
        } else {
            let mut at = 0;
            while let Some(&flags) = chunk.get(at) {
                // A trailing flag byte without tokens is valid and Office writes it.
                at += 1;
                for bit in 0..8 {
                    if at == chunk.len() {
                        break;
                    }
                    if flags & (1 << bit) == 0 {
                        output.push(*chunk.get(at).ok_or(Error::Invalid("literal"))?);
                        at += 1;
                    } else {
                        let low = *chunk.get(at).ok_or(Error::Invalid("copy token"))?;
                        let high = *chunk.get(at + 1).ok_or(Error::Invalid("copy token"))?;
                        at += 2;
                        let token = u16::from_le_bytes([low, high]);
                        let position = output.len() - start;
                        let bits = (usize::BITS - position.saturating_sub(1).leading_zeros()).max(4);
                        let mask = u16::MAX >> bits;
                        let length = usize::from(token & mask) + 3;
                        let offset = usize::from(token >> (16 - bits)) + 1;
                        if offset > position || position + length > 4096 {
                            return Err(Error::Invalid("copy token outside chunk"));
                        }
                        for _ in 0..length {
                            let byte = *output.get(output.len() - offset).ok_or(Error::Invalid("copy offset"))?;
                            output.push(byte);
                        }
                    }
                    if output.len() - start > 4096 {
                        return Err(Error::Invalid("decompressed chunk size"));
                    }
                }
            }
        }
        let added = output.len() - start;
        *budget = budget.checked_sub(added).ok_or(Error::Limit("total decompressed bytes"))?;
        pos += size;
        if pos < input.len() && added != 4096 {
            return Err(Error::Invalid("short non-final chunk"));
        }
    }
    Ok(output)
}
