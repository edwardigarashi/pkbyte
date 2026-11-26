use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

#[path = "support/checksum.rs"]
mod checksum;
use checksum::refresh_crc;

static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(0);

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new() -> Self {
        loop {
            let serial = NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("pkbyte-test-{}-{serial}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create test directory: {error}"),
            }
        }
    }

    fn file(&self, filename: &str) -> PathBuf {
        self.0.join(filename)
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn command(arguments: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pkbyte"))
        .args(arguments)
        .output()
        .expect("run pkbyte")
}

fn command_with_input(arguments: &[&std::ffi::OsStr], input: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_pkbyte"))
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start pkbyte");
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().expect("wait for pkbyte")
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn build(input: &Path, output: &Path, options: &[&str]) -> Output {
    let mut args = vec![
        std::ffi::OsStr::new("build"),
        input.as_os_str(),
        output.as_os_str(),
    ];
    args.extend(options.iter().map(std::ffi::OsStr::new));
    command(&args)
}

fn decoded_values(path: &Path) -> BTreeSet<String> {
    let bytes = fs::read(path).unwrap();
    decoded_bytes(&bytes)
}

fn decoded_bytes(bytes: &[u8]) -> BTreeSet<String> {
    let table = pkbyte::Table::open(bytes).unwrap();
    table.validate_all(true).unwrap();
    (0..table.len())
        .map(|rank| table.get(rank).unwrap())
        .collect()
}

#[test]
fn cli_help_introduces_the_canonical_commands_and_their_options() {
    let default = command(&[]);
    assert_success(&default);
    assert!(default.stderr.is_empty());
    let help = std::str::from_utf8(&default.stdout).unwrap();
    for example in [
        "pkbyte pack values.txt",
        "pkbyte pick values.npk",
        "pkbyte pick values.npk -n 10 --seed 42",
    ] {
        assert!(help.contains(example));
    }
    for flag in ["-h", "--help", "help"] {
        let result = command(&[flag.as_ref()]);
        assert_success(&result);
        assert_eq!(result.stdout, default.stdout);
        assert!(result.stderr.is_empty());
    }
    for (operation, example, flag) in [
        (
            "pack",
            "pkbyte pack values.jsonl -o exact.npk",
            "--deduplicate",
        ),
        (
            "pick",
            "pkbyte pick values.npk -n 10 --seed 42",
            "replacement",
        ),
        ("info", "pkbyte info values.npk", "--max-file-bytes"),
        ("verify", "pkbyte verify values.npk", "--max-decoded-bytes"),
        (
            "unpack",
            "pkbyte unpack values.npk > values.jsonl",
            "--format lines",
        ),
    ] {
        let result = command(&[operation.as_ref(), "--help".as_ref()]);
        assert_success(&result);
        let detail = std::str::from_utf8(&result.stdout).unwrap();
        assert!(
            detail.contains(example) && detail.contains(flag),
            "{detail}"
        );
        assert!(result.stderr.is_empty());
        let subcommand = command(&["help".as_ref(), operation.as_ref()]);
        assert_success(&subcommand);
        assert_eq!(subcommand.stdout, result.stdout);
    }
}

#[test]
fn pack_infers_a_sibling_table_and_refuses_accidental_overwrites() {
    let directory = TempDirectory::new();
    let source = directory.file("input values.txt");
    let inferred = directory.file("input values.npk");
    fs::write(&source, " leading \r\n大阪\r\nfinal\r").unwrap();
    let result = command(&["pack".as_ref(), source.as_os_str()]);
    assert_success(&result);
    assert!(result.stdout.is_empty());
    let status = std::str::from_utf8(&result.stderr).unwrap();
    assert_eq!(status.lines().count(), 1);
    assert!(status.contains("input values.npk") && status.contains("3 values"));
    assert_eq!(
        decoded_values(&inferred),
        BTreeSet::from([" leading ".into(), "大阪".into(), "final\r".into()])
    );

    let original = fs::read(&inferred).unwrap();
    fs::write(&source, "new value\n").unwrap();
    let blocked = command(&["pack".as_ref(), source.as_os_str()]);
    assert!(!blocked.status.success());
    assert!(blocked.stdout.is_empty());
    let diagnostic = std::str::from_utf8(&blocked.stderr).unwrap();
    assert!(diagnostic.contains("input values.npk") && diagnostic.contains("--force"));
    assert_eq!(fs::read(&inferred).unwrap(), original);

    let replaced = command(&["pack".as_ref(), source.as_os_str(), "--force".as_ref()]);
    assert_success(&replaced);
    assert_eq!(
        decoded_values(&inferred),
        BTreeSet::from(["new value".into()])
    );

    let extensionless = directory.file("one-value");
    fs::write(&extensionless, "last record has no delimiter").unwrap();
    assert_success(&command(&["pack".as_ref(), extensionless.as_os_str()]));
    assert!(directory.file("one-value.npk").is_file());
    let explicit = directory.file("my table.bin");
    assert_success(&command(&[
        "pack".as_ref(),
        "--output".as_ref(),
        explicit.as_os_str(),
        extensionless.as_os_str(),
    ]));
    assert_eq!(
        fs::read(&explicit).unwrap(),
        fs::read(directory.file("one-value.npk")).unwrap()
    );

    let same_path = directory.file("compiled-looking.NPK");
    fs::write(&same_path, "please keep me\n").unwrap();
    let result = command(&["pack".as_ref(), same_path.as_os_str(), "--force".as_ref()]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("-o OUTPUT"));
    assert_eq!(fs::read(&same_path).unwrap(), b"please keep me\n");
}

#[test]
fn pack_selects_format_from_the_filename_or_an_explicit_option() {
    let directory = TempDirectory::new();
    let source = directory.file("case.JSONL");
    fs::write(&source, "\"東京\"\r\n\"\\n\\t\\u0000\"").unwrap();
    assert_success(&command(&["pack".as_ref(), source.as_os_str()]));
    let inferred = directory.file("case.npk");
    assert_eq!(
        decoded_values(&inferred),
        BTreeSet::from(["東京".into(), "\n\t\0".into()])
    );

    let plain = directory.file("explicit-lines.npk");
    assert_success(&command(&[
        "pack".as_ref(),
        source.as_os_str(),
        "-o".as_ref(),
        plain.as_os_str(),
        "--format".as_ref(),
        "lines".as_ref(),
    ]));
    let plain_values = BTreeSet::from(["\"東京\"".into(), "\"\\n\\t\\u0000\"".into()]);
    assert_eq!(decoded_values(&plain), plain_values);
    let legacy = directory.file("legacy-lines.npk");
    assert_success(&build(&source, &legacy, &[]));
    assert_eq!(decoded_values(&legacy), plain_values);

    let text = directory.file("explicit.txt");
    fs::write(&text, "\"value\\nwith newline\"\n").unwrap();
    assert_success(&command(&[
        "pack".as_ref(),
        text.as_os_str(),
        "--format".as_ref(),
        "jsonl".as_ref(),
    ]));
    assert_eq!(
        decoded_values(&directory.file("explicit.npk")),
        BTreeSet::from(["value\nwith newline".into()])
    );

    let incorrect = directory.file("plain.jsonl");
    fs::write(&incorrect, "plain value\n").unwrap();
    let rejected = command(&["pack".as_ref(), incorrect.as_os_str()]);
    assert!(!rejected.status.success());
    assert!(rejected.stdout.is_empty());
    let message = String::from_utf8_lossy(&rejected.stderr);
    for detail in ["plain.jsonl", "line 1", "one JSON string", "--format lines"] {
        assert!(message.contains(detail), "{message}");
    }
    assert!(!directory.file("plain.npk").exists());
}

#[test]
fn pick_defaults_to_one_json_string_and_keeps_seeded_selection() {
    let directory = TempDirectory::new();
    let input = directory.file("singleton.jsonl");
    let record = concat!(r#""\"Tokyo\" \\ \n後""#, "\n");
    fs::write(&input, record).unwrap();
    assert_success(&command(&["pack".as_ref(), input.as_os_str()]));
    let table = directory.file("singleton.npk");
    let one = command(&["pick".as_ref(), table.as_os_str()]);
    assert_success(&one);
    assert!(one.stderr.is_empty());
    assert_eq!(one.stdout, record.as_bytes());
    let lines = command(&[
        "pick".as_ref(),
        table.as_os_str(),
        "--format".as_ref(),
        "lines".as_ref(),
    ]);
    assert!(!lines.status.success());
    assert!(lines.stdout.is_empty());
    assert!(String::from_utf8_lossy(&lines.stderr).contains("--format jsonl"));
    let many = command(&[
        "pick".as_ref(),
        "--count".as_ref(),
        "10".as_ref(),
        table.as_os_str(),
        "--seed".as_ref(),
        "42".as_ref(),
    ]);
    assert_success(&many);
    assert_eq!(many.stdout, one.stdout.repeat(10));
    let diagnostics = String::from_utf8_lossy(&many.stderr);
    assert!(diagnostics.contains("with replacement") && diagnostics.contains("seed=42"));

    let frozen = Path::new(env!("CARGO_MANIFEST_DIR")).join("data/given_names_2024.npk");
    let args = [
        "pick".as_ref(),
        frozen.as_os_str(),
        "-n".as_ref(),
        "5".as_ref(),
        "--seed".as_ref(),
        "42".as_ref(),
        "--format".as_ref(),
        "lines".as_ref(),
    ];
    let seeded = command(&args);
    assert_success(&seeded);
    assert_eq!(seeded.stdout, b"Dessa\nXailey\nSameer\nShulem\nRhaelynn\n");
    assert_eq!(command(&args).stdout, seeded.stdout);
    let legacy = command(&[
        "sample".as_ref(),
        frozen.as_os_str(),
        "5".as_ref(),
        "--seed".as_ref(),
        "42".as_ref(),
    ]);
    assert_success(&legacy);
    assert_eq!(legacy.stdout, seeded.stdout);
}

#[test]
fn canonical_pipelines_keep_binary_status_and_value_records_separate() {
    let pack = [
        "pack".as_ref(),
        "-".as_ref(),
        "-o".as_ref(),
        "-".as_ref(),
        "--format".as_ref(),
        "jsonl".as_ref(),
    ];
    let input = concat!(
        r#""\"quotes\" and \\""#,
        "\n",
        r#""\t🐈""#,
        "\n",
        r#""first\nsecond\t\u0000\r""#,
        "\n",
        "\"\"\n",
    );
    let compiled = command_with_input(&pack, input.as_bytes());
    assert_success(&compiled);
    assert!(String::from_utf8_lossy(&compiled.stderr).contains("stdout"));
    assert_eq!(
        decoded_bytes(&compiled.stdout),
        BTreeSet::from([
            "\"quotes\" and \\".into(),
            "\t🐈".into(),
            "first\nsecond\t\0\r".into(),
            "".into(),
        ])
    );
    let unpack = ["unpack".as_ref(), "-".as_ref()];
    let unpacked = command_with_input(&unpack, &compiled.stdout);
    assert_success(&unpacked);
    assert!(unpacked.stderr.is_empty());
    assert_eq!(unpacked.stdout.iter().filter(|&&b| b == b'\n').count(), 4);
    let previous = command_with_input(&["export".as_ref(), "-".as_ref()], &compiled.stdout);
    assert_success(&previous);
    assert_eq!(previous.stdout, unpacked.stdout);
    let rebuilt = command_with_input(&pack, &unpacked.stdout);
    assert_success(&rebuilt);
    assert_eq!(rebuilt.stdout, compiled.stdout);

    let directory = TempDirectory::new();
    let table_path = directory.file("pipeline result.npk");
    let exported_path = directory.file("saved values.jsonl");
    fs::write(&table_path, &compiled.stdout).unwrap();
    let redirected = Command::new(env!("CARGO_BIN_EXE_pkbyte"))
        .arg("unpack")
        .arg(table_path)
        .stdout(fs::File::create(&exported_path).unwrap())
        .output()
        .unwrap();
    assert_success(&redirected);
    assert!(redirected.stderr.is_empty());
    assert_eq!(fs::read(exported_path).unwrap(), unpacked.stdout);

    let lines = command_with_input(
        &[
            "unpack".as_ref(),
            "-".as_ref(),
            "--format".as_ref(),
            "lines".as_ref(),
        ],
        &compiled.stdout,
    );
    assert!(!lines.status.success());
    assert!(lines.stdout.is_empty());
    let issue = String::from_utf8_lossy(&lines.stderr);
    assert!(
        issue.contains("record 1") && issue.contains("--format jsonl"),
        "{issue}"
    );

    let required = command(&["pack".as_ref(), "-".as_ref()]);
    assert!(!required.status.success());
    assert!(String::from_utf8_lossy(&required.stderr).contains("-o OUTPUT"));
    let stdin_default = command_with_input(&pack[..4], "\"literal quotes\"\r\n".as_bytes());
    assert_success(&stdin_default);
    assert_eq!(
        decoded_bytes(&stdin_default.stdout),
        BTreeSet::from(["\"literal quotes\"".into()])
    );
    let singleton_empty = command_with_input(&pack, b"\"\"\n");
    assert_success(&singleton_empty);
    let picked = command_with_input(&["pick".as_ref(), "-".as_ref()], &singleton_empty.stdout);
    assert_success(&picked);
    assert_eq!(picked.stdout, b"\"\"\n");
    for operation in ["info", "verify"] {
        let response = command_with_input(&[operation.as_ref(), "-".as_ref()], &compiled.stdout);
        assert_success(&response);
        assert!(response.stderr.is_empty());
        assert!(String::from_utf8_lossy(&response.stdout).contains("verified"));
    }
}

#[test]
fn canonical_errors_explain_the_input_and_next_step() {
    let directory = TempDirectory::new();
    let missing = directory.file("missing input.txt");
    let result = command(&["pack".as_ref(), missing.as_os_str()]);
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("missing input.txt"));

    let duplicate = directory.file("repeated.jsonl");
    let record = format!("\"value\\n\\u0000{}\"", "long".repeat(300));
    fs::write(&duplicate, format!("{record}\n{record}\n")).unwrap();
    let result = command(&["pack".as_ref(), duplicate.as_os_str()]);
    assert!(!result.status.success());
    let text = std::str::from_utf8(&result.stderr).unwrap();
    assert!(text.lines().count() == 1 && text.len() < 768, "{text}");
    for detail in [
        "repeated.jsonl",
        "duplicate value",
        "value\\n\\0",
        "--deduplicate",
    ] {
        assert!(text.contains(detail), "{text}");
    }
    assert!(!directory.file("repeated.npk").exists());

    let blank = directory.file("blank.txt");
    fs::write(&blank, "one\r\n\r\nlast").unwrap();
    let result = command(&["pack".as_ref(), blank.as_os_str()]);
    assert!(!result.status.success());
    let text = String::from_utf8_lossy(&result.stderr);
    assert!(
        text.contains("line 2") && text.contains("--format jsonl"),
        "{text}"
    );
    assert!(!directory.file("blank.npk").exists());

    let invalid = directory.file("invalid.npk");
    fs::write(&invalid, [0; 64]).unwrap();
    let result = command(&["info".as_ref(), invalid.as_os_str()]);
    assert!(!result.status.success());
    let text = String::from_utf8_lossy(&result.stderr);
    assert!(
        text.contains("invalid.npk") && text.contains("signature") && text.contains("pkbyte pack")
    );

    fs::write(&invalid, b"plain text\n").unwrap();
    let result = command(&["pick".as_ref(), invalid.as_os_str()]);
    assert!(!result.status.success());
    let text = String::from_utf8_lossy(&result.stderr);
    assert!(text.contains("invalid.npk") && text.contains("pkbyte pack"));

    let mut unsupported = pkbyte::build(["value"], &pkbyte::BuildOptions::default())
        .unwrap()
        .bytes;
    unsupported[8..10].copy_from_slice(&u16::MAX.to_le_bytes());
    fs::write(&invalid, unsupported).unwrap();
    let result = command(&["pick".as_ref(), invalid.as_os_str()]);
    assert!(!result.status.success());
    let text = String::from_utf8_lossy(&result.stderr);
    assert!(
        text.contains("invalid.npk")
            && text.contains("65535")
            && text.contains("rebuild from the original values"),
        "{text}"
    );

    let empty = directory.file("empty.txt");
    fs::write(&empty, b"").unwrap();
    assert_success(&command(&["pack".as_ref(), empty.as_os_str()]));
    let empty = directory.file("empty.npk");
    let result = command(&["pick".as_ref(), empty.as_os_str()]);
    assert!(!result.status.success());
    assert!(result.stdout.is_empty());
    let text = String::from_utf8_lossy(&result.stderr);
    assert!(
        text.contains("empty.npk") && text.contains("Pack at least one value"),
        "{text}"
    );
    let zero = command(&[
        "pick".as_ref(),
        empty.as_os_str(),
        "-n".as_ref(),
        "0".as_ref(),
    ]);
    assert_success(&zero);
    assert!(zero.stdout.is_empty() && zero.stderr.is_empty());
}

#[test]
fn cli_build_inspect_verify_and_seeded_sample_work_together() {
    let directory = TempDirectory::new();
    let input = directory.file("values.txt");
    let output = directory.file("values.npk");
    fs::write(&input, "Alice\r\n山田 太郎\r\nÉlodie\r\n leading \r\n").unwrap();
    assert_success(&build(&input, &output, &[]));
    let inspected = command(&["inspect".as_ref(), output.as_os_str()]);
    assert_success(&inspected);
    assert!(String::from_utf8(inspected.stdout)
        .unwrap()
        .contains("Stored values: 4"));
    assert_success(&command(&[
        "verify".as_ref(),
        output.as_os_str(),
        "--unique".as_ref(),
    ]));
    let arguments = [
        "sample".as_ref(),
        output.as_os_str(),
        "20".as_ref(),
        "--seed".as_ref(),
        "42".as_ref(),
    ];
    let first = command(&arguments);
    let second = command(&arguments);
    assert_success(&first);
    assert_success(&second);
    assert_eq!(first.stdout, second.stdout);
    assert!(String::from_utf8_lossy(&first.stderr).contains("noncryptographic"));
    let values = ["Alice", "山田 太郎", "Élodie", " leading "];
    let text = String::from_utf8(first.stdout).unwrap();
    assert_eq!(text.lines().count(), 20);
    assert!(text.lines().all(|value| values.contains(&value)));
}

#[test]
fn cli_requires_explicit_deduplication_for_lines() {
    let directory = TempDirectory::new();
    let input = directory.file("duplicates.txt");
    let output = directory.file("deduplicated.npk");
    fs::write(&input, "Alice\nAlice\n").unwrap();
    assert!(!build(&input, &output, &[]).status.success());
    assert!(!output.exists());
    assert_success(&build(&input, &output, &["--deduplicate", "--raw"]));
    assert_eq!(decoded_values(&output), BTreeSet::from(["Alice".into()]));
}

#[test]
fn cli_handles_empty_input_and_bad_arguments() {
    let directory = TempDirectory::new();
    let input = directory.file("empty.txt");
    let output = directory.file("empty.npk");
    fs::write(&input, b"").unwrap();
    assert_success(&build(&input, &output, &[]));
    assert_success(&command(&[
        "verify".as_ref(),
        output.as_os_str(),
        "--unique".as_ref(),
    ]));
    assert!(!command(&["sample".as_ref(), output.as_os_str()])
        .status
        .success());
    assert!(
        !command(&["sample".as_ref(), output.as_os_str(), "--seed".as_ref()])
            .status
            .success()
    );
    assert!(
        !command(&["sample".as_ref(), output.as_os_str(), "-1".as_ref()])
            .status
            .success()
    );
    assert!(!command(&["unknown".as_ref()]).status.success());
}

#[test]
fn cli_reports_the_crate_version() {
    for flag in ["--version", "-V"] {
        let result = command(&[flag.as_ref()]);
        assert_success(&result);
        assert_eq!(
            result.stdout,
            format!("pkbyte {}\n", env!("CARGO_PKG_VERSION")).as_bytes()
        );
        assert!(result.stderr.is_empty());
    }
}

#[test]
fn cli_rejects_oversized_files_before_loading_their_contents() {
    let directory = TempDirectory::new();
    let input = directory.file("oversized.txt");
    let output = directory.file("unused.npk");
    // A sparse file exercises the metadata check without writing a large fixture.
    let file = fs::File::create(&input).unwrap();
    let limit = pkbyte::BuildOptions::default()
        .max_input_bytes
        .max(pkbyte::Limits::default().max_file_bytes);
    file.set_len(limit as u64 + 1).unwrap();
    let built = build(&input, &output, &[]);
    assert!(!built.status.success());
    assert!(!output.exists());
    assert!(String::from_utf8_lossy(&built.stderr).contains("input limit"));
    let inspected = command(&["inspect".as_ref(), input.as_os_str()]);
    assert!(!inspected.status.success());
    assert!(String::from_utf8_lossy(&inspected.stderr).contains("input limit"));
}

#[test]
fn jsonl_build_and_sample_preserve_exact_values() {
    let directory = TempDirectory::new();
    let input = directory.file("values.jsonl");
    let output = directory.file("values.npk");
    fs::write(
        &input,
        concat!(
            " \t\"\"\r\n",
            "\"   \"\n",
            "\"line\\nfeed\\t\\u0000\\r\"\n",
            "\"\\b\\f\\u001f\\\"\\\\\\/\"\n",
            "\"\\uD83d\\ude00\"\n",
            "\"山田 太郎\"\n",
            "\"Élodie\"\n",
            "\"E\\u0301lodie\"\n",
            "\"élodie\" \t"
        ),
    )
    .unwrap();
    assert_success(&build(&input, &output, &["--jsonl"]));
    let expected = BTreeSet::from([
        "".to_owned(),
        "   ".to_owned(),
        "line\nfeed\t\0\r".to_owned(),
        "\x08\x0c\x1f\"\\/".to_owned(),
        "😀".to_owned(),
        "山田 太郎".to_owned(),
        "Élodie".to_owned(),
        "E\u{301}lodie".to_owned(),
        "élodie".to_owned(),
    ]);
    assert_eq!(decoded_values(&output), expected);

    let sampled = command(&[
        "sample".as_ref(),
        output.as_os_str(),
        "256".as_ref(),
        "--jsonl".as_ref(),
        "--seed".as_ref(),
        "1337".as_ref(),
    ]);
    assert_success(&sampled);
    let text = std::str::from_utf8(&sampled.stdout).unwrap();
    assert_eq!(text.lines().count(), 256);
    assert!(text
        .lines()
        .all(|line| line.starts_with('"') && line.ends_with('"')));
    assert!(text.lines().any(|line| line == r#""line\nfeed\t\u0000\r""#));
    assert!(text.lines().any(|line| line == r#""\b\f\u001f\"\\/""#));
    let sampled_path = directory.file("sampled.jsonl");
    let rebuilt = directory.file("rebuilt.npk");
    fs::write(&sampled_path, sampled.stdout).unwrap();
    assert_success(&build(
        &sampled_path,
        &rebuilt,
        &["--jsonl", "--deduplicate"],
    ));
    assert_eq!(decoded_values(&rebuilt), expected);
}

#[test]
fn jsonl_rejects_malformed_records_at_their_physical_line() {
    let directory = TempDirectory::new();
    let input = directory.file("invalid.jsonl");
    let output = directory.file("unused.npk");
    let malformed: &[&[u8]] = &[
        b"\n",
        b" \t\r\n",
        b"42\n",
        b"null\n",
        b"[\"listed\"]\n",
        b"\"a\" \"b\"\n",
        b"\"unterminated\n",
        b"\"literal\ttab\"\n",
        b"\"literal\0nul\"\n",
        b"\"\\z\"\n",
        b"\"\\u000g\"\n",
        b"\"\\uD800\"\n",
        b"\"\\uD800\\u0041\"\n",
        b"\"\\uDC00\"\n",
        b"\"\xff\"\n",
    ];
    for record in malformed {
        let mut source = b"\"good\"\r\n".to_vec();
        source.extend_from_slice(record);
        fs::write(&input, source).unwrap();
        let result = build(&input, &output, &["--jsonl"]);
        assert!(!result.status.success(), "accepted {record:?}");
        assert!(
            String::from_utf8_lossy(&result.stderr).contains("line 2"),
            "stderr: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(!output.exists());
    }
}

#[test]
fn jsonl_decodes_before_deduplication() {
    let directory = TempDirectory::new();
    let input = directory.file("duplicates.jsonl");
    let output = directory.file("deduplicated.npk");
    fs::write(&input, "\"😀\"\n\"\\ud83d\\ude00\"\n\"\"\n\"\"\n").unwrap();
    let rejected = build(&input, &output, &["--jsonl"]);
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("duplicate value"));
    assert!(!output.exists());
    assert_success(&build(&input, &output, &["--jsonl", "--deduplicate"]));
    assert_eq!(
        decoded_values(&output),
        BTreeSet::from(["".into(), "😀".into()])
    );
}

#[test]
fn plain_sampling_rejects_values_that_would_change_during_framing() {
    let directory = TempDirectory::new();
    let input = directory.file("input.txt");
    for (index, (value, options, json_record)) in [
        ("\"\"\n", &["--jsonl"][..], "\"\"\n"),
        ("\"a\\nb\"\n", &["--jsonl"][..], "\"a\\nb\"\n"),
        ("ends in CR\r", &[][..], "\"ends in CR\\r\"\n"),
    ]
    .into_iter()
    .enumerate()
    {
        let output = directory.file(&format!("single{index}.npk"));
        fs::write(&input, value).unwrap();
        assert_success(&build(&input, &output, options));

        let plain = command(&["sample".as_ref(), output.as_os_str()]);
        assert!(!plain.status.success());
        assert!(plain.stdout.is_empty());
        assert!(String::from_utf8_lossy(&plain.stderr).contains("--jsonl"));

        let json = command(&["sample".as_ref(), output.as_os_str(), "--jsonl".as_ref()]);
        assert_success(&json);
        assert_eq!(json.stdout, json_record.as_bytes());
    }
}

#[test]
fn max_value_bytes_applies_to_decoded_values_and_opening() {
    let directory = TempDirectory::new();
    let input = directory.file("values.jsonl");
    let output = directory.file("values.npk");
    fs::write(&input, "\"\\ud83d\\ude00\"\n").unwrap();
    let rejected = build(&input, &output, &["--jsonl", "--max-value-bytes", "3"]);
    assert!(!rejected.status.success());
    let stderr = String::from_utf8_lossy(&rejected.stderr);
    assert!(stderr.contains("line 1") && stderr.contains("decoded value"));
    assert!(!output.exists());

    let long_value = "あ".repeat(1400);
    fs::write(&input, format!("\"{long_value}\"\n")).unwrap();
    assert!(!build(&input, &output, &["--jsonl"]).status.success());
    assert_success(&build(
        &input,
        &output,
        &["--jsonl", "--max-value-bytes", "4200"],
    ));
    for operation in ["inspect", "verify", "sample", "export"] {
        assert!(!command(&[operation.as_ref(), output.as_os_str()])
            .status
            .success());
        let with_limit = command(&[
            operation.as_ref(),
            output.as_os_str(),
            "--max-value-bytes".as_ref(),
            "4200".as_ref(),
        ]);
        assert_success(&with_limit);
        if operation == "sample" {
            assert_eq!(with_limit.stdout, format!("{long_value}\n").as_bytes());
        }
    }
}

#[test]
fn cli_streams_binary_and_round_trips_exact_jsonl() {
    let args = [
        "build".as_ref(),
        "-".as_ref(),
        "-".as_ref(),
        "--jsonl".as_ref(),
    ];
    let packed = command_with_input(
        &args,
        "\"\"\n\"\\n\\t\\u0000\"\n\"  \"\n\"東京😀\"\n\"tail\\r\"\n".as_bytes(),
    );
    assert_success(&packed);
    assert!(String::from_utf8_lossy(&packed.stderr).contains("Stored values: 5"));
    assert_eq!(
        decoded_bytes(&packed.stdout),
        BTreeSet::from([
            "".into(),
            "\n\t\0".into(),
            "  ".into(),
            "東京😀".into(),
            "tail\r".into()
        ]),
    );

    let exported = command_with_input(&["export".as_ref(), "-".as_ref()], &packed.stdout);
    assert_success(&exported);
    assert!(exported.stderr.is_empty());
    let table = pkbyte::Table::open(&packed.stdout).unwrap();
    assert_eq!(
        exported
            .stdout
            .iter()
            .filter(|&&byte| byte == b'\n')
            .count(),
        table.len() as usize
    );
    let rebuilt = command_with_input(&args, &exported.stdout);
    assert_success(&rebuilt);
    assert_eq!(rebuilt.stdout, packed.stdout);
    let plain = command_with_input(
        &["export".as_ref(), "-".as_ref(), "--plain".as_ref()],
        &packed.stdout,
    );
    assert!(!plain.status.success());
    assert!(plain.stdout.is_empty());
    assert!(String::from_utf8_lossy(&plain.stderr).contains("--jsonl"));

    for operation in ["inspect", "verify", "sample"] {
        let mut flags = vec![operation.as_ref(), "-".as_ref()];
        if operation == "sample" {
            flags.push("--jsonl".as_ref());
        }
        assert_success(&command_with_input(&flags, &packed.stdout));
    }
}

#[test]
fn cli_pipelines_work_with_spaced_paths() {
    let directory = TempDirectory::new();
    let input = directory.file("input values.txt");
    let output = directory.file("packed values.npk");
    fs::write(&input, "  first  \n東京\t語\n").unwrap();
    let result = build(&input, &output, &["--raw"]);
    assert_success(&result);
    assert!(result.stdout.is_empty());
    let mut producer = Command::new(env!("CARGO_BIN_EXE_pkbyte"))
        .args([
            "build".as_ref(),
            input.as_os_str(),
            "-".as_ref(),
            "--raw".as_ref(),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let exported = Command::new(env!("CARGO_BIN_EXE_pkbyte"))
        .args(["export", "-", "--plain"])
        .stdin(producer.stdout.take().unwrap())
        .output()
        .unwrap();
    assert_success(&producer.wait_with_output().unwrap());
    assert_success(&exported);
    let standalone = command(&["export".as_ref(), output.as_os_str(), "--plain".as_ref()]);
    assert_success(&standalone);
    assert_eq!(exported.stdout, standalone.stdout);
    assert_eq!(
        std::str::from_utf8(&exported.stdout)
            .unwrap()
            .lines()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["  first  ", "東京\t語"]),
    );
}

#[test]
fn cli_force_replaces_only_after_success() {
    let directory = TempDirectory::new();
    let input = directory.file("source.jsonl");
    let output = directory.file("existing result.npk");
    fs::write(&output, b"original bytes").unwrap();
    fs::write(&input, b"\"unfinished\n").unwrap();
    let rejected = build(&input, &output, &["--force", "--jsonl"]);
    assert!(!rejected.status.success());
    assert_eq!(fs::read(&output).unwrap(), b"original bytes");

    fs::write(&input, b"\"replacement\"\n").unwrap();
    let exists = build(&input, &output, &["--jsonl"]);
    assert!(!exists.status.success());
    assert!(String::from_utf8_lossy(&exists.stderr).contains("--force"));
    assert_eq!(fs::read(&output).unwrap(), b"original bytes");

    let replaced = build(&input, &output, &["--force", "--jsonl"]);
    assert_success(&replaced);
    assert!(replaced.stdout.is_empty());
    assert_eq!(
        decoded_values(&output),
        BTreeSet::from(["replacement".into()])
    );
    assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 2);

    let blocked = directory.file("nonempty directory");
    fs::create_dir(&blocked).unwrap();
    fs::write(blocked.join("keep.txt"), b"preserve").unwrap();
    let failed_publication = build(&input, &blocked, &["--force", "--jsonl"]);
    assert!(!failed_publication.status.success());
    assert!(String::from_utf8_lossy(&failed_publication.stderr).contains("nonempty directory"));
    assert_eq!(fs::read(blocked.join("keep.txt")).unwrap(), b"preserve");
    assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 3);

    let invalid_force = command(&[
        "build".as_ref(),
        input.as_os_str(),
        "-".as_ref(),
        "--force".as_ref(),
    ]);
    assert!(!invalid_force.status.success());
    assert!(invalid_force.stdout.is_empty());
    assert!(String::from_utf8_lossy(&invalid_force.stderr).contains("--force"));
}

#[test]
fn cli_limits_apply_to_files_and_streams() {
    for (option, expected) in [
        ("--max-input-bytes", "input limit"),
        ("--max-input-rows", "row"),
        ("--max-value-bytes", "value"),
    ] {
        let args = [
            "build".as_ref(),
            "-".as_ref(),
            "-".as_ref(),
            option.as_ref(),
            "1".as_ref(),
        ];
        let limited = command_with_input(&args, b"ab\nx\n");
        assert!(!limited.status.success(), "{option}");
        assert!(limited.stdout.is_empty());
        assert!(String::from_utf8_lossy(&limited.stderr).contains(expected));
    }
    let args = [
        "build".as_ref(),
        "-".as_ref(),
        "-".as_ref(),
        "--raw".as_ref(),
        "--max-input-bytes".as_ref(),
        "5".as_ref(),
        "--max-input-rows".as_ref(),
        "2".as_ref(),
        "--max-value-bytes".as_ref(),
        "2".as_ref(),
    ];
    let packed = command_with_input(&args, b"ab\nx\n");
    assert_success(&packed);
    let directory = TempDirectory::new();
    let file = directory.file("two.npk");
    fs::write(&file, &packed.stdout).unwrap();
    for option in [
        "--max-file-bytes",
        "--max-rows",
        "--max-decoded-bytes",
        "--max-value-bytes",
        "--max-groups",
    ] {
        let stream = command_with_input(
            &[
                "export".as_ref(),
                "-".as_ref(),
                option.as_ref(),
                "1".as_ref(),
            ],
            &packed.stdout,
        );
        assert!(!stream.status.success(), "{option}");
        assert!(stream.stdout.is_empty());
        let path = command(&[
            "inspect".as_ref(),
            file.as_os_str(),
            option.as_ref(),
            "1".as_ref(),
        ]);
        assert!(!path.status.success(), "{option}");
        assert!(path.stdout.is_empty());
    }
}

#[test]
fn cli_verifies_uniqueness_and_samples_zero_without_output() {
    let mut packed = pkbyte::build(
        ["aa", "bb"],
        &pkbyte::BuildOptions {
            mode: pkbyte::BuildMode::Raw,
            ..pkbyte::BuildOptions::default()
        },
    )
    .unwrap()
    .bytes;
    let record_start = packed.len() - 2;
    packed[record_start..].copy_from_slice(b"aa");
    refresh_crc(&mut packed);
    assert!(pkbyte::Table::open(&packed).is_ok());

    for operation in ["verify", "sample", "export"] {
        let result = command_with_input(&[operation.as_ref(), "-".as_ref()], &packed);
        assert!(!result.status.success(), "{operation}");
        assert!(result.stdout.is_empty());
        assert!(String::from_utf8_lossy(&result.stderr).contains("duplicate value"));
    }

    let zero_args = ["sample".as_ref(), "-".as_ref(), "0".as_ref()];
    let zero = command_with_input(&zero_args, &packed);
    assert_success(&zero);
    assert!(zero.stdout.is_empty());
    assert!(zero.stderr.is_empty());
    packed[record_start] = b'c';
    let corrupted = command_with_input(&zero_args, &packed);
    assert!(!corrupted.status.success());
    assert!(String::from_utf8_lossy(&corrupted.stderr).contains("CRC-32 mismatch"));
}

#[test]
fn cli_treats_a_closed_stdout_pipe_as_success() {
    let directory = TempDirectory::new();
    let input = directory.file("source.txt");
    let output = directory.file("table.npk");
    fs::write(&input, b"abc\n").unwrap();
    assert_success(&build(&input, &output, &[]));
    for args in [
        vec!["build".as_ref(), input.as_os_str(), "-".as_ref()],
        vec!["inspect".as_ref(), output.as_os_str()],
        vec!["verify".as_ref(), output.as_os_str()],
        vec!["export".as_ref(), output.as_os_str()],
        vec!["sample".as_ref(), output.as_os_str(), "10000".as_ref()],
    ] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_pkbyte"))
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        drop(child.stdout.take().unwrap());
        let result = child.wait_with_output().unwrap();
        assert_success(&result);
        assert!(!String::from_utf8_lossy(&result.stderr).contains("panicked"));
    }
}

#[test]
fn cli_can_write_binary_with_stderr_closed() {
    let directory = TempDirectory::new();
    let input = directory.file("source.txt");
    fs::write(&input, b"alpha\n").unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_pkbyte"))
        .args(["build".as_ref(), input.as_os_str(), "-".as_ref()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stderr.take().unwrap());
    let result = child.wait_with_output().unwrap();
    assert_success(&result);
    assert_eq!(
        decoded_bytes(&result.stdout),
        BTreeSet::from(["alpha".into()])
    );
}
