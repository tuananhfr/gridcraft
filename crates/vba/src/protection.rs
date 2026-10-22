//! MS-OVBA §2.4.3 data encryption of the PROJECT stream `CMG=` and `GC=` values. Only the protection and visibility
//! flags are read; `DPB=` (the password hash) is never decoded.

const MAX_HEX: usize = 2048;

#[derive(Default)]
pub(crate) struct Protection {
    state: Option<Option<Vec<u8>>>,
    visibility: Option<Option<Vec<u8>>>,
}

impl Protection {
    pub(crate) fn read(&mut self, key: &str, value: &str) {
        match key {
            "cmg" => self.state = Some(decrypt(value)),
            "gc" => self.visibility = Some(decrypt(value)),
            _ => {}
        }
    }

    /// `None` when a protection value is present but cannot be decoded.
    pub(crate) fn locked(&self) -> Option<bool> {
        // ProjectProtectionState bit 0 is fUserProtected.
        let user_protected = match &self.state {
            Some(Some(state)) if state.len() == 4 => state.first().is_some_and(|b| b & 1 != 0),
            Some(_) => return None,
            None => false,
        };
        // ProjectVisibilityState 0x00 means the project is locked for viewing.
        let hidden = match &self.visibility {
            Some(Some(visibility)) => match visibility.first() {
                Some(0x00) => true,
                Some(0xff) => false,
                _ => return None,
            },
            Some(None) => return None,
            None => false,
        };
        Some(user_protected || hidden)
    }
}

fn decrypt(value: &str) -> Option<Vec<u8>> {
    let hex = value.trim().strip_prefix('"')?.strip_suffix('"')?;
    if hex.len() > MAX_HEX || !hex.len().is_multiple_of(2) {
        return None;
    }
    let bytes = (0..hex.len()).step_by(2).map(|i| hex.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok())).collect::<Option<Vec<_>>>()?;
    let (&seed, rest) = bytes.split_first()?;
    let (&version_enc, rest) = rest.split_first()?;
    let (&key_enc, rest) = rest.split_first()?;
    if seed ^ version_enc != 2 {
        return None;
    }
    let (mut plain, mut enc1, mut enc2) = (seed ^ key_enc, key_enc, version_enc);
    let mut next = |byte_enc: u8| {
        let byte = byte_enc ^ enc2.wrapping_add(plain);
        enc2 = enc1;
        enc1 = byte_enc;
        plain = byte;
        byte
    };
    let (ignored, rest) = rest.split_at_checked(usize::from((seed & 6) / 2))?;
    ignored.iter().for_each(|&b| {
        next(b);
    });
    let (length, data) = rest.split_at_checked(4)?;
    let length: Vec<u8> = length.iter().map(|&b| next(b)).collect();
    let length = u32::from_le_bytes(length.try_into().ok()?) as usize;
    if length != data.len() {
        return None;
    }
    Some(data.iter().map(|&b| next(b)).collect())
}
