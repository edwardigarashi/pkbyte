use pkbyte::{Picker, PrepareOptions, Table};
use std::collections::BTreeSet;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::{self, Command, Output};
use std::thread;
use std::time::{Duration, Instant};

const EXAMPLE_MANIFEST: &str = include_str!("../examples/generated/Cargo.toml.in");
const EXAMPLE_LOCKFILE: &str = include_str!("../examples/generated/Cargo.lock.in");
const EXAMPLE_BUILD: &str = include_str!("../examples/generated/build.rs");
const EXAMPLE_MAIN: &str = include_str!("../examples/generated/src/main.rs");
const EXAMPLE_DATASET: &str = include_str!("../examples/generated/values.jsonl");
const EXPECTED: &[&str] = &[
    "Tokyo",
    "東京都",
    "",
    "first line\nsecond line",
    "A\0B",
    "  preserve spaces  ",
    "👩🏽‍💻",
    "ends with CR\r",
    r#""; include!("assets\table.npk"); // Rust-looking text"#,
];

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new() -> Self {
        for serial in 0.. {
            let path = std::env::temp_dir().join(format!(
                "pkbyte-build-integration-{}-{serial}",
                process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create test directory: {error}"),
            }
        }
        unreachable!()
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Consumer {
    root: PathBuf,
    target: PathBuf,
}

impl Consumer {
    fn copy_to(directory: &Path, name: &str) -> Self {
        let root = directory.join(name);
        let target = root.join("target");
        fs::create_dir_all(root.join("src")).unwrap();
        let repository = env!("CARGO_MANIFEST_DIR")
            .replace('\\', "\\\\")
            .replace('"', "\\\"");
        let manifest =
            EXAMPLE_MANIFEST.replace(r#"path = "../..""#, &format!(r#"path = "{repository}""#));
        assert_ne!(manifest, EXAMPLE_MANIFEST);
        fs::write(root.join("Cargo.toml"), manifest).unwrap();
        fs::write(root.join("Cargo.lock"), EXAMPLE_LOCKFILE).unwrap();
        fs::write(root.join("build.rs"), EXAMPLE_BUILD).unwrap();
        fs::write(root.join("src/main.rs"), EXAMPLE_MAIN).unwrap();
        fs::write(root.join("values.jsonl"), EXAMPLE_DATASET).unwrap();
        Self { root, target }
    }

    fn run(&self) -> Vec<u8> {
        successful_stdout(self.run_cargo())
    }

    fn run_cargo(&self) -> Output {
        Command::new(env!("CARGO"))
            .arg("run")
            .args(["--locked", "--offline"])
            .current_dir(&self.root)
            .env("CARGO_TARGET_DIR", &self.target)
            .output()
            .unwrap()
    }

    fn generated_directory(&self) -> PathBuf {
        // Cargo controls the intermediate directory layout. Find the files
        // in this consumer's isolated target directory.
        let mut directories = vec![self.target.clone()];
        let mut outputs = Vec::new();
        while let Some(directory) = directories.pop() {
            if directory.join("pkbyte_table.rs").is_file()
                && directory.join("pkbyte_table.npk").is_file()
            {
                outputs.push(directory);
            } else {
                for entry in fs::read_dir(directory).unwrap() {
                    let entry = entry.unwrap();
                    if entry.file_type().unwrap().is_dir() {
                        directories.push(entry.path());
                    }
                }
            }
        }
        assert_eq!(outputs.len(), 1, "generated table directories: {outputs:?}");
        outputs.pop().unwrap()
    }
}

fn successful_stdout(output: Output) -> Vec<u8> {
    assert!(
        output.status.success(),
        "process failed with {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    output.stdout
}

fn assert_exact_values(bytes: &[u8], expected: &[&str]) {
    let table = Table::open(bytes).unwrap();
    let report = table.validate_all(true).unwrap();
    assert_eq!(report.row_count, expected.len() as u64);
    let prepared = table.prepare(&PrepareOptions::default()).unwrap();
    let mut scratch = prepared.new_output_buffer().unwrap();
    let mut values = BTreeSet::new();
    for rank in 0..table.len() {
        let value = prepared.get_into(rank, &mut scratch).unwrap();
        assert_eq!(value, table.get(rank).unwrap());
        assert!(values.insert(value.to_owned()));
    }
    assert_eq!(
        values,
        expected.iter().map(|value| (*value).to_owned()).collect()
    );
}

fn rewrite_input(path: &Path, contents: &str, generated: &Path) {
    let previous_build = fs::metadata(generated).unwrap().modified().unwrap();
    let start = Instant::now();
    loop {
        fs::write(path, contents).unwrap();
        if fs::metadata(path).unwrap().modified().unwrap() > previous_build {
            return;
        }
        // A coarser timestamp clock can initially record the edit and the
        // preceding build at the same time.
        assert!(start.elapsed() < Duration::from_secs(4));
        thread::sleep(Duration::from_millis(25));
    }
}

#[test]
fn cargo_generates_reproducible_tables_and_rebuilds_after_a_dataset_edit() {
    let directory = TempDirectory::new();
    let first = Consumer::copy_to(&directory.0, "first consumer");
    let second = Consumer::copy_to(&directory.0, "second consumer");
    let original_samples = first.run();
    assert_eq!(
        String::from_utf8_lossy(&original_samples).lines().count(),
        8
    );
    let first_out = first.generated_directory();
    let first_table = first_out.join("pkbyte_table.npk");
    let first_module = first_out.join("pkbyte_table.rs");
    let original_bytes = fs::read(&first_table).unwrap();
    let original_module = fs::read_to_string(&first_module).unwrap();
    assert!(original_module.contains("include_bytes!"));
    assert!(original_module.contains(r#""pkbyte_table.npk""#));
    assert_exact_values(&original_bytes, EXPECTED);

    assert_eq!(second.run(), original_samples);
    let second_out = second.generated_directory();
    assert_ne!(first_out, second_out);
    assert_eq!(
        fs::read(second_out.join("pkbyte_table.npk")).unwrap(),
        original_bytes
    );
    assert_eq!(
        fs::read_to_string(second_out.join("pkbyte_table.rs")).unwrap(),
        original_module
    );

    let external = directory.0.join("external.npk");
    let output = Command::new(env!("CARGO_BIN_EXE_pkbyte"))
        .arg("build")
        .arg(first.root.join("values.jsonl"))
        .arg(&external)
        .arg("--jsonl")
        .output()
        .unwrap();
    successful_stdout(output);
    assert_eq!(fs::read(&external).unwrap(), original_bytes);

    let mut picker = Picker::open(&external).unwrap();
    fs::copy(&external, second.root.join("precompiled.npk")).unwrap();
    fs::remove_file(&external).unwrap();
    let mut scratch = Vec::new();
    for _ in 0..64 {
        assert!(EXPECTED.contains(&picker.pick_into(&mut scratch).unwrap()));
    }
    let precompiled_main = EXAMPLE_MAIN.replace(
        r#"include!(concat!(env!("OUT_DIR"), "/pkbyte_table.rs"));"#,
        r#"pub static TABLE_BYTES: &[u8] = include_bytes!("../precompiled.npk");"#,
    );
    assert_ne!(precompiled_main, EXAMPLE_MAIN);
    fs::write(second.root.join("src/main.rs"), precompiled_main).unwrap();
    assert_eq!(second.run(), original_samples);

    let first_input = first.root.join("values.jsonl");
    rewrite_input(&first_input, "\"regenerated\\nvalue\"\n", &first_table);
    let regenerated_samples = first.run();
    assert_ne!(regenerated_samples, original_samples);
    assert_eq!(
        regenerated_samples,
        "\"regenerated\\nvalue\"\n".repeat(8).as_bytes()
    );
    let regenerated_bytes = fs::read(&first_table).unwrap();
    assert_ne!(regenerated_bytes, original_bytes);
    assert_exact_values(&regenerated_bytes, &["regenerated\nvalue"]);
    assert_eq!(fs::read_to_string(&first_module).unwrap(), original_module);

    rewrite_input(&first_input, "\"valid\"\n{}\n", &first_table);
    let failed = first.run_cargo();
    assert!(!failed.status.success());
    let stderr = String::from_utf8_lossy(&failed.stderr);
    assert!(stderr.contains("expected one JSON string"), "{stderr}");
    assert_eq!(fs::read(&first_table).unwrap(), regenerated_bytes);
    assert_eq!(fs::read_to_string(&first_module).unwrap(), original_module);
}
