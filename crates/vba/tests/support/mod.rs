pub use gridcraft_vba::testing::{FixtureModule, compress, encrypt, record, unicode};

pub fn fixture(modules: &[FixtureModule<'_>], page: u16, protection: &str, references: &[u8]) -> Vec<u8> {
    gridcraft_vba::testing::fixture(modules, page, protection, references).unwrap()
}
