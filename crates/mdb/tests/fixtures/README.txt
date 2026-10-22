Jackcess regression fixtures (Apache-2.0)

Upstream: https://github.com/jahlborn/jackcess
Pinned revision: be0a2139b087a227d14cafd4ea231306a5a3a862
Author: Jackcess contributors / Health Market Science, Inc.
These are upstream test databases, not customer files. No vendor artwork is included.
LICENSE.txt and NOTICE.txt are supplied from that revision.

jet3.mdb: src/test/data/V1997/testV1997.mdb (118784 bytes)
SHA-256: 27788b47e17412427830e80c2cb69366885aa11b12c61422de73029194bef135
jet4.mdb: src/test/data/V2000/overflowTestV2000.mdb (143360 bytes)
SHA-256: 3e56f34bf57ab765f633cf865baf312130ddd4a723a22422b94dc05538e9570e

The ETABS-shaped fixture is original Rust code in ../support/mod.rs. It uses
synthetic values only, with no bytes from the customer workbook or exports.

Implementation references (format facts, no GPL/LGPL code):
https://github.com/mdbtools/mdbtools/blob/dev/HACKING.md
https://jackcess.sourceforge.io/

jetdb 0.3.3 (MIT OR Apache-2.0) was evaluated but its PageReader requires a
std::fs::File; it has no bytes/Read+Seek constructor. mdbtools-pure-rs is a
translation of LGPL mdbtools and was not used. This crate implements a bounded
Jet 3/4 reader, with no SQL, filesystem, VBA, COM, ACE or encryption support.
Limits: 128 MiB input, 1024 user tables, 256 columns per table, 1048575 rows per
table, 64 MiB decoded data per database (work capped per table), 4 MiB strings/TDEFs, 64 overflow-row hops.
User-table discovery uses MSysObjects; row scanning follows owned-page maps,
not indexes. Damaged rows return errors rather than silently losing records.
Jet 3 text uses the declared Windows code page; Jet 4 supports UTF-16 and compressed Unicode.
