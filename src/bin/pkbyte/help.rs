use super::Command;

pub(super) const GENERAL: &str = "\
pkbyte packs exact UTF-8 strings into compact tables and picks stored values.

Usage
  pkbyte pack INPUT
  pkbyte pick TABLE [-n COUNT] [--seed SEED]
  pkbyte info TABLE
  pkbyte verify TABLE
  pkbyte unpack TABLE

Examples
  pkbyte pack values.txt
  pkbyte pick values.npk
  pkbyte pick values.npk -n 10 --seed 42

Use 'pkbyte COMMAND --help' for advanced options or 'pkbyte --version'.
The build, sample, inspect, and export commands retain their original syntax.
";

const PACK: &str = "\
Usage
  pkbyte pack INPUT [-o OUTPUT] [--format lines|jsonl] [OPTIONS]

The default output replaces the input extension with .npk or appends .npk
when the input has no extension. Use -o or --output to choose a path.
An input whose extension is already .npk requires an explicit output path.
Use INPUT '-' for stdin and supply -o OUTPUT. Use -o - for binary stdout.

Lines is the default for stdin and paths without a .jsonl extension.
The .jsonl extension is matched case-insensitively. --format overrides it.
Lines uses LF or CRLF, accepts a final record without a newline, and rejects
blank records. It preserves bytes outside the delimiters. JSON Lines accepts
one JSON string per physical line, including an empty string and escaped controls.

Examples
  pkbyte pack values.txt
  pkbyte pack values.jsonl -o exact.npk
  pkbyte pack - --format jsonl -o exact.npk
";

const BUILD: &str = "\
Usage
  pkbyte build INPUT OUTPUT [--jsonl] [OPTIONS]

This alias expects the output path as the second argument and reads lines
by default. Use --jsonl to read one JSON string per physical line.
Use '-' for standard input or standard output.

Example
  pkbyte build values.txt values.npk
";

const PACK_OPTIONS: &str = "\n\
Options
  --force             Replace an existing output file.
  --deduplicate       Keep one copy of each duplicate value.
  --raw               Use raw storage.
  --phrase            Compare raw and phrase storage.
  --ef-mixed          Try mixed-radix Elias–Fano with raw fallback.
  --ef-power-of-two   Try power-of-two Elias–Fano with raw fallback.
  --max-input-bytes N Limit encoded input bytes.
  --max-input-rows N  Limit input rows before deduplication.
  --max-value-bytes N Limit UTF-8 bytes per decoded value.

The default compares all storage choices.
";

const PICK: &str = "\
Usage
  pkbyte pick TABLE [-n COUNT] [--seed SEED] [--format jsonl|lines]

Pick one value by default. Use -n or --count for multiple picks with
replacement. A count of zero checks structure and the checksum without
producing values. --seed uses reproducible SplitMix64 and modulo reduction.
For the same sequence, keep the table file and seed fixed.

Each output record is a JSON string by default. Use --format lines for
nonempty values without embedded LF or trailing CR. A lines framing error
can follow earlier successful picks. Use TABLE '-' for stdin.

Examples
  pkbyte pick values.npk
  pkbyte pick values.npk -n 10 --seed 42
  pkbyte pick values.npk --format lines
";

const SAMPLE: &str = "\
Usage
  pkbyte sample TABLE [COUNT] [--seed SEED] [--jsonl]

This alias uses a positional count and defaults to plain lines.
It picks with replacement. --jsonl supports all UTF-8 values.
Use TABLE '-' for stdin.

Example
  pkbyte sample values.npk 10 --seed 42 --jsonl
";

const INFO: &str = "\
Usage
  pkbyte info TABLE

Show a table's value count, sizes, format, and storage summary. This checks
the file structure and checksum. Use 'pkbyte verify' to check every value.
Use TABLE '-' for stdin.

Example
  pkbyte info values.npk
";

const INSPECT: &str = "\
Usage
  pkbyte inspect TABLE

Alias for 'pkbyte info'. Use TABLE '-' for stdin.

Example
  pkbyte inspect values.npk
";

const VERIFY: &str = "\
Usage
  pkbyte verify TABLE

Check structure, checksum, decoded metadata, UTF-8, and exact uniqueness.
Use TABLE '-' for stdin. --unique is accepted for compatibility.

Example
  pkbyte verify values.npk
";

const UNPACK: &str = "\
Usage
  pkbyte unpack TABLE [--format jsonl|lines]

Write every value once in table rank order. The default is one JSON string
per line. Use --format lines for nonempty values without embedded LF or
trailing CR. This mode checks every value before output. Use TABLE '-' for stdin.

Examples
  pkbyte unpack values.npk > values.jsonl
  pkbyte unpack values.npk --format lines
";

const EXPORT: &str = "\
Usage
  pkbyte export TABLE [--jsonl | --plain]

Alias for 'pkbyte unpack'. It defaults to JSON Lines, with --plain to
request lines. Use TABLE '-' for stdin.

Example
  pkbyte export values.npk > values.jsonl
";

const READ_LIMITS: &str = "\n\
Limits
  --max-file-bytes N    Limit table file bytes.
  --max-rows N          Limit stored values.
  --max-value-bytes N   Limit UTF-8 bytes per decoded value.
  --max-decoded-bytes N Limit the total decoded UTF-8 bytes.
  --max-groups N        Limit storage groups.
";

pub(super) fn for_command(command: Command) -> (&'static str, &'static str) {
    let usage = match command {
        Command::Pack => PACK,
        Command::Build => BUILD,
        Command::Info => INFO,
        Command::Inspect => INSPECT,
        Command::Verify => VERIFY,
        Command::Pick => PICK,
        Command::Sample => SAMPLE,
        Command::Unpack => UNPACK,
        Command::Export => EXPORT,
    };
    let options = if matches!(command, Command::Pack | Command::Build) {
        PACK_OPTIONS
    } else {
        READ_LIMITS
    };
    (usage, options)
}
