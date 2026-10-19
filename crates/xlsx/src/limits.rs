//! Decompression limits for zip packages (XLSX, XLSB, ODS): the one place they are chosen, per
//! build target. Workbooks arrive from outside (mail, chat, ERP attachments), so a package that
//! would inflate past these is refused before it can exhaust memory.

/// How far one package may inflate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Largest inflated entry, in bytes.
    pub entry: u64,
    /// Largest inflated total read from one package, in bytes.
    pub total: u64,
    /// Most entries in the zip directory.
    pub entries: usize,
    /// Largest inflated/compressed ratio, checked once an entry passes [`Limits::ratio_floor`].
    pub ratio: u64,
    /// Inflated size below which the ratio is not checked: small parts of a few repeated
    /// elements legitimately compress far beyond any useful ratio.
    pub ratio_floor: u64,
}

const MIB: u64 = 1024 * 1024;

// Measured on generated parts: data sheets deflate 8–25:1, mail-merge-like documents ~55:1 and
// a document of 300 identical table pages 184:1, while a deflate bomb runs at ~1000:1 (deflate's
// ceiling is 1032:1). 200:1 leaves the legitimate cases room; a bomb tuned to stay just under it
// still meets the size caps. Letting an entry reach 4 MiB before the ratio applies costs nothing
// and keeps small repetitive parts clear.
const RATIO: u64 = 200;
const RATIO_FLOOR: u64 = 4 * MIB;

/// Desktop: the limits GridCraft has always had.
pub const NATIVE: Limits = Limits { entry: 512 * MIB, total: 1024 * MIB, entries: 200_000, ratio: RATIO, ratio_floor: RATIO_FLOOR };

/// Browser tab: wasm32 has 4 GiB of address space at most and a tab often gets less, while the
/// file itself, its base64 copy and the parsed workbook all live there too. A 256 MiB part is
/// already some 8 million cells; 512 MiB in all keeps the inflated working set to a quarter of a
/// 2 GiB heap. 50,000 entries is many times what a workbook with a thousand sheets uses.
pub const WEB: Limits = Limits { entry: 256 * MIB, total: 512 * MIB, entries: 50_000, ratio: RATIO, ratio_floor: RATIO_FLOOR };

/// The limits for a target: [`WEB`] for wasm32, [`NATIVE`] otherwise.
pub const fn for_target(wasm32: bool) -> Limits {
    if wasm32 { WEB } else { NATIVE }
}

/// The limits of this build.
pub const LIMITS: Limits = for_target(cfg!(target_arch = "wasm32"));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_web_build_gets_the_smaller_limits() {
        assert_eq!(for_target(true), WEB);
        assert_eq!(for_target(false), NATIVE);
        assert_eq!(LIMITS, for_target(cfg!(target_arch = "wasm32")));
        const { assert!(WEB.entry < NATIVE.entry && WEB.total < NATIVE.total && WEB.entries < NATIVE.entries) };
        for l in [WEB, NATIVE] {
            assert!(l.entry <= l.total && l.ratio_floor < l.entry && l.ratio >= 100);
        }
    }
}
