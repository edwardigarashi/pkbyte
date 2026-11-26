use std::str::Chars;

pub(crate) fn decode_json_string(line: &[u8], maximum: usize) -> Result<String, &'static str> {
    let line = std::str::from_utf8(line).map_err(|_| "expected valid UTF-8")?;
    let line = line.trim_matches(&[' ', '\t', '\r'][..]);
    let content = line.strip_prefix('"').ok_or("expected one JSON string")?;
    let mut chars = content.chars();
    let mut value = String::new();
    while let Some(character) = chars.next() {
        let decoded = match character {
            '"' if chars.as_str().is_empty() => return Ok(value),
            '"' => return Err("unexpected data after the JSON string"),
            '\\' => decode_escape(&mut chars)?,
            '\0'..='\u{1f}' => return Err("unescaped control character in the JSON string"),
            ordinary => ordinary,
        };
        if decoded.len_utf8() > maximum - value.len() {
            return Err("decoded value exceeds the configured byte limit");
        }
        value.push(decoded);
    }
    Err("unterminated JSON string")
}

fn decode_escape(chars: &mut Chars<'_>) -> Result<char, &'static str> {
    match chars.next() {
        Some(escaped @ ('"' | '\\' | '/')) => Ok(escaped),
        Some('b') => Ok('\x08'),
        Some('f') => Ok('\x0c'),
        Some('n') => Ok('\n'),
        Some('r') => Ok('\r'),
        Some('t') => Ok('\t'),
        Some('u') => decode_unicode(chars),
        _ => Err("invalid JSON escape"),
    }
}

fn decode_unicode(chars: &mut Chars<'_>) -> Result<char, &'static str> {
    let first = hex_quad(chars)?;
    let scalar = if (0xd800..=0xdbff).contains(&first) {
        if chars.next() != Some('\\') || chars.next() != Some('u') {
            return Err("unpaired Unicode surrogate");
        }
        let second = hex_quad(chars)?;
        if !(0xdc00..=0xdfff).contains(&second) {
            return Err("unpaired Unicode surrogate");
        }
        0x10000 + ((first - 0xd800) << 10) + (second - 0xdc00)
    } else {
        first
    };
    char::from_u32(scalar).ok_or("unpaired Unicode surrogate")
}

fn hex_quad(chars: &mut Chars<'_>) -> Result<u32, &'static str> {
    let mut value = 0;
    for _ in 0..4 {
        let digit = chars
            .next()
            .and_then(|character| character.to_digit(16))
            .ok_or("invalid Unicode escape")?;
        value = (value << 4) | digit;
    }
    Ok(value)
}
