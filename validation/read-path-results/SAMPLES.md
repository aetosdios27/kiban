# Sample evidence

Self percentages come from perf report. Empty DWARF chains are restored from their sampled instruction pointer and labeled unwound_callers_unavailable. Every decoded sample contributes its complete period to the SVG; missing callers are not invented.

## bulk-512-w2-kiban-stacks

[SVG](profile-complete-stacks/bulk-512-w2-kiban-stacks.svg), [text report](profile-complete-stacks/bulk-512-w2-kiban-stacks.report), [sample coverage](profile-complete-stacks/bulk-512-w2-kiban-stacks.coverage.json)

| Self % | Symbol |
| ---: | --- |
| 28.92 | `__memcmp_avx2_movbe` |
| 26.05 | `kiban::db::get_from_version_at::{closure#0}` |
| 14.35 | `<kiban::cache::BlockCache>::get` |
| 10.48 | `kiban::sstable::block::header_at` |
| 2.39 | `__memmove_avx_unaligned_erms` |
| 2.36 | `<serde_json::read::SliceRead>::skip_to_escape` |
| 1.57 | `<kiban::sstable::block::VerifiedBlock>::get` |
| 1.39 | `core::str::converts::from_utf8` |
| 1.17 | `serde_json::de::from_trait::<serde_json::read::SliceRead, kiban_readbench::model::Memory>` |
| 1.11 | `<serde_json::read::SliceRead as serde_json::read::Read>::parse_str` |
| 0.77 | `<serde_json::de::Deserializer<serde_json::read::SliceRead>>::parse_integer` |
| 0.76 | `<kiban::db::Kiban>::get::<alloc::string::String>` |
## bulk-512-w2-lmdb-stacks

[SVG](profile-complete-stacks/bulk-512-w2-lmdb-stacks.svg), [text report](profile-complete-stacks/bulk-512-w2-lmdb-stacks.report), [sample coverage](profile-complete-stacks/bulk-512-w2-lmdb-stacks.coverage.json)

| Self % | Symbol |
| ---: | --- |
| 27.81 | `mdb_page_search_root` |
| 26.61 | `mdb_node_search` |
| 10.60 | `__memcmp_avx2_movbe` |
| 4.75 | `<serde_json::read::SliceRead>::skip_to_escape` |
| 3.96 | `__memset_avx2_unaligned_erms` |
| 2.65 | `core::str::converts::from_utf8` |
| 2.50 | `mdb_cmp_memn` |
| 2.10 | `serde_json::de::from_trait::<serde_json::read::SliceRead, kiban_readbench::model::Memory>` |
| 1.61 | `<serde_json::read::SliceRead as serde_json::read::Read>::parse_str` |
| 1.22 | `<serde_json::de::Deserializer<serde_json::read::SliceRead>>::parse_integer` |
| 1.18 | `<serde_json::de::MapAccess<_> as serde_core::de::MapAccess>::next_key_seed::has_next_key::<serde_json::read::SliceRead>` |
| 1.18 | `<u64>::_fmt_inner` |
## bulk-512-w3-kiban-stacks

[SVG](profile-complete-stacks/bulk-512-w3-kiban-stacks.svg), [text report](profile-complete-stacks/bulk-512-w3-kiban-stacks.report), [sample coverage](profile-complete-stacks/bulk-512-w3-kiban-stacks.coverage.json)

| Self % | Symbol |
| ---: | --- |
| 13.89 | `<serde_json::read::SliceRead>::skip_to_escape` |
| 8.68 | `core::str::converts::from_utf8` |
| 8.53 | `__memcmp_avx2_movbe` |
| 7.10 | `serde_json::de::from_trait::<serde_json::read::SliceRead, kiban_readbench::model::Memory>` |
| 6.41 | `<serde_json::read::SliceRead as serde_json::read::Read>::parse_str` |
| 5.69 | `__memmove_avx_unaligned_erms` |
| 4.26 | `<serde_json::de::MapAccess<_> as serde_core::de::MapAccess>::next_key_seed::has_next_key::<serde_json::read::SliceRead>` |
| 3.88 | `mi_heap_malloc_aligned_at` |
| 3.48 | `<core::iter::adapters::GenericShunt<core::iter::adapters::map::Map<core::iter::adapters::take::Take<core::iter::adapters::take_while::TakeWhile<kiban::db::DbIter, <kiban::db::Kiban>::range::{closure#0}>>, <kiban_readbench::embedded::Embedded as kiban_readbench::api::Backend>::recent::{closure#0}>, core::result::Result<core::convert::Infallible, alloc::boxed::Box<dyn core::error::Error>>> as core::iter::traits::iterator::Iterator>::next` |
| 3.40 | `<serde_json::de::Deserializer<serde_json::read::SliceRead>>::parse_integer` |
| 3.30 | `mi_free` |
| 3.20 | `<kiban::sstable::reader::SstTable>::iter_from` |
## bulk-512-w3-lmdb-stacks

[SVG](profile-complete-stacks/bulk-512-w3-lmdb-stacks.svg), [text report](profile-complete-stacks/bulk-512-w3-lmdb-stacks.report), [sample coverage](profile-complete-stacks/bulk-512-w3-lmdb-stacks.coverage.json)

| Self % | Symbol |
| ---: | --- |
| 19.29 | `<serde_json::read::SliceRead>::skip_to_escape` |
| 11.44 | `core::str::converts::from_utf8` |
| 9.73 | `serde_json::de::from_trait::<serde_json::read::SliceRead, kiban_readbench::model::Memory>` |
| 8.62 | `<serde_json::read::SliceRead as serde_json::read::Read>::parse_str` |
| 5.47 | `<serde_json::de::MapAccess<_> as serde_core::de::MapAccess>::next_key_seed::has_next_key::<serde_json::read::SliceRead>` |
| 4.83 | `<serde_json::de::Deserializer<serde_json::read::SliceRead>>::parse_integer` |
| 3.73 | `<core::iter::adapters::GenericShunt<core::iter::adapters::map::Map<core::iter::adapters::take::Take<heed3::iterator::prefix::RoPrefix<heed_types::bytes::Bytes, heed_types::bytes::Bytes>>, <kiban_readbench::backend::Lmdb as kiban_readbench::api::Backend>::recent::{closure#0}>, core::result::Result<core::convert::Infallible, alloc::boxed::Box<dyn core::error::Error>>> as core::iter::traits::iterator::Iterator>::next` |
| 3.54 | `mdb_node_search` |
| 3.09 | `__memcmp_avx2_movbe` |
| 2.74 | `mi_heap_malloc_aligned_at` |
| 2.18 | `mi_free` |
| 2.04 | `<kiban_readbench::backend::Lmdb as kiban_readbench::api::Backend>::recent` |
## normal-512-w2-kiban-stacks

[SVG](profile-complete-stacks/normal-512-w2-kiban-stacks.svg), [text report](profile-complete-stacks/normal-512-w2-kiban-stacks.report), [sample coverage](profile-complete-stacks/normal-512-w2-kiban-stacks.coverage.json)

| Self % | Symbol |
| ---: | --- |
| 30.02 | `__memcmp_avx2_movbe` |
| 24.34 | `kiban::db::get_from_version_at::{closure#0}` |
| 12.76 | `<kiban::cache::BlockCache>::get` |
| 8.82 | `kiban::sstable::block::header_at` |
| 6.25 | `<kiban::db::Kiban>::get::<alloc::string::String>` |
| 2.27 | `<serde_json::read::SliceRead>::skip_to_escape` |
| 2.07 | `__memmove_avx_unaligned_erms` |
| 1.65 | `core::str::converts::from_utf8` |
| 1.22 | `serde_json::de::from_trait::<serde_json::read::SliceRead, kiban_readbench::model::Memory>` |
| 1.12 | `<kiban::sstable::block::VerifiedBlock>::get` |
| 1.01 | `<serde_json::read::SliceRead as serde_json::read::Read>::parse_str` |
| 0.87 | `<serde_json::de::MapAccess<_> as serde_core::de::MapAccess>::next_key_seed::has_next_key::<serde_json::read::SliceRead>` |
## normal-512-w2-lmdb-stacks

[SVG](profile-complete-stacks/normal-512-w2-lmdb-stacks.svg), [text report](profile-complete-stacks/normal-512-w2-lmdb-stacks.report), [sample coverage](profile-complete-stacks/normal-512-w2-lmdb-stacks.coverage.json)

| Self % | Symbol |
| ---: | --- |
| 27.47 | `mdb_page_search_root` |
| 26.14 | `mdb_node_search` |
| 10.92 | `__memcmp_avx2_movbe` |
| 4.44 | `<serde_json::read::SliceRead>::skip_to_escape` |
| 3.49 | `__memset_avx2_unaligned_erms` |
| 2.60 | `core::str::converts::from_utf8` |
| 2.18 | `mdb_cmp_memn` |
| 2.18 | `serde_json::de::from_trait::<serde_json::read::SliceRead, kiban_readbench::model::Memory>` |
| 1.88 | `<serde_json::read::SliceRead as serde_json::read::Read>::parse_str` |
| 1.20 | `<kiban_readbench::backend::Lmdb as kiban_readbench::api::Backend>::get` |
| 1.11 | `alloc::fmt::format::format_inner` |
| 1.07 | `<u64>::_fmt_inner` |
## normal-512-w3-kiban-stacks

[SVG](profile-complete-stacks/normal-512-w3-kiban-stacks.svg), [text report](profile-complete-stacks/normal-512-w3-kiban-stacks.report), [sample coverage](profile-complete-stacks/normal-512-w3-kiban-stacks.coverage.json)

| Self % | Symbol |
| ---: | --- |
| 13.48 | `__memcmp_avx2_movbe` |
| 11.74 | `<serde_json::read::SliceRead>::skip_to_escape` |
| 7.29 | `core::str::converts::from_utf8` |
| 6.11 | `serde_json::de::from_trait::<serde_json::read::SliceRead, kiban_readbench::model::Memory>` |
| 6.05 | `__memmove_avx_unaligned_erms` |
| 5.47 | `<serde_json::read::SliceRead as serde_json::read::Read>::parse_str` |
| 3.92 | `<kiban::db::Kiban>::range` |
| 3.45 | `<serde_json::de::MapAccess<_> as serde_core::de::MapAccess>::next_key_seed::has_next_key::<serde_json::read::SliceRead>` |
| 3.26 | `<kiban::sstable::reader::SstTable>::iter_from` |
| 3.23 | `mi_free` |
| 3.22 | `mi_heap_malloc_aligned_at` |
| 3.02 | `_mi_page_malloc` |
## normal-512-w3-lmdb-stacks

[SVG](profile-complete-stacks/normal-512-w3-lmdb-stacks.svg), [text report](profile-complete-stacks/normal-512-w3-lmdb-stacks.report), [sample coverage](profile-complete-stacks/normal-512-w3-lmdb-stacks.coverage.json)

| Self % | Symbol |
| ---: | --- |
| 19.44 | `<serde_json::read::SliceRead>::skip_to_escape` |
| 11.86 | `core::str::converts::from_utf8` |
| 9.60 | `serde_json::de::from_trait::<serde_json::read::SliceRead, kiban_readbench::model::Memory>` |
| 8.76 | `<serde_json::read::SliceRead as serde_json::read::Read>::parse_str` |
| 5.49 | `<serde_json::de::MapAccess<_> as serde_core::de::MapAccess>::next_key_seed::has_next_key::<serde_json::read::SliceRead>` |
| 4.71 | `<serde_json::de::Deserializer<serde_json::read::SliceRead>>::parse_integer` |
| 3.76 | `<core::iter::adapters::GenericShunt<core::iter::adapters::map::Map<core::iter::adapters::take::Take<heed3::iterator::prefix::RoPrefix<heed_types::bytes::Bytes, heed_types::bytes::Bytes>>, <kiban_readbench::backend::Lmdb as kiban_readbench::api::Backend>::recent::{closure#0}>, core::result::Result<core::convert::Infallible, alloc::boxed::Box<dyn core::error::Error>>> as core::iter::traits::iterator::Iterator>::next` |
| 3.45 | `mdb_node_search` |
| 3.05 | `__memcmp_avx2_movbe` |
| 2.41 | `mi_heap_malloc_aligned_at` |
| 2.24 | `<kiban_readbench::backend::Lmdb as kiban_readbench::api::Backend>::recent` |
| 2.10 | `<core::marker::PhantomData<alloc::string::String> as serde_core::de::DeserializeSeed>::deserialize::<&mut serde_json::de::Deserializer<serde_json::read::SliceRead>>` |
