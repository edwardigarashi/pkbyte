use pkbyte::build_support::{build_from_reader, InputFormat};
use pkbyte::{BuildOptions, Error};
use std::io::{self, Read};

struct MeteredReader<R> {
    input: R,
    bytes_read: usize,
}

impl<R: Read> Read for MeteredReader<R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let read = self.input.read(output)?;
        self.bytes_read += read;
        Ok(read)
    }
}

#[test]
fn exhausted_row_budget_stops_before_reading_the_next_record() {
    let input = b"first\n".as_slice().chain(io::repeat(b'x').take(1 << 20));
    let mut reader = MeteredReader {
        input,
        bytes_read: 0,
    };
    let options = BuildOptions {
        max_input_rows: 1,
        ..BuildOptions::default()
    };

    let result = build_from_reader(&mut reader, InputFormat::Lines, &options);
    assert!(matches!(
        result,
        Err(Error::LimitExceeded("input row count"))
    ));
    // Buffered I/O can read ahead. It should inspect only a bounded prefix
    // of the next record, even if it has no line terminator.
    assert!(
        reader.bytes_read < 64 * 1024,
        "read {} bytes",
        reader.bytes_read
    );
    let exact = build_from_reader(b"first\n".as_slice(), InputFormat::Lines, &options).unwrap();
    assert_eq!(exact.stats.row_count, 1);
}
