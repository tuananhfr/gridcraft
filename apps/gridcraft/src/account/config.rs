//! Where the desktop app finds PI and which keys verify its leases.

use erpcons_account::{Config, PinnedKey, Product};
use gridcraft_ui_egui::i18n::Language;

/// Read at build time only: a release names its issuer, and nothing at run time can point the app
/// at another one.
pub(super) const ISSUER: Option<&str> = option_env!("ERPCONS_PI_ISSUER");
/// PI's end-user API base; unset, the library uses `{issuer}/api/v1/id`.
const API_BASE: Option<&str> = option_env!("ERPCONS_PI_API_BASE");
/// `kid:hex32` of pi-mock's lease key, for debug builds run against it; releases never read it.
#[cfg(debug_assertions)]
const DEV_LEASE_KEY: Option<&str> = option_env!("ERPCONS_PI_DEV_LEASE_KEY");

// A release that cannot verify a lease would lock every user out of editing.
#[cfg(not(debug_assertions))]
const _: () = assert!(!erpcons_account::PLATFORM_KEYS.is_empty());

/// The account configuration of this build, `None` when it names no PI issuer.
pub(super) fn from_build(language: Language) -> Option<Config> {
    let issuer = ISSUER.filter(|issuer| !issuer.is_empty())?;
    let mut config = config_for(issuer, language);
    config.api_base = API_BASE.filter(|base| !base.is_empty()).map(str::to_owned);
    #[cfg(debug_assertions)]
    if let Some(value) = DEV_LEASE_KEY.filter(|v| !v.is_empty()) {
        match parse_dev_lease_key(value) {
            Some(key) => config.platform_keys.push(key),
            None => log::warn!("ERPCONS_PI_DEV_LEASE_KEY is not kid:<64 hex digits>; ignored"),
        }
    }
    Some(config)
}

pub(super) fn config_for(issuer: &str, language: Language) -> Config {
    let mut config = Config::new(issuer, Product::BangTinh, env!("CARGO_PKG_VERSION"));
    // Editing needs the shared `erpconsoffice` lease from PI's device protocol (ADR 0013).
    config.offline_licensing = true;
    config.ui_locale = Some(if language == Language::Vi { "vi" } else { "en" }.into());
    config.callback_success_html = callback_page(language, "Signed in to PI ID. You can close this tab and go back to the app.");
    config.callback_failure_html = callback_page(language, "Sign-in did not finish. Close this tab and try again in the app.");
    config
}

fn callback_page(language: Language, key: &str) -> String {
    let text = language.tr(key).replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    format!("<!doctype html><meta charset=\"utf-8\"><title>PI ID</title><p style=\"font:16px sans-serif;margin:3em\">{text}</p>")
}

/// `kid:hex32`: a lease-signing key id and its Ed25519 public key in 64 hex digits.
#[cfg_attr(not(debug_assertions), allow(dead_code))]
pub(super) fn parse_dev_lease_key(value: &str) -> Option<PinnedKey> {
    let (kid, hex) = value.split_once(':')?;
    // `from_str_radix` alone would accept a leading `+`.
    if kid.is_empty() || hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut key = [0u8; 32];
    for (byte, pair) in key.iter_mut().zip(hex.as_bytes().chunks(2)) {
        *byte = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(PinnedKey { kid: kid.to_owned(), key })
}

#[cfg(test)]
mod tests {
    use erpcons_account::{OFFICE_PI_PRODUCT, PinnedKey, Product};

    use super::*;

    #[test]
    fn licensing_is_on_for_the_office_product() {
        let config = config_for("http://127.0.0.1:9", Language::Vi);
        assert!(config.offline_licensing);
        assert_eq!(config.product, Product::BangTinh);
        assert_eq!(config.profile.pi_product, OFFICE_PI_PRODUCT);
        assert_eq!(config.client_id, "erpcons-office-desktop");
        assert!(!config.platform_keys.is_empty(), "release keys stay pinned");
    }

    #[test]
    fn dev_lease_keys_parse_only_as_kid_and_64_hex_digits() {
        let hex: String = pi_mock::dev_lease_public_key().iter().map(|b| format!("{b:02x}")).collect();
        let key = parse_dev_lease_key(&format!("{}:{hex}", pi_mock::DEV_LEASE_KID));
        assert_eq!(key, Some(PinnedKey { kid: pi_mock::DEV_LEASE_KID.into(), key: pi_mock::dev_lease_public_key() }));
        for bad in ["", "kid", ":00", "kid:00", &format!("kid:{hex}0"), &format!("kid:{}", "zz".repeat(32)), &format!("kid:+{}", &hex[1..])] {
            assert_eq!(parse_dev_lease_key(bad), None, "{bad:?}");
        }
    }
}
