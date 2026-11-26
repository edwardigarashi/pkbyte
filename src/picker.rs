use std::collections::hash_map::RandomState;
use std::fmt;
use std::fs::File;
use std::hash::BuildHasher;
use std::io::{self, Read};
use std::path::Path;

use crate::reader::OwnedPrepared;
use crate::{build, BuildOptions, Error, Limits, Result};

/// An owning picker over an immutable compressed table of unique strings.
///
/// Construction loads or builds the table, checks every value, and prepares
/// the decoder once. Each pick samples with replacement. The picker keeps its
/// random source and reusable storage between calls. Use [`crate::Table`] and
/// [`crate::PreparedTable`] for borrowed bytes and caller-provided randomness.
/// The built-in generator is for noncryptographic use.
///
/// ```
/// use pkbyte::Picker;
///
/// # fn main() -> pkbyte::Result<()> {
/// let mut picker = Picker::from_strings(["Tokyo", "大阪", "", "first line\nsecond line"])?;
/// let value: String = picker.pick()?;
/// # assert!(["Tokyo", "大阪", "", "first line\nsecond line"].contains(&value.as_str()));
/// # Ok(())
/// # }
/// ```
pub struct Picker {
    table: OwnedPrepared,
    rng: SplitMix64,
    output: Vec<u8>,
}

impl Picker {
    /// Load and fully validate a table with the default reader [`Limits`].
    ///
    /// Returns I/O errors for an unreadable path and the usual table validation
    /// errors for invalid or oversized files.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_limits(path, &Limits::default())
    }

    /// Load and prepare a table with explicit reader limits.
    ///
    /// The file size limit also bounds the original read, including a file that
    /// changes size while it is being loaded.
    pub fn open_with_limits(path: impl AsRef<Path>, limits: &Limits) -> Result<Self> {
        let path = path.as_ref();
        let file = File::open(path).map_err(|e| file_error("open", path, e))?;
        let size = file
            .metadata()
            .map_err(|e| file_error("read metadata for", path, e))?
            .len();
        let limit = limits.max_file_bytes as u64;
        if size > limit {
            return Err(Error::LimitExceeded("file size"));
        }
        let mut bytes = Vec::new();
        file.take(limit.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|e| file_error("read", path, e))?;
        Self::from_bytes_with_limits(bytes, limits)
    }

    /// Build a compact table from exact UTF-8 strings with default options.
    ///
    /// Accepts borrowed and owned strings. Exact duplicates return
    /// [`Error::DuplicateValue`]. Empty strings are stored as values.
    pub fn from_strings<I, S>(values: I) -> Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self::from_strings_with_options(values, &BuildOptions::default(), &Limits::default())
    }

    /// Build and prepare a table with explicit encoder options and reader limits.
    ///
    /// Set the duplicate policy in `options`. When raising a builder limit,
    /// set the corresponding reader limit to admit the resulting table.
    pub fn from_strings_with_options<I, S>(
        values: I,
        options: &BuildOptions,
        limits: &Limits,
    ) -> Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self::from_bytes_with_limits(build(values, options)?.bytes, limits)
    }

    /// Take ownership of portable table bytes and fully validate and prepare them.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self> {
        Self::from_bytes_with_limits(bytes, &Limits::default())
    }

    /// Take ownership of portable table bytes, checking them with explicit limits.
    pub fn from_bytes_with_limits(bytes: Vec<u8>, limits: &Limits) -> Result<Self> {
        Ok(Self {
            table: OwnedPrepared::new(bytes, limits)?,
            rng: SplitMix64(RandomState::new().hash_one(())),
            output: Vec::new(),
        })
    }

    /// Number of distinct stored values.
    pub fn len(&self) -> u64 {
        self.table.len()
    }

    /// Whether there are zero values. A singleton empty string is a value.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Borrow the original compressed table bytes.
    pub fn as_bytes(&self) -> &[u8] {
        self.table.as_bytes()
    }

    /// Restart the noncryptographic generator from a chosen seed.
    ///
    /// The same table bytes and seed reproduce the same sequence with the
    /// SplitMix64 generator and modulo reduction used by `pkbyte pick`.
    /// Table rebuilds or changes to the generator or reducer can change the
    /// sequence. The default generator is seeded once at construction.
    pub fn set_seed(&mut self, seed: u64) {
        self.rng = SplitMix64(seed);
    }

    /// Pick a string with replacement and return an owned copy.
    ///
    /// This generally allocates for the result. Internal decoding storage is
    /// retained after the first pick. An empty table returns [`Error::EmptyTable`].
    pub fn pick(&mut self) -> Result<String> {
        let value = self
            .table
            .sample_into(&mut || self.rng.next(), &mut self.output)?;
        let mut owned = String::new();
        owned
            .try_reserve_exact(value.len())
            .map_err(|_| Error::LimitExceeded("picked string allocation"))?;
        owned.push_str(value);
        Ok(owned)
    }

    /// Pick with replacement into a reusable byte vector.
    ///
    /// The vector grows on demand. Calls with sufficient capacity allocate no
    /// memory. The returned string is only a prefix of the initialized storage.
    /// Consume it before reusing the buffer. An empty table returns
    /// [`Error::EmptyTable`].
    #[inline]
    pub fn pick_into<'a>(&mut self, output: &'a mut Vec<u8>) -> Result<&'a str> {
        self.table.sample_into(&mut || self.rng.next(), output)
    }
}

impl fmt::Debug for Picker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Picker")
            .field("values", &self.len())
            .field("file_bytes", &self.as_bytes().len())
            .finish_non_exhaustive()
    }
}

fn file_error(operation: &str, path: &Path, error: io::Error) -> Error {
    io::Error::new(
        error.kind(),
        format!("cannot {operation} {path:?}: {error}"),
    )
    .into()
}

// SplitMix64, the noncryptographic generator used for the CLI's seeded picks.
struct SplitMix64(u64);

impl SplitMix64 {
    #[inline]
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut value = self.0;
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^ (value >> 31)
    }
}
