# ERPCons changes

This branch (`erpcons`) is the ERPCons Văn phòng fork of [GridCraft](https://github.com/storytold/gridcraft), based on GridCraft by the ArtCraft team, licensed MIT OR Apache-2.0 (see `LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE`). Changes from upstream `main` (Apache-2.0 §4b):

- Removed the ArtCraft logos and marks (`docs/brand/`) and the links to the ArtCraft Discord and getartcraft.com from the About dialog and README, as the ArtCraft brand terms require for forks.
- About now reads "ERPCons Văn phòng — dựa trên GridCraft của nhóm ArtCraft. MIT OR Apache-2.0." and links this fork's source.
- Builds with `CRAFT_FONTS_DIR` (all ERPCons releases, web and native) embed Inter Regular/Bold (OFL-1.1) from [craft-fonts](https://github.com/storytold/craft-fonts) at `43913056` (`crates/ui-egui/build.rs`); it is appended after the primary face of every font family (after egui's monospace font for MONO), so Vietnamese renders where no system font can be loaded. `craft-fonts-ref` in `release.yml` and `.cargo/config.toml` point at that checkout.
