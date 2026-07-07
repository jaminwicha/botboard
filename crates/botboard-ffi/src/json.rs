//! Minimal recursive-descent JSON parser for the FFI boundary. The
//! workspace is dependency-free by policy (§10.1: the core owns its
//! formats), so the battle-setup schema is parsed with this tiny value
//! model rather than serde. Accepts strict JSON; numbers parse as f64.

#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(kv) => kv.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }
    pub fn as_u64(&self) -> Option<u64> {
        self.as_f64().map(|n| n as u64)
    }
    pub fn as_i64(&self) -> Option<i64> {
        self.as_f64().map(|n| n as i64)
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }
    pub fn as_arr(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(v) => Some(v),
            _ => None,
        }
    }
    /// Convenience: `obj.num("w", 8.0)` with default.
    pub fn num(&self, key: &str, default: f64) -> f64 {
        self.get(key).and_then(|v| v.as_f64()).unwrap_or(default)
    }
    pub fn str_or<'a>(&'a self, key: &str, default: &'a str) -> &'a str {
        self.get(key).and_then(|v| v.as_str()).unwrap_or(default)
    }
    pub fn bool_or(&self, key: &str, default: bool) -> bool {
        self.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
    }
}

pub fn parse(s: &str) -> Result<Json, String> {
    let b = s.as_bytes();
    let mut i = 0;
    let v = value(b, &mut i)?;
    skip_ws(b, &mut i);
    if i != b.len() {
        return Err(format!("trailing data at byte {i}"));
    }
    Ok(v)
}

fn skip_ws(b: &[u8], i: &mut usize) {
    while *i < b.len() && matches!(b[*i], b' ' | b'\t' | b'\n' | b'\r') {
        *i += 1;
    }
}

fn value(b: &[u8], i: &mut usize) -> Result<Json, String> {
    skip_ws(b, i);
    match b.get(*i) {
        None => Err("unexpected end".into()),
        Some(b'{') => obj(b, i),
        Some(b'[') => arr(b, i),
        Some(b'"') => Ok(Json::Str(string(b, i)?)),
        Some(b't') => lit(b, i, "true", Json::Bool(true)),
        Some(b'f') => lit(b, i, "false", Json::Bool(false)),
        Some(b'n') => lit(b, i, "null", Json::Null),
        Some(_) => number(b, i),
    }
}

fn lit(b: &[u8], i: &mut usize, word: &str, v: Json) -> Result<Json, String> {
    if b[*i..].starts_with(word.as_bytes()) {
        *i += word.len();
        Ok(v)
    } else {
        Err(format!("bad literal at byte {}", *i))
    }
}

fn number(b: &[u8], i: &mut usize) -> Result<Json, String> {
    let start = *i;
    while *i < b.len() && matches!(b[*i], b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E') {
        *i += 1;
    }
    std::str::from_utf8(&b[start..*i])
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .map(Json::Num)
        .ok_or_else(|| format!("bad number at byte {start}"))
}

fn string(b: &[u8], i: &mut usize) -> Result<String, String> {
    debug_assert_eq!(b[*i], b'"');
    *i += 1;
    let mut out = String::new();
    while *i < b.len() {
        match b[*i] {
            b'"' => {
                *i += 1;
                return Ok(out);
            }
            b'\\' => {
                *i += 1;
                match b.get(*i) {
                    Some(b'n') => out.push('\n'),
                    Some(b't') => out.push('\t'),
                    Some(b'r') => out.push('\r'),
                    Some(b'u') => {
                        let hex = std::str::from_utf8(&b[*i + 1..*i + 5])
                            .map_err(|_| "bad \\u escape")?;
                        let cp = u32::from_str_radix(hex, 16).map_err(|_| "bad \\u escape")?;
                        out.push(char::from_u32(cp).unwrap_or('?'));
                        *i += 4;
                    }
                    Some(&c) => out.push(c as char),
                    None => return Err("unterminated escape".into()),
                }
                *i += 1;
            }
            c => {
                // Pass through UTF-8 bytes untouched.
                let len = utf8_len(c);
                out.push_str(
                    std::str::from_utf8(&b[*i..*i + len]).map_err(|_| "bad utf-8")?,
                );
                *i += len;
            }
        }
    }
    Err("unterminated string".into())
}

fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

fn arr(b: &[u8], i: &mut usize) -> Result<Json, String> {
    *i += 1; // '['
    let mut out = Vec::new();
    skip_ws(b, i);
    if b.get(*i) == Some(&b']') {
        *i += 1;
        return Ok(Json::Arr(out));
    }
    loop {
        out.push(value(b, i)?);
        skip_ws(b, i);
        match b.get(*i) {
            Some(b',') => *i += 1,
            Some(b']') => {
                *i += 1;
                return Ok(Json::Arr(out));
            }
            _ => return Err(format!("expected , or ] at byte {}", *i)),
        }
    }
}

fn obj(b: &[u8], i: &mut usize) -> Result<Json, String> {
    *i += 1; // '{'
    let mut out = Vec::new();
    skip_ws(b, i);
    if b.get(*i) == Some(&b'}') {
        *i += 1;
        return Ok(Json::Obj(out));
    }
    loop {
        skip_ws(b, i);
        if b.get(*i) != Some(&b'"') {
            return Err(format!("expected key at byte {}", *i));
        }
        let k = string(b, i)?;
        skip_ws(b, i);
        if b.get(*i) != Some(&b':') {
            return Err(format!("expected : at byte {}", *i));
        }
        *i += 1;
        let v = value(b, i)?;
        out.push((k, v));
        skip_ws(b, i);
        match b.get(*i) {
            Some(b',') => *i += 1,
            Some(b'}') => {
                *i += 1;
                return Ok(Json::Obj(out));
            }
            _ => return Err(format!("expected , or }} at byte {}", *i)),
        }
    }
}
