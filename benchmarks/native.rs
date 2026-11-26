//! Optional Linux benchmark executable. Unsafe code is confined to small FFI
//! adapters for system SQLite, JSON-C and zlib; it is not part of the library.
use pkbyte::{
    build, uniform_below, BuildMode, BuildOptions, PrepareOptions, SamplingMethod, Table,
};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{CStr, CString};
use std::fmt::Write as _;
use std::fs;
use std::hint::black_box;
use std::io;
use std::os::raw::{c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::ptr;
use std::time::Instant;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
fn error(message: impl Into<String>) -> Box<dyn std::error::Error> {
    io::Error::other(message.into()).into()
}

#[repr(C)]
struct Sqlite3 {
    _private: [u8; 0],
}
#[repr(C)]
struct Sqlite3Stmt {
    _private: [u8; 0],
}
#[repr(C)]
struct JsonObject {
    _private: [u8; 0],
}
#[repr(C)]
struct JsonTokener {
    _private: [u8; 0],
}

extern "C" {
    fn sqlite3_open_v2(
        path: *const c_char,
        db: *mut *mut Sqlite3,
        flags: c_int,
        vfs: *const c_char,
    ) -> c_int;
    fn sqlite3_close(db: *mut Sqlite3) -> c_int;
    fn sqlite3_errmsg(db: *mut Sqlite3) -> *const c_char;
    fn sqlite3_libversion() -> *const c_char;
    fn sqlite3_exec(
        db: *mut Sqlite3,
        sql: *const c_char,
        callback: Option<
            unsafe extern "C" fn(*mut c_void, c_int, *mut *mut c_char, *mut *mut c_char) -> c_int,
        >,
        arg: *mut c_void,
        message: *mut *mut c_char,
    ) -> c_int;
    fn sqlite3_free(value: *mut c_void);
    fn sqlite3_prepare_v2(
        db: *mut Sqlite3,
        sql: *const c_char,
        len: c_int,
        statement: *mut *mut Sqlite3Stmt,
        tail: *mut *const c_char,
    ) -> c_int;
    fn sqlite3_finalize(statement: *mut Sqlite3Stmt) -> c_int;
    fn sqlite3_bind_int64(statement: *mut Sqlite3Stmt, index: c_int, value: i64) -> c_int;
    fn sqlite3_bind_text(
        statement: *mut Sqlite3Stmt,
        index: c_int,
        value: *const c_char,
        length: c_int,
        destructor: Option<unsafe extern "C" fn(*mut c_void)>,
    ) -> c_int;
    fn sqlite3_step(statement: *mut Sqlite3Stmt) -> c_int;
    fn sqlite3_reset(statement: *mut Sqlite3Stmt) -> c_int;
    fn sqlite3_column_text(statement: *mut Sqlite3Stmt, column: c_int) -> *const u8;
    fn sqlite3_column_bytes(statement: *mut Sqlite3Stmt, column: c_int) -> c_int;

    fn json_tokener_new() -> *mut JsonTokener;
    fn json_tokener_free(tokener: *mut JsonTokener);
    fn json_tokener_parse_ex(
        tokener: *mut JsonTokener,
        text: *const c_char,
        length: c_int,
    ) -> *mut JsonObject;
    fn json_tokener_get_error(tokener: *mut JsonTokener) -> c_int;
    fn json_object_put(object: *mut JsonObject) -> c_int;
    fn json_object_get_type(object: *const JsonObject) -> c_int;
    fn json_object_array_length(object: *const JsonObject) -> usize;
    fn json_object_array_get_idx(object: *const JsonObject, index: usize) -> *mut JsonObject;
    fn json_object_get_string(object: *mut JsonObject) -> *const c_char;
    fn json_object_get_string_len(object: *const JsonObject) -> c_int;
    fn json_c_version() -> *const c_char;

    fn gzopen(path: *const c_char, mode: *const c_char) -> *mut c_void;
    fn gzwrite(file: *mut c_void, buffer: *const c_void, length: u32) -> c_int;
    fn gzread(file: *mut c_void, buffer: *mut c_void, length: u32) -> c_int;
    fn gzclose(file: *mut c_void) -> c_int;
    fn zlibVersion() -> *const c_char;
}

struct Connection(*mut Sqlite3);
impl Connection {
    fn open(path: &Path, create: bool) -> Result<Self> {
        let path = CString::new(path.to_str().ok_or_else(|| error("Non-UTF-8 path"))?)?;
        let mut db = ptr::null_mut();
        // This benchmark is single-threaded. NOMUTEX removes unnecessary
        // connection mutex overhead; the comparison is still a real SQL query.
        let flags = if create { 2 | 4 | 0x8000 } else { 1 | 0x8000 };
        let status = unsafe { sqlite3_open_v2(path.as_ptr(), &mut db, flags, ptr::null()) };
        if status != 0 {
            let message = if db.is_null() {
                "SQLite open failed".into()
            } else {
                unsafe {
                    CStr::from_ptr(sqlite3_errmsg(db))
                        .to_string_lossy()
                        .into_owned()
                }
            };
            if !db.is_null() {
                unsafe {
                    sqlite3_close(db);
                }
            }
            return Err(error(message));
        }
        Ok(Self(db))
    }
    fn check(&self, code: c_int) -> Result<()> {
        if code == 0 {
            return Ok(());
        }
        Err(error(unsafe {
            CStr::from_ptr(sqlite3_errmsg(self.0)).to_string_lossy()
        }))
    }
    fn execute(&self, sql: &str) -> Result<()> {
        let sql = CString::new(sql)?;
        let mut message = ptr::null_mut();
        let status =
            unsafe { sqlite3_exec(self.0, sql.as_ptr(), None, ptr::null_mut(), &mut message) };
        if status == 0 {
            return Ok(());
        }
        let detail = if message.is_null() {
            "SQLite execute failed".into()
        } else {
            let detail = unsafe { CStr::from_ptr(message).to_string_lossy().into_owned() };
            unsafe {
                sqlite3_free(message.cast());
            }
            detail
        };
        Err(error(detail))
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        unsafe {
            sqlite3_close(self.0);
        }
    }
}

struct Statement<'a> {
    pointer: *mut Sqlite3Stmt,
    connection: &'a Connection,
}
impl<'a> Statement<'a> {
    fn prepare(connection: &'a Connection, sql: &str) -> Result<Self> {
        let sql = CString::new(sql)?;
        let mut pointer = ptr::null_mut();
        connection.check(unsafe {
            sqlite3_prepare_v2(
                connection.0,
                sql.as_ptr(),
                -1,
                &mut pointer,
                ptr::null_mut(),
            )
        })?;
        if pointer.is_null() {
            return Err(error("SQLite returned no prepared statement"));
        }
        Ok(Self {
            pointer,
            connection,
        })
    }
    fn read_into(&mut self, rank: u64, out: &mut Vec<u8>) -> Result<()> {
        let rank = i64::try_from(rank)?;
        self.connection
            .check(unsafe { sqlite3_bind_int64(self.pointer, 1, rank) })?;
        if unsafe { sqlite3_step(self.pointer) } != 100 {
            return Err(error("SQLite lookup did not return a row"));
        }
        let bytes = unsafe { sqlite3_column_text(self.pointer, 0) };
        let length = unsafe { sqlite3_column_bytes(self.pointer, 0) };
        if bytes.is_null() || length < 0 {
            return Err(error("Invalid SQLite text result"));
        }
        out.clear();
        // The result pointer remains valid until reset. Copy before resetting.
        out.extend_from_slice(unsafe { std::slice::from_raw_parts(bytes, length as usize) });
        self.connection
            .check(unsafe { sqlite3_reset(self.pointer) })
    }
}
impl Drop for Statement<'_> {
    fn drop(&mut self) {
        unsafe {
            sqlite3_finalize(self.pointer);
        }
    }
}

fn create_sqlite(path: &Path, names: &[Vec<u8>]) -> Result<()> {
    if path.exists() {
        fs::remove_file(path)?;
    }
    let db = Connection::open(path, true)?;
    db.execute(
        "PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF; BEGIN; \
        CREATE TABLE names (id INTEGER PRIMARY KEY, name TEXT NOT NULL);",
    )?;
    {
        let statement = Statement::prepare(&db, "INSERT INTO names VALUES (?1, ?2)")?;
        for (rank, name) in names.iter().enumerate() {
            db.check(unsafe { sqlite3_bind_int64(statement.pointer, 1, rank as i64) })?;
            // SQLITE_STATIC is safe here: all source buffers live until this
            // statement is finalized, and explicit lengths permit non-NUL data.
            db.check(unsafe {
                sqlite3_bind_text(
                    statement.pointer,
                    2,
                    name.as_ptr().cast(),
                    c_int::try_from(name.len())?,
                    None,
                )
            })?;
            if unsafe { sqlite3_step(statement.pointer) } != 101 {
                return Err(error("SQLite insertion failed"));
            }
            db.check(unsafe { sqlite3_reset(statement.pointer) })?;
        }
    }
    db.execute("COMMIT")
}

struct JsonDoc(*mut JsonObject);
impl JsonDoc {
    fn parse(bytes: &[u8]) -> Result<Self> {
        let length = c_int::try_from(bytes.len())?;
        let tokener = unsafe { json_tokener_new() };
        if tokener.is_null() {
            return Err(error("JSON-C allocation failed"));
        }
        let object = unsafe { json_tokener_parse_ex(tokener, bytes.as_ptr().cast(), length) };
        let status = unsafe { json_tokener_get_error(tokener) };
        unsafe {
            json_tokener_free(tokener);
        }
        if status != 0 || object.is_null() {
            if !object.is_null() {
                unsafe {
                    json_object_put(object);
                }
            }
            return Err(error(format!("JSON-C parse failed with error {status}")));
        }
        let document = Self(object);
        if unsafe { json_object_get_type(document.0) } != 5 {
            return Err(error("JSON fixture must be an array"));
        }
        Ok(document)
    }
    fn read_into(&self, rank: u64, out: &mut Vec<u8>) -> Result<()> {
        let rank = usize::try_from(rank)?;
        if rank >= unsafe { json_object_array_length(self.0) } {
            return Err(error("JSON rank outside array"));
        }
        let item = unsafe { json_object_array_get_idx(self.0, rank) };
        if item.is_null() || unsafe { json_object_get_type(item) } != 6 {
            return Err(error("JSON item is not a string"));
        }
        let pointer = unsafe { json_object_get_string(item) };
        let length = unsafe { json_object_get_string_len(item) };
        if pointer.is_null() || length < 0 {
            return Err(error("Invalid JSON string"));
        }
        out.clear();
        out.extend_from_slice(unsafe {
            std::slice::from_raw_parts(pointer.cast::<u8>(), length as usize)
        });
        Ok(())
    }
}
impl Drop for JsonDoc {
    fn drop(&mut self) {
        unsafe {
            json_object_put(self.0);
        }
    }
}

struct Gzip(*mut c_void);
impl Gzip {
    fn open(path: &Path, mode: &str) -> Result<Self> {
        let path = CString::new(path.to_str().ok_or_else(|| error("Non-UTF-8 path"))?)?;
        let mode = CString::new(mode)?;
        let pointer = unsafe { gzopen(path.as_ptr(), mode.as_ptr()) };
        if pointer.is_null() {
            return Err(error("Could not open gzip fixture"));
        }
        Ok(Self(pointer))
    }
    fn finish(mut self) -> Result<()> {
        let status = unsafe { gzclose(self.0) };
        self.0 = ptr::null_mut();
        if status != 0 {
            return Err(error(format!("gzip close failed: {status}")));
        }
        Ok(())
    }
}
impl Drop for Gzip {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                gzclose(self.0);
            }
        }
    }
}
fn write_gzip(path: &Path, bytes: &[u8]) -> Result<()> {
    let file = Gzip::open(path, "wb9")?;
    for chunk in bytes.chunks(1 << 20) {
        if unsafe { gzwrite(file.0, chunk.as_ptr().cast(), chunk.len() as u32) }
            != chunk.len() as c_int
        {
            return Err(error("gzip write failed"));
        }
    }
    file.finish()
}
fn read_gzip(path: &Path) -> Result<Vec<u8>> {
    let file = Gzip::open(path, "rb")?;
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 65536];
    loop {
        let size = unsafe { gzread(file.0, buffer.as_mut_ptr().cast(), buffer.len() as u32) };
        if size < 0 {
            return Err(error("gzip read failed"));
        }
        if size == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..size as usize]);
    }
    file.finish()?;
    Ok(bytes)
}

fn json_string(value: &str) -> String {
    let mut result = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            ch if ch < '\u{20}' => {
                write!(result, "\\u{:04x}", ch as u32).unwrap();
            }
            ch => result.push(ch),
        }
    }
    result.push('"');
    result
}

const HASH_START: u64 = 0xcbf29ce484222325;
const SAMPLING_SEED: u64 = 0x6e616d657061636b;
fn consume(mut checksum: u64, bytes: &[u8]) -> u64 {
    checksum = (checksum ^ bytes.len() as u64).wrapping_mul(0x100000001b3);
    for &byte in bytes {
        checksum = (checksum ^ byte as u64).wrapping_mul(0x100000001b3);
    }
    checksum
}

fn consume_checked(checksum: u64, bytes: &[u8]) -> Result<u64> {
    std::str::from_utf8(bytes)?;
    Ok(consume(checksum, bytes))
}
fn next_u64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e3779b97f4a7c15);
    let mut x = *state;
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d049bb133111eb);
    x ^ (x >> 31)
}
fn ranks(count: usize, n: u64) -> Vec<u64> {
    let mut state = SAMPLING_SEED;
    let reject = n.wrapping_neg() % n;
    (0..count)
        .map(|_| loop {
            let random = next_u64(&mut state);
            if random >= reject {
                break random % n;
            }
        })
        .collect()
}

struct Paths {
    compact: PathBuf,
    raw: PathBuf,
    json: PathBuf,
    gzip: PathBuf,
    sqlite: PathBuf,
}
impl Paths {
    fn new(root: &Path) -> Self {
        Self {
            compact: root.join("names.compact.npk"),
            raw: root.join("names.raw.npk"),
            json: root.join("names.json"),
            gzip: root.join("names.txt.gz"),
            sqlite: root.join("names.sqlite"),
        }
    }
}

fn startup(kind: &str, paths: &Paths, rank: u64, max_length: usize) -> Result<(u128, u64)> {
    let mut output = Vec::with_capacity(max_length);
    let timer = Instant::now();
    // The timer stops before the reader/table/object graph is dropped. This
    // measures time to the first consumed result, not teardown throughput.
    match kind {
        "compact_crc_open" | "compact_structural_open" | "raw_crc_open" => {
            let bytes = fs::read(if kind == "raw_crc_open" {
                &paths.raw
            } else {
                &paths.compact
            })?;
            let table = if kind == "compact_structural_open" {
                Table::open_structural(&bytes)?
            } else {
                Table::open(&bytes)?
            };
            table.get_into(rank, &mut output)?;
            let checksum = consume(HASH_START, &output);
            let elapsed = timer.elapsed().as_nanos();
            black_box(&output);
            Ok((elapsed, checksum))
        }
        "sqlite_open_prepare_lookup" => {
            let db = Connection::open(&paths.sqlite, false)?;
            let mut query = Statement::prepare(&db, "SELECT name FROM names WHERE id = ?1")?;
            query.read_into(rank, &mut output)?;
            let checksum = consume_checked(HASH_START, &output)?;
            let elapsed = timer.elapsed().as_nanos();
            black_box(&output);
            Ok((elapsed, checksum))
        }
        "json_c_read_parse_lookup" => {
            let bytes = fs::read(&paths.json)?;
            let document = JsonDoc::parse(&bytes)?;
            document.read_into(rank, &mut output)?;
            let checksum = consume_checked(HASH_START, &output)?;
            let elapsed = timer.elapsed().as_nanos();
            black_box(&output);
            Ok((elapsed, checksum))
        }
        "gzip_text_read_decode_parse_lookup" => {
            let bytes = read_gzip(&paths.gzip)?;
            let text = std::str::from_utf8(&bytes)?;
            let names: Vec<Vec<u8>> = text
                .split_terminator('\n')
                .map(|name| name.as_bytes().to_vec())
                .collect();
            output.extend_from_slice(
                names
                    .get(rank as usize)
                    .ok_or_else(|| error("gzip rank out of bounds"))?,
            );
            let checksum = consume_checked(HASH_START, &output)?;
            let elapsed = timer.elapsed().as_nanos();
            black_box(&output);
            Ok((elapsed, checksum))
        }
        _ => Err(error("Unknown startup method")),
    }
}

fn statistics_json(values: &[u128], divisor: f64) -> String {
    let mut ordered = values.to_vec();
    ordered.sort_unstable();
    let median = ordered[ordered.len() / 2] as f64 / divisor;
    let minimum = ordered[0] as f64 / divisor;
    let maximum = ordered[ordered.len() - 1] as f64 / divisor;
    format!("{{\"median\":{median:.3},\"min\":{minimum:.3},\"max\":{maximum:.3},\"batch_elapsed_ns\":{values:?}}}")
}

fn run() -> Result<()> {
    let arguments: Vec<String> = std::env::args().collect();
    if arguments.len() != 6 {
        return Err(error(
            "Usage: native INPUT OUTPUT_DIR ITERATIONS ROUNDS STARTUP_ROUNDS",
        ));
    }
    let input = Path::new(&arguments[1]);
    let destination = Path::new(&arguments[2]);
    let iterations: usize = arguments[3].parse()?;
    let rounds: usize = arguments[4].parse()?;
    let startup_rounds: usize = arguments[5].parse()?;
    if iterations == 0 || rounds == 0 || startup_rounds == 0 {
        return Err(error("Iteration and round counts must be positive"));
    }
    fs::create_dir_all(destination)?;
    let paths = Paths::new(destination);
    let input_bytes = fs::read(input)?;
    let input_text = std::str::from_utf8(&input_bytes)?;
    let source: Vec<String> = input_text.lines().map(str::to_owned).collect();
    if source.is_empty() || source.iter().any(|name| name.is_empty()) {
        return Err(error(
            "Benchmark input must contain nonempty names, one per line",
        ));
    }
    let source_set: BTreeSet<&str> = source.iter().map(String::as_str).collect();
    if source_set.len() != source.len() {
        return Err(error("Benchmark input must contain unique names"));
    }

    eprintln!("Building and verifying {} unique names...", source.len());
    let timer = Instant::now();
    let compact = build(
        source.iter(),
        &BuildOptions {
            mode: BuildMode::Compact,
            ..BuildOptions::default()
        },
    )?;
    let compact_build_ns = timer.elapsed().as_nanos();
    let timer = Instant::now();
    let raw = build(
        source.iter(),
        &BuildOptions {
            mode: BuildMode::Raw,
            ..BuildOptions::default()
        },
    )?;
    let raw_build_ns = timer.elapsed().as_nanos();
    fs::write(&paths.compact, &compact.bytes)?;
    fs::write(&paths.raw, &raw.bytes)?;
    let compact_table = Table::open(&compact.bytes)?;
    let raw_table = Table::open(&raw.bytes)?;
    let prepared = compact_table.prepare(&PrepareOptions {
        sampling: SamplingMethod::Modulo,
        ..PrepareOptions::default()
    })?;
    let mut prepared_output = prepared.new_output_buffer()?;
    raw_table.validate_all(true)?;
    if compact_table.len() != source.len() as u64 || raw_table.len() != compact_table.len() {
        return Err(error("Built table size differs from source"));
    }

    let mut names = Vec::with_capacity(source.len());
    for rank in 0..compact_table.len() {
        names.push(compact_table.get(rank)?.into_bytes());
    }
    let decoded_set: BTreeSet<&str> = names
        .iter()
        .map(|name| std::str::from_utf8(name).unwrap())
        .collect();
    if decoded_set != source_set {
        return Err(error("Compact table does not reconstruct the input set"));
    }
    let mut raw_by_name = BTreeMap::new();
    for rank in 0..raw_table.len() {
        raw_by_name.insert(raw_table.get(rank)?, rank);
    }
    if raw_by_name
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>()
        != source_set
    {
        return Err(error("Raw table does not reconstruct the input set"));
    }
    let sequence = ranks(iterations, compact_table.len());
    let raw_sequence: Vec<u64> = sequence
        .iter()
        .map(|&rank| raw_by_name[std::str::from_utf8(&names[rank as usize]).unwrap()])
        .collect();
    drop(raw_by_name);
    // Let whole-stream gzip exploit lexical locality; using the pack's bucket
    // order would make its size comparison needlessly unfavorable.
    let sorted_source: Vec<&str> = source_set.iter().copied().collect();
    let sorted_by_name: BTreeMap<&str, u64> = sorted_source
        .iter()
        .enumerate()
        .map(|(rank, &name)| (name, rank as u64))
        .collect();
    let sorted_sequence: Vec<u64> = sequence
        .iter()
        .map(|&rank| sorted_by_name[std::str::from_utf8(&names[rank as usize]).unwrap()])
        .collect();
    drop(sorted_by_name);
    let max_length = compact_table.max_value_bytes();

    let mut json = String::from("[");
    let mut text = Vec::with_capacity(input_bytes.len());
    for (index, name) in sorted_source.iter().enumerate() {
        if index != 0 {
            json.push(',');
        }
        json.push_str(&json_string(name));
        text.extend_from_slice(name.as_bytes());
        text.push(b'\n');
    }
    json.push(']');
    fs::write(&paths.json, json.as_bytes())?;
    write_gzip(&paths.gzip, &text)?;
    create_sqlite(&paths.sqlite, &names)?;

    let expected_checksum = sequence.iter().fold(HASH_START, |checksum, &rank| {
        consume(checksum, &names[rank as usize])
    });
    let db = Connection::open(&paths.sqlite, false)?;
    let mut statement = Statement::prepare(&db, "SELECT name FROM names WHERE id = ?1")?;
    let transaction_db = Connection::open(&paths.sqlite, false)?;
    let mut transaction_statement =
        Statement::prepare(&transaction_db, "SELECT name FROM names WHERE id = ?1")?;
    let methods = [
        "compact",
        "compact_prepared",
        "raw",
        "loaded_vector",
        "sqlite_prepared_autocommit",
        "sqlite_prepared_read_transaction",
    ];
    let mut measurements: BTreeMap<&str, Vec<u128>> =
        methods.iter().map(|&name| (name, Vec::new())).collect();
    eprintln!(
        "Measuring warm retrieval: {} rounds x {} lookups per method...",
        rounds, iterations
    );
    // Round zero is an unreported warmup. Rotate method order for later rounds.
    for round in 0..=rounds {
        for offset in 0..methods.len() {
            let method = methods[(round + offset) % methods.len()];
            let mut output = Vec::with_capacity(max_length);
            let mut checksum = HASH_START;
            if method == "sqlite_prepared_read_transaction" {
                transaction_db.execute("BEGIN")?;
                // Acquire the snapshot before timing this warmed reader. End
                // it after the batch so its process-wide file lock cannot
                // inadvertently accelerate the autocommit baseline.
                transaction_statement.read_into(sequence[0], &mut output)?;
            }
            let timer = Instant::now();
            match method {
                "compact" => {
                    for &rank in &sequence {
                        compact_table.get_into(rank, &mut output)?;
                        checksum = consume(checksum, &output);
                    }
                }
                "compact_prepared" => {
                    for &rank in &sequence {
                        let value = prepared.get_into(rank, &mut prepared_output)?;
                        checksum = consume(checksum, value.as_bytes());
                    }
                }
                "raw" => {
                    for &rank in &raw_sequence {
                        raw_table.get_into(rank, &mut output)?;
                        checksum = consume(checksum, &output);
                    }
                }
                "loaded_vector" => {
                    for &rank in &sequence {
                        output.clear();
                        output.extend_from_slice(&names[rank as usize]);
                        checksum = consume_checked(checksum, &output)?;
                    }
                }
                "sqlite_prepared_autocommit" => {
                    for &rank in &sequence {
                        statement.read_into(rank, &mut output)?;
                        checksum = consume_checked(checksum, &output)?;
                    }
                }
                "sqlite_prepared_read_transaction" => {
                    for &rank in &sequence {
                        transaction_statement.read_into(rank, &mut output)?;
                        checksum = consume_checked(checksum, &output)?;
                    }
                }
                _ => unreachable!(),
            }
            let elapsed = timer.elapsed().as_nanos();
            if method == "sqlite_prepared_read_transaction" {
                transaction_db.execute("COMMIT")?;
            }
            black_box(checksum);
            if checksum != expected_checksum {
                return Err(error(format!("Checksum mismatch for {method}")));
            }
            if round != 0 {
                measurements.get_mut(method).unwrap().push(elapsed);
            }
        }
    }

    let sampling_methods = [
        "compact_sample_into",
        "compact_prepared_sample_into",
        "loaded_vector",
        "sqlite_prepared_read_transaction",
    ];
    let mut sampling_measurements: BTreeMap<&str, Vec<u128>> = sampling_methods
        .iter()
        .map(|&name| (name, Vec::new()))
        .collect();
    let name_count = compact_table.len();
    eprintln!("Measuring complete warm sampling, including RNG and range reduction...");
    for round in 0..=rounds {
        for offset in 0..sampling_methods.len() {
            let method = sampling_methods[(round + offset) % sampling_methods.len()];
            let mut output = Vec::with_capacity(max_length);
            let mut checksum = HASH_START;
            let mut random_state = SAMPLING_SEED;
            let mut random = || next_u64(&mut random_state);
            if method == "sqlite_prepared_read_transaction" {
                transaction_db.execute("BEGIN")?;
                transaction_statement.read_into(sequence[0], &mut output)?;
            }
            let timer = Instant::now();
            match method {
                "compact_sample_into" => {
                    for _ in 0..iterations {
                        compact_table.sample_into(&mut random, &mut output)?;
                        checksum = consume(checksum, &output);
                    }
                }
                "compact_prepared_sample_into" => {
                    for _ in 0..iterations {
                        let value = prepared.sample_into(&mut random, &mut prepared_output)?;
                        checksum = consume(checksum, value.as_bytes());
                    }
                }
                "loaded_vector" => {
                    for _ in 0..iterations {
                        let rank = uniform_below(name_count, &mut random)?;
                        output.clear();
                        output.extend_from_slice(&names[rank as usize]);
                        checksum = consume_checked(checksum, &output)?;
                    }
                }
                "sqlite_prepared_read_transaction" => {
                    for _ in 0..iterations {
                        let rank = uniform_below(name_count, &mut random)?;
                        transaction_statement.read_into(rank, &mut output)?;
                        checksum = consume_checked(checksum, &output)?;
                    }
                }
                _ => unreachable!(),
            }
            let elapsed = timer.elapsed().as_nanos();
            if method == "sqlite_prepared_read_transaction" {
                transaction_db.execute("COMMIT")?;
            }
            black_box(checksum);
            if checksum != expected_checksum {
                return Err(error(format!("Sampling checksum mismatch for {method}")));
            }
            if round != 0 {
                sampling_measurements.get_mut(method).unwrap().push(elapsed);
            }
        }
    }

    let startup_methods = [
        "compact_crc_open",
        "compact_structural_open",
        "raw_crc_open",
        "sqlite_open_prepare_lookup",
        "json_c_read_parse_lookup",
        "gzip_text_read_decode_parse_lookup",
    ];
    let mut startup_measurements: BTreeMap<&str, Vec<u128>> = startup_methods
        .iter()
        .map(|&name| (name, Vec::new()))
        .collect();
    // Explicitly warm the OS page cache. These measurements are NOT cold I/O.
    for path in [
        &paths.compact,
        &paths.raw,
        &paths.json,
        &paths.gzip,
        &paths.sqlite,
    ] {
        black_box(fs::read(path)?);
    }
    eprintln!("Measuring reader startup with warm OS page cache...");
    for round in 0..=startup_rounds {
        let index = round % sequence.len();
        let expected = consume(HASH_START, &names[sequence[index] as usize]);
        for offset in 0..startup_methods.len() {
            let method = startup_methods[(round + offset) % startup_methods.len()];
            let rank = if method == "raw_crc_open" {
                raw_sequence[index]
            } else if method == "json_c_read_parse_lookup"
                || method == "gzip_text_read_decode_parse_lookup"
            {
                sorted_sequence[index]
            } else {
                sequence[index]
            };
            let (elapsed, checksum) = startup(method, &paths, rank, max_length)?;
            if checksum != expected {
                return Err(error(format!("Startup checksum mismatch for {method}")));
            }
            if round != 0 {
                startup_measurements.get_mut(method).unwrap().push(elapsed);
            }
        }
    }

    let raw_name_bytes: usize = names.iter().map(Vec::len).sum();
    let sqlite_version = unsafe { CStr::from_ptr(sqlite3_libversion()).to_string_lossy() };
    let json_version = unsafe { CStr::from_ptr(json_c_version()).to_string_lossy() };
    let zlib_version = unsafe { CStr::from_ptr(zlibVersion()).to_string_lossy() };
    let mut report = String::new();
    write!(report, "{{\n  \"schema_version\":3,\n  \"dataset\":{{\"path\":{},\"unique_names\":{},\"maximum_utf8_bytes\":{}}},\n",
        json_string(&input.display().to_string()), names.len(), max_length)?;
    writeln!(report, "  \"environment\":{{\"os\":{},\"arch\":{},\"sqlite_version\":{},\"json_c_version\":{},\"zlib_version\":{}}},",
        json_string(std::env::consts::OS), json_string(std::env::consts::ARCH),
        json_string(&sqlite_version), json_string(&json_version), json_string(&zlib_version))?;
    writeln!(report, "  \"methodology\":{{\"iterations_per_batch\":{iterations},\"timed_batches\":{rounds},\"startup_rounds\":{startup_rounds},\"unreported_warmup_batches\":1,\"rng_timed\":{{\"warm_retrieval\":false,\"warm_sampling\":true}},\"sampling_rng\":\"SplitMix64 (noncryptographic)\",\"sampling_range_reduction\":\"pkbyte::uniform_below\",\"rng_reseeded_before_each_batch\":true,\"full_byte_checksum_timed\":true,\"result_copy_timed\":true,\"result_utf8_checked_timed\":true,\"warm_buffers_preallocated\":true,\"startup_output_buffer_preallocated\":true,\"os_page_cache\":\"warm\",\"startup_excludes_reader_teardown\":true,\"rank_translations_precomputed\":true,\"json_gzip_order\":\"lexical\",\"sqlite_single_thread_nomutex\":true,\"individual_request_percentiles\":false,\"sampling_seed_hex\":\"{SAMPLING_SEED:016x}\",\"checksum_hex\":\"{expected_checksum:016x}\"}},")?;
    writeln!(report, "  \"sizes_bytes\":{{\"name_bytes_without_delimiters\":{raw_name_bytes},\"input_text\":{},\"compact_json\":{},\"gzip_text_level_9\":{},\"sqlite\":{},\"pkbyte_raw\":{},\"pkbyte_compact\":{}}},",
        input_bytes.len(), fs::metadata(&paths.json)?.len(), fs::metadata(&paths.gzip)?.len(),
        fs::metadata(&paths.sqlite)?.len(), raw.bytes.len(), compact.bytes.len())?;
    writeln!(
        report,
        "  \"build_time_ns\":{{\"compact\":{compact_build_ns},\"raw\":{raw_build_ns}}},"
    )?;
    report.push_str("  \"warm_retrieval_ns_per_lookup\":{\n");
    for (index, (method, values)) in measurements.iter().enumerate() {
        if index != 0 {
            report.push_str(",\n");
        }
        write!(
            report,
            "    {}:{}",
            json_string(method),
            statistics_json(values, iterations as f64)
        )?;
    }
    report.push_str("\n  },\n  \"warm_sampling_ns_per_name\":{\n");
    for (index, (method, values)) in sampling_measurements.iter().enumerate() {
        if index != 0 {
            report.push_str(",\n");
        }
        write!(
            report,
            "    {}:{}",
            json_string(method),
            statistics_json(values, iterations as f64)
        )?;
    }
    report.push_str("\n  },\n  \"startup_warm_cache_microseconds\":{\n");
    for (index, (method, values)) in startup_measurements.iter().enumerate() {
        if index != 0 {
            report.push_str(",\n");
        }
        write!(
            report,
            "    {}:{}",
            json_string(method),
            statistics_json(values, 1000.0)
        )?;
    }
    report.push_str("\n  }\n}\n");
    print!("{report}");
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("Benchmark failed: {error}");
        std::process::exit(1);
    }
}
