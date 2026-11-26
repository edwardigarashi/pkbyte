use std::fmt;

/// Result type returned by the builder and reader.
pub type Result<T> = std::result::Result<T, Error>;

/// A build, input, or file-validation error.
#[derive(Debug)]
pub enum Error {
    /// File signature was not recognized.
    InvalidMagic,
    /// On-disk version is unsupported.
    UnsupportedVersion(u16),
    /// A file invariant was violated.
    InvalidFormat(&'static str),
    /// Builder or reader options are inconsistent.
    InvalidOptions(&'static str),
    /// An explicit resource limit was exceeded.
    LimitExceeded(&'static str),
    /// A complete decoded string is not UTF-8.
    InvalidUtf8,
    /// A line of a text dataset is invalid.
    InvalidInput {
        /// One-based physical line number.
        line: usize,
        /// What made the record invalid.
        reason: &'static str,
    },
    /// A repeated value was found during building or uniqueness validation.
    DuplicateValue(String),
    /// Sampling requires a nonempty table.
    EmptyTable,
    /// The requested stored rank does not exist.
    OutOfBounds {
        /// Requested rank.
        index: u64,
        /// Record count.
        len: u64,
    },
    /// The initialized output slice is too short for prepared retrieval.
    OutputTooSmall {
        /// Minimum initialized length in bytes.
        required: usize,
        /// Initialized length of the supplied slice in bytes.
        provided: usize,
    },
    /// Stored and computed CRC-32 differ.
    ChecksumMismatch {
        /// Stored checksum.
        expected: u32,
        /// Computed checksum.
        actual: u32,
    },
    /// An operating-system I/O operation failed.
    Io(std::io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMagic => write!(f, "unrecognized pkbyte signature"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported pkbyte format version {v}"),
            Self::InvalidFormat(s) => write!(f, "invalid pkbyte file: {s}"),
            Self::InvalidOptions(s) => write!(f, "invalid options: {s}"),
            Self::LimitExceeded(s) => write!(f, "resource limit exceeded: {s}"),
            Self::InvalidUtf8 => write!(f, "decoded value is not valid UTF-8"),
            Self::InvalidInput { line, reason } => write!(f, "input line {line}: {reason}"),
            Self::DuplicateValue(s) => {
                let mut end = s.len().min(80);
                while !s.is_char_boundary(end) {
                    end -= 1;
                }
                write!(f, "duplicate value: {:?}", &s[..end])?;
                if end < s.len() {
                    write!(f, "… ({} bytes)", s.len())?;
                }
                Ok(())
            }
            Self::EmptyTable => write!(f, "cannot sample an empty table"),
            Self::OutOfBounds { index, len } => write!(f, "rank {index} is outside 0..{len}"),
            Self::OutputTooSmall { required, provided } => write!(
                f,
                "prepared output needs {required} initialized bytes, got {provided}"
            ),
            Self::ChecksumMismatch { expected, actual } => write!(
                f,
                "CRC-32 mismatch: expected {expected:08x}, got {actual:08x}"
            ),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        if let Self::Io(e) = self {
            Some(e)
        } else {
            None
        }
    }
}
impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
