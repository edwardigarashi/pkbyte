//! Command-line builder and checked reader for pkbyte tables.
#![forbid(unsafe_code)]

use pkbyte::build_support::{build_from_reader, write_table, InputFormat as RecordFormat};
use pkbyte::{
    BuildMode, BuildOptions, BuildStats, DuplicatePolicy, Error, Limits, OpenOptions, Picker,
    PrepareOptions, Table,
};
use std::collections::BTreeSet;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::File;
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

#[path = "pkbyte/help.rs"]
mod help;
#[path = "pkbyte/jsonl.rs"]
mod jsonl;

type Result<T> = std::result::Result<T, CliError>;

enum CliError {
    Argument(String),
    Library(pkbyte::Error),
    Input {
        operation: &'static str,
        source: OsString,
        error: pkbyte::Error,
        advice: Option<String>,
    },
    Stdout(io::Error),
    File {
        operation: &'static str,
        path: PathBuf,
        error: io::Error,
    },
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Argument(message) => f.write_str(message),
            Self::Library(error) => error.fmt(f),
            Self::Input {
                operation,
                source,
                error,
                advice,
            } => {
                write!(f, "Could not {operation} {}. {error}.", input_name(source))?;
                if let Some(advice) = advice {
                    write!(f, " {advice}")?;
                }
                Ok(())
            }
            Self::Stdout(error) => error.fmt(f),
            Self::File {
                operation,
                path,
                error,
            } => {
                write!(f, "{operation} {path:?} failed. {error}")
            }
        }
    }
}

impl From<pkbyte::Error> for CliError {
    fn from(error: pkbyte::Error) -> Self {
        Self::Library(error)
    }
}

fn invalid(message: impl Into<String>) -> CliError {
    CliError::Argument(message.into())
}

fn unsigned<T: FromStr>(value: &OsStr, label: &str) -> Result<T> {
    value
        .to_str()
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| invalid(format!("{label} must be an unsigned integer in range")))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Command {
    Pack,
    Build,
    Info,
    Inspect,
    Verify,
    Pick,
    Sample,
    Unpack,
    Export,
}

impl Command {
    fn parse(command: &OsStr) -> Result<Self> {
        match command.to_str() {
            Some("pack") => Ok(Self::Pack),
            Some("build") => Ok(Self::Build),
            Some("info") => Ok(Self::Info),
            Some("inspect") => Ok(Self::Inspect),
            Some("verify") => Ok(Self::Verify),
            Some("pick") => Ok(Self::Pick),
            Some("sample") => Ok(Self::Sample),
            Some("unpack") => Ok(Self::Unpack),
            Some("export") => Ok(Self::Export),
            _ => Err(invalid(format!(
                "Unknown command {command:?}. See 'pkbyte --help'."
            ))),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Pack => "pack",
            Self::Build => "build",
            Self::Info => "info",
            Self::Inspect => "inspect",
            Self::Verify => "verify",
            Self::Pick => "pick",
            Self::Sample => "sample",
            Self::Unpack => "unpack",
            Self::Export => "export",
        }
    }

    fn is_legacy(self) -> bool {
        matches!(
            self,
            Self::Build | Self::Inspect | Self::Sample | Self::Export
        )
    }

    fn picks(self) -> bool {
        matches!(self, Self::Pick | Self::Sample)
    }

    fn unpacks(self) -> bool {
        matches!(self, Self::Unpack | Self::Export)
    }

    fn argument_error(self, argument: &OsStr) -> CliError {
        let name = self.name();
        invalid(format!(
            "Unexpected {name} argument {argument:?}. See 'pkbyte {name} --help'."
        ))
    }
}

fn positional(argument: &OsStr) -> bool {
    argument == "-" || !argument.as_encoded_bytes().starts_with(b"-")
}

fn set_format(setting: &mut Option<RecordFormat>, format: RecordFormat) -> Result<()> {
    if setting.is_some_and(|current| current != format) {
        return Err(invalid("Choose one record format."));
    }
    *setting = Some(format);
    Ok(())
}

struct Flags<'a> {
    arguments: std::slice::Iter<'a, OsString>,
    seen: BTreeSet<&'a str>,
}

impl<'a> Flags<'a> {
    fn new(arguments: &'a [OsString]) -> Self {
        Self {
            arguments: arguments.iter(),
            seen: BTreeSet::new(),
        }
    }

    fn next(&mut self) -> Option<&'a OsString> {
        self.arguments.next()
    }

    fn once(&mut self, flag: &'a str) -> Result<()> {
        if !self.seen.insert(flag) {
            return Err(invalid(format!("{flag} may appear only once")));
        }
        Ok(())
    }

    fn value(&mut self, flag: &'a str) -> Result<&'a OsStr> {
        self.once(flag)?;
        self.next()
            .map(OsString::as_os_str)
            .ok_or_else(|| invalid(format!("{flag} requires a value")))
    }

    fn number<T: FromStr>(&mut self, flag: &'a str) -> Result<T> {
        unsigned(self.value(flag)?, flag)
    }

    fn set_format(&mut self, setting: &mut Option<RecordFormat>) -> Result<()> {
        let value = self.value("--format")?;
        let format = match value.to_str() {
            Some("lines") => RecordFormat::Lines,
            Some("jsonl") => RecordFormat::JsonLines,
            _ => {
                return Err(invalid(format!(
                    "Unknown record format {value:?}. Use --format lines or --format jsonl."
                )))
            }
        };
        set_format(setting, format)
    }

    fn value_bytes(&mut self, flag: &'a str) -> Result<usize> {
        let maximum = self.number::<u32>(flag)?;
        usize::try_from(maximum)
            .map_err(|_| invalid(format!("{flag} exceeds this platform's range")))
    }

    fn apply_read_limit(&mut self, flag: &'a str, limits: &mut Limits) -> Result<bool> {
        match flag {
            "--max-file-bytes" => limits.max_file_bytes = self.number(flag)?,
            "--max-rows" => limits.max_rows = self.number(flag)?,
            "--max-decoded-bytes" => limits.max_decoded_bytes = self.number(flag)?,
            "--max-value-bytes" => limits.max_value_bytes = self.value_bytes(flag)?,
            "--max-groups" => limits.max_groups = self.number(flag)?,
            _ => return Ok(false),
        }
        Ok(true)
    }
}

fn file_error(operation: &'static str, path: &Path, error: io::Error) -> CliError {
    CliError::File {
        operation,
        path: path.to_owned(),
        error,
    }
}

fn input_name(path: &OsStr) -> String {
    if path == "-" {
        "stdin".into()
    } else {
        format!("{path:?}")
    }
}

fn input_file(path: &OsStr, maximum: usize, flag: &str) -> Result<File> {
    let path = Path::new(path);
    let file = File::open(path).map_err(|error| file_error("Opening", path, error))?;
    let metadata = file
        .metadata()
        .map_err(|error| file_error("Reading metadata for", path, error))?;
    if metadata.len() > maximum as u64 {
        return Err(invalid(format!(
            "{path:?} exceeds the CLI input limit of {maximum} bytes. See {flag}."
        )));
    }
    Ok(file)
}

fn read_bounded(input: impl Read, source: &OsStr, maximum: usize) -> Result<Vec<u8>> {
    // Check one extra byte for stdin or a file growing after its metadata was read.
    let mut bytes = Vec::new();
    input
        .take((maximum as u64).saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| file_error("Reading", Path::new(source), error))?;
    if bytes.len() > maximum {
        return Err(invalid(format!(
            "{} exceeds the CLI input limit of {maximum} bytes. See --max-file-bytes.",
            input_name(source)
        )));
    }
    Ok(bytes)
}

fn table_bytes(path: &OsStr, maximum: usize) -> Result<Vec<u8>> {
    if path == "-" {
        read_bounded(io::stdin().lock(), path, maximum)
    } else {
        read_bounded(
            input_file(path, maximum, "--max-file-bytes")?,
            path,
            maximum,
        )
    }
}

fn codec_flag(flag: &str) -> Option<BuildMode> {
    match flag {
        "--raw" => Some(BuildMode::Raw),
        "--phrase" => Some(BuildMode::Phrase),
        "--ef-mixed" => Some(BuildMode::EliasFanoMixed),
        "--ef-power-of-two" => Some(BuildMode::EliasFanoPowerOfTwo),
        _ => None,
    }
}

fn print_build_report(
    diagnostics: &mut impl Write,
    destination: &OsStr,
    stats: &BuildStats,
) -> io::Result<()> {
    let target = if destination == "-" {
        "stdout".into()
    } else {
        format!("{destination:?}")
    };
    writeln!(diagnostics, "Wrote {target}")?;
    writeln!(diagnostics, "Stored values: {}", stats.row_count)?;
    writeln!(
        diagnostics,
        "Duplicate input rows removed: {}",
        stats.duplicate_rows
    )?;
    writeln!(
        diagnostics,
        "Original value bytes: {}",
        stats.raw_value_bytes
    )?;
    writeln!(diagnostics, "Complete file bytes: {}", stats.packed_bytes)?;
    writeln!(
        diagnostics,
        "Dictionary symbols: {}",
        stats.dictionary_symbols
    )?;
    writeln!(diagnostics, "Raw rows: {}", stats.raw_rows)?;
    writeln!(diagnostics, "Compressed rows: {}", stats.compressed_rows)?;
    writeln!(
        diagnostics,
        "Elias–Fano mixed rows: {}",
        stats.ef_mixed_rows
    )?;
    writeln!(
        diagnostics,
        "Elias–Fano power-of-two rows: {}",
        stats.ef_power_of_two_rows
    )?;
    writeln!(diagnostics, "Elias–Fano block bytes: {}", stats.ef_bytes)
}

fn build_error(
    command: Command,
    source: &OsStr,
    format: RecordFormat,
    options: &BuildOptions,
    error: Error,
) -> CliError {
    let advice = match &error {
        Error::DuplicateValue(_) => Some("Use --deduplicate to keep one copy.".into()),
        Error::InvalidInput { reason, .. } if reason.contains("configured byte limit") => {
            Some(format!(
                "The value limit is {} UTF-8 bytes. See --max-value-bytes.",
                options.max_value_bytes
            ))
        }
        Error::InvalidInput { reason, .. }
            if format == RecordFormat::JsonLines && !reason.contains("UTF-8") =>
        {
            Some("Use one JSON string per physical line, or choose --format lines for plain text.".into())
        }
        Error::LimitExceeded(_) => Some(format!(
            "Input limits are {} bytes, {} rows, and {} UTF-8 bytes per value. See 'pkbyte {} --help'.",
            options.max_input_bytes,
            options.max_input_rows,
            options.max_value_bytes,
            command.name()
        )),
        _ => None,
    };
    CliError::Input {
        operation: "pack",
        source: source.to_owned(),
        error,
        advice,
    }
}

fn inferred_output(source: &OsStr) -> Result<OsString> {
    if source == "-" {
        return Err(invalid(
            "Packing stdin requires -o OUTPUT. Use -o - for binary stdout.",
        ));
    }
    let path = Path::new(source);
    if path.file_name().is_none()
        || path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("npk"))
    {
        return Err(invalid(format!(
            "Choose an output path with -o OUTPUT when packing {path:?}."
        )));
    }
    Ok(path.with_extension("npk").into_os_string())
}

fn build_command(command: Command, arguments: &[OsString], output: &mut impl Write) -> Result<()> {
    let (mut source, mut destination, arguments) = if command == Command::Build {
        let [source, destination, tail @ ..] = arguments else {
            return Err(invalid(
                "build requires INPUT and OUTPUT. See 'pkbyte build --help'.",
            ));
        };
        (
            Some(source.as_os_str()),
            Some(destination.as_os_str()),
            tail,
        )
    } else {
        (None, None, arguments)
    };
    let mut options = BuildOptions::default();
    let mut format = None;
    let mut replace = false;
    let mut flags = Flags::new(arguments);
    while let Some(argument) = flags.next() {
        let flag = argument.to_str().unwrap_or_default();
        match flag {
            "--deduplicate" => options.duplicate_policy = DuplicatePolicy::Deduplicate,
            "--jsonl" if command == Command::Build => {
                set_format(&mut format, RecordFormat::JsonLines)?
            }
            "--format" => flags.set_format(&mut format)?,
            "-o" | "--output" if command == Command::Pack => {
                let path = flags.value("--output")?;
                if !positional(path) {
                    return Err(invalid("-o or --output requires an output path."));
                }
                destination = Some(path);
            }
            "--force" => replace = true,
            "--max-input-bytes" => options.max_input_bytes = flags.number(flag)?,
            "--max-input-rows" => options.max_input_rows = flags.number(flag)?,
            "--max-value-bytes" => options.max_value_bytes = flags.value_bytes(flag)?,
            _ if source.is_none() && positional(argument) => source = Some(argument),
            _ => {
                let mode = codec_flag(flag).ok_or_else(|| command.argument_error(argument))?;
                if options.mode != BuildMode::Compact && options.mode != mode {
                    return Err(invalid("Choose one encoding option."));
                }
                options.mode = mode;
            }
        }
    }

    let source = source.ok_or_else(|| invalid("pack requires INPUT. See 'pkbyte pack --help'."))?;
    let inferred;
    let destination = match destination {
        Some(destination) => destination,
        None => {
            inferred = inferred_output(source)?;
            inferred.as_os_str()
        }
    };
    if destination == "-" && replace {
        return Err(invalid("--force requires an output file."));
    }
    let format = format.unwrap_or_else(|| {
        if command == Command::Pack
            && Path::new(source)
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("jsonl"))
        {
            RecordFormat::JsonLines
        } else {
            RecordFormat::Lines
        }
    });
    let packed = if source == "-" {
        build_from_reader(io::stdin().lock(), format, &options)
    } else {
        build_from_reader(
            input_file(source, options.max_input_bytes, "--max-input-bytes")?,
            format,
            &options,
        )
    }
    .map_err(|error| build_error(command, source, format, &options, error))?;

    if destination == "-" {
        output.write_all(&packed.bytes).map_err(CliError::Stdout)?;
        output.flush().map_err(CliError::Stdout)?;
    } else {
        let path = Path::new(destination);
        write_table(path, &packed, replace).map_err(|error| {
            if error.kind() == io::ErrorKind::AlreadyExists && !replace {
                invalid(format!(
                    "Output {path:?} already exists. Choose another path or use --force."
                ))
            } else {
                file_error("Writing", path, error)
            }
        })?;
    }
    let mut diagnostics = io::stderr().lock();
    if command == Command::Build {
        let _ = print_build_report(&mut diagnostics, destination, &packed.stats);
    } else {
        let target = if destination == "-" {
            "table to stdout".into()
        } else {
            format!("{destination:?}")
        };
        let _ = writeln!(
            diagnostics,
            "Wrote {target} ({} values, {} bytes).",
            packed.stats.row_count, packed.stats.packed_bytes
        );
    }
    Ok(())
}

fn inspect(table: &Table<'_>, output: &mut impl Write) -> io::Result<()> {
    let stats = table.stats();
    writeln!(output, "Format version: {}", table.version())?;
    writeln!(output, "Stored values: {}", table.len())?;
    writeln!(output, "Maximum value bytes: {}", table.max_value_bytes())?;
    writeln!(output, "Complete file bytes: {}", stats.file_bytes)?;
    writeln!(output, "Structure and CRC-32: verified")?;
    writeln!(output, "Declared decoded bytes: {}", stats.raw_value_bytes)?;
    writeln!(output, "Payload bytes: {}", stats.payload_bytes)?;
    writeln!(output, "Length groups: {}", stats.groups)?;
    writeln!(output, "Dictionary symbols: {}", stats.dictionary_symbols)?;
    writeln!(output, "Dictionary bytes: {}", stats.dictionary_bytes)?;
    writeln!(output, "Raw rows: {}", stats.raw_rows)?;
    writeln!(output, "Compressed rows: {}", stats.compressed_rows)?;
    writeln!(output, "Elias–Fano mixed rows: {}", stats.ef_mixed_rows)?;
    writeln!(
        output,
        "Elias–Fano power-of-two rows: {}",
        stats.ef_power_of_two_rows
    )?;
    writeln!(output, "Elias–Fano block bytes: {}", stats.ef_bytes)?;
    writeln!(
        output,
        "Use 'verify FILE' to check every decoded value and uniqueness."
    )
}

fn verify(table: &Table<'_>, output: &mut impl Write) -> Result<()> {
    let report = table.validate_all(true)?;
    writeln!(
        output,
        "Structure, CRC-32, UTF-8, and decoded metadata: verified\n\
        Exact-byte value uniqueness: verified\n\
        Verified rows: {}\n\
        Decoded value bytes: {}",
        report.row_count, report.decoded_bytes
    )
    .map_err(CliError::Stdout)
}

fn check_lines(value: &str, command: Command, record: u64) -> Result<()> {
    if value.is_empty() || value.contains('\n') || value.ends_with('\r') {
        let option = if command.is_legacy() {
            "--jsonl"
        } else {
            "--format jsonl"
        };
        return Err(invalid(format!(
            "Output record {record} cannot be represented in lines. Use {option} to preserve it."
        )));
    }
    Ok(())
}

fn write_value(output: &mut impl Write, format: RecordFormat, value: &str) -> Result<()> {
    match format {
        RecordFormat::JsonLines => jsonl::write_record(output, value),
        RecordFormat::Lines => output
            .write_all(value.as_bytes())
            .and_then(|_| output.write_all(b"\n")),
    }
    .map_err(CliError::Stdout)
}

fn default_seed() -> u64 {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    elapsed.as_secs().rotate_left(23)
        ^ u64::from(elapsed.subsec_nanos())
        ^ u64::from(std::process::id()).rotate_left(41)
}

fn sample(
    picker: &mut Picker,
    command: Command,
    output: &mut impl Write,
    count: u64,
    seed: Option<u64>,
    format: RecordFormat,
) -> Result<()> {
    if picker.is_empty() {
        return Err(Error::EmptyTable.into());
    }
    if let Some(seed) = seed.or_else(|| (command.is_legacy() || count > 1).then(default_seed)) {
        picker.set_seed(seed);
        let _ = writeln!(
            io::stderr().lock(),
            "Sampling with replacement using noncryptographic SplitMix64 (seed={seed})."
        );
    }
    let mut scratch = Vec::new();
    for index in 0..count {
        let value = picker.pick_into(&mut scratch)?;
        if format == RecordFormat::Lines {
            check_lines(value, command, index + 1)?;
        }
        write_value(output, format, value)?;
    }
    Ok(())
}

fn export(
    table: &Table<'_>,
    command: Command,
    output: &mut impl Write,
    format: RecordFormat,
) -> Result<()> {
    let prepared = table.prepare(&PrepareOptions::default())?;
    let mut scratch = prepared.new_output_buffer()?;
    if format == RecordFormat::Lines {
        for rank in 0..prepared.len() {
            check_lines(prepared.get_into(rank, &mut scratch)?, command, rank + 1)?;
        }
    }
    for rank in 0..prepared.len() {
        let value = prepared.get_into(rank, &mut scratch)?;
        write_value(output, format, value)?;
    }
    Ok(())
}

fn table_error(source: &OsStr, limits: &Limits, error: Error) -> CliError {
    let advice = match &error {
        Error::InvalidMagic | Error::InvalidFormat("truncated header") => Some(
            "Give this command a pkbyte table. Use 'pkbyte pack' to compile a text dataset.".into(),
        ),
        Error::UnsupportedVersion(_) => Some(
            "Use a pkbyte reader that supports this version or rebuild from the original values."
                .into(),
        ),
        Error::EmptyTable => Some("Pack at least one value before picking.".into()),
        Error::LimitExceeded(reason) => {
            let setting = match *reason {
                "file size" => Some(("--max-file-bytes", limits.max_file_bytes as u64)),
                "row count" => Some(("--max-rows", limits.max_rows)),
                "decoded byte total" => Some(("--max-decoded-bytes", limits.max_decoded_bytes)),
                "decoded value size" => Some(("--max-value-bytes", limits.max_value_bytes as u64)),
                "group count" => Some(("--max-groups", limits.max_groups as u64)),
                _ => None,
            };
            setting.map(|(flag, maximum)| format!("The current limit is {maximum}. See {flag}."))
        }
        _ => None,
    };
    CliError::Input {
        operation: "read table from",
        source: source.to_owned(),
        error,
        advice,
    }
}

fn read_command(command: Command, arguments: &[OsString], output: &mut impl Write) -> Result<()> {
    let missing_table = || {
        let name = command.name();
        invalid(format!(
            "{name} requires TABLE. See 'pkbyte {name} --help'."
        ))
    };
    let (mut path, arguments) = if command.is_legacy() {
        let (path, tail) = arguments.split_first().ok_or_else(missing_table)?;
        (Some(path.as_os_str()), tail)
    } else {
        (None, arguments)
    };
    let mut options = OpenOptions::default();
    let mut count = None;
    let mut seed = None;
    let mut format = None;
    let mut flags = Flags::new(arguments);
    while let Some(argument) = flags.next() {
        let flag = argument.to_str().unwrap_or_default();
        match flag {
            "--unique" if command == Command::Verify => flags.once(flag)?,
            "--seed" if command.picks() => seed = Some(flags.number(flag)?),
            "-n" | "--count" if command == Command::Pick => {
                count = Some(flags.number("--count")?);
            }
            "--format" if command.picks() || command.unpacks() => flags.set_format(&mut format)?,
            "--jsonl" if matches!(command, Command::Sample | Command::Export) => {
                set_format(&mut format, RecordFormat::JsonLines)?
            }
            "--plain" if command == Command::Export => {
                set_format(&mut format, RecordFormat::Lines)?
            }
            _ if flags.apply_read_limit(flag, &mut options.limits)? => {}
            _ if path.is_none() && positional(argument) => path = Some(argument),
            _ if command == Command::Sample && count.is_none() && !flag.starts_with("--") => {
                count = Some(unsigned(argument, "sample count")?);
            }
            _ => return Err(command.argument_error(argument)),
        }
    }

    let path = path.ok_or_else(missing_table)?;
    let count = count.unwrap_or(1);
    let format = format.unwrap_or(if command == Command::Sample {
        RecordFormat::Lines
    } else {
        RecordFormat::JsonLines
    });
    // Full validation and preparation perform their own checksum pass.
    options.verify_checksum =
        matches!(command, Command::Info | Command::Inspect) || (command.picks() && count == 0);
    let bytes = table_bytes(path, options.limits.max_file_bytes)?;
    let contextualize = |error| table_error(path, &options.limits, error);
    let result = if command.picks() && count > 0 {
        let mut picker =
            Picker::from_bytes_with_limits(bytes, &options.limits).map_err(contextualize)?;
        sample(&mut picker, command, output, count, seed, format)
    } else {
        let table = Table::open_with_options(&bytes, &options).map_err(contextualize)?;
        match command {
            Command::Info | Command::Inspect => inspect(&table, output).map_err(CliError::Stdout),
            Command::Verify => verify(&table, output),
            Command::Unpack | Command::Export => export(&table, command, output, format),
            Command::Pick | Command::Sample => Ok(()),
            Command::Pack | Command::Build => unreachable!("writer dispatched to reader"),
        }
    };
    result.map_err(|error| match error {
        CliError::Library(error) => contextualize(error),
        error => error,
    })
}

fn print_help(command: Option<Command>, output: &mut impl Write) -> Result<()> {
    let (main, options) = command.map_or((help::GENERAL, ""), help::for_command);
    output
        .write_all(main.as_bytes())
        .and_then(|()| output.write_all(options.as_bytes()))
        .map_err(CliError::Stdout)
}

fn run(arguments: &[OsString], output: &mut impl Write) -> Result<()> {
    let Some((command, arguments)) = arguments.split_first() else {
        return print_help(None, output);
    };
    match command.to_str() {
        Some("--help" | "-h") => return print_help(None, output),
        Some("help") => {
            return match arguments {
                [] => print_help(None, output),
                [name] => print_help(Some(Command::parse(name)?), output),
                _ => Err(invalid("Use 'pkbyte help COMMAND' or 'pkbyte --help'.")),
            };
        }
        Some("--version" | "-V") if arguments.is_empty() => {
            return writeln!(output, "pkbyte {}", env!("CARGO_PKG_VERSION"))
                .map_err(CliError::Stdout);
        }
        _ => {}
    }
    let command = Command::parse(command)?;
    let help_flag = |argument: &OsString| argument == "--help" || argument == "-h";
    if matches!(arguments, [flag] if help_flag(flag))
        || (!command.is_legacy() && arguments.iter().any(help_flag))
    {
        return print_help(Some(command), output);
    }
    match command {
        Command::Pack | Command::Build => build_command(command, arguments, output),
        _ => read_command(command, arguments, output),
    }
}

fn main() -> ExitCode {
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    let mut output = BufWriter::new(io::stdout().lock());
    let result =
        run(&arguments, &mut output).and_then(|()| output.flush().map_err(CliError::Stdout));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(CliError::Stdout(error)) if error.kind() == io::ErrorKind::BrokenPipe => {
            ExitCode::SUCCESS
        }
        Err(error) => {
            let _ = writeln!(io::stderr().lock(), "pkbyte: {error}");
            ExitCode::FAILURE
        }
    }
}
