//! Compact immutable UTF-8 string tables with exact, uniform sampling.
//!
//! [`Picker::from_strings`] builds a table in memory and prepares it for
//! repeated draws. Each draw samples with replacement.
//!
//! ```
//! use pkbyte::Picker;
//!
//! fn main() -> pkbyte::Result<()> {
//!     let values = ["Tokyo", "大阪", "", "first line\nsecond line"];
//!     let mut picker = Picker::from_strings(values)?;
//!     let value = picker.pick()?;
//!     assert!(values.contains(&value.as_str()));
//!     println!("{value:?}");
//!     Ok(())
//! }
//! ```
//!
//! Use [`Picker::open`] to load a portable table from a file.
//!
//! ## Working with table bytes
//!
//! [`build`] preserves exact UTF-8, including empty strings and controls, and
//! rejects duplicate values by default. Its [`BuildOutput`] owns the portable
//! bytes. The writer emits wire version 3 when using Elias–Fano and version 2
//! otherwise. The checked reader also accepts legacy wire version 1.
//!
//! [`Table`] borrows immutable bytes from an external file, an embedded table,
//! or [`build_support::generate`]. [`Table::prepare`] verifies the CRC, every
//! decoded value, and exact uniqueness. Keep the checked table and its bytes
//! alive while using the prepared reader. Callers can share the reader across
//! threads with separate output buffers and random sources.
//!
//! Sampling draws with replacement from the table's local ranks. Supply
//! independent uniform `u64` draws for exact uniformity. Prepared retrieval
//! and sampling reuse initialized storage of at least
//! [`PreparedTable::output_buffer_len`] bytes.
//!
//! The [wire format] and [reader design] describe versioning and limits.
//!
//! [wire format]: https://github.com/edwardigarashi/pkbyte/blob/v0.1.1/docs/FORMAT.md
//! [reader design]: https://github.com/edwardigarashi/pkbyte/blob/v0.1.1/docs/DESIGN.md
//!
//! ```
//! use pkbyte::{build, BuildOptions, PrepareOptions, Table};
//!
//! # fn main() -> pkbyte::Result<()> {
//! let packed = build(["", "  exact value  "], &BuildOptions::default())?;
//! let table = Table::open_structural(&packed.bytes)?;
//! let prepared = table.prepare(&PrepareOptions::default())?;
//! let mut output = prepared.new_output_buffer()?;
//! assert_eq!(prepared.get_into(0, &mut output)?, "");
//! assert_eq!(prepared.get_into(1, &mut output)?, "  exact value  ");
//! # Ok(())
//! # }
//! ```
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod build_support;
mod builder;
mod ef;
mod error;
pub(crate) mod format;
mod picker;
mod reader;
mod sampling;
mod text;

pub use builder::{build, BuildMode, BuildOptions, BuildOutput, BuildStats, DuplicatePolicy};
pub use error::{Error, Result};
pub use picker::Picker;
pub use reader::{
    Limits, OpenOptions, PrepareOptions, PreparedTable, Table, TableStats, ValidationReport,
};
pub use sampling::{uniform_below, SamplingMethod};
