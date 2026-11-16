//! A small, strict JSON reader for Connect's messages, and the escaping
//! its writers use.
//!
//! Strict where JSON parsers commonly differ, so the panel, the Worker and
//! a controller can never read one message two ways:
//!
//! - a **duplicate key** in any object is an error (parsers differ on
//!   whether the first or the last wins);
//! - numbers are **integers only**, `-?(0|[1-9][0-9]*)`, at most
//!   2^53 - 1 in magnitude (what JavaScript holds exactly): no fraction,
//!   no exponent, no `-0`;
//! - strings must be valid UTF-8 with no raw control character, no lone
//!   surrogate in a `\u` escape, and no U+0000;
//! - nesting at most [`MAX_DEPTH`] deep; nothing but whitespace after the
//!   value; the top level is an object.
//!
//! Only what the messages need: no streaming, no floats.

/// The deepest nesting taken (a command body is two levels; the shared
/// vectors' own documents are read with serde in the tests).
pub const MAX_DEPTH: usize = 4;

/// The largest integer exactly representable in an IEEE double.
pub const MAX_SAFE: i64 = (1 << 53) - 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Str(String),
    Array(Vec<Value>),
    Object(Vec<(String, Value)>),
}

impl Value {
    /// The member `key` of an object.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(n) => Some(*n),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }
    pub fn is_object(&self) -> bool {
        matches!(self, Value::Object(_))
    }
}

/// Why a document was refused (no positions: nothing shows them).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonError {
    Syntax,
    DuplicateKey,
    Number,
    BadString,
    TooDeep,
    NotAnObject,
}

/// Parse `text` as one JSON object.
pub fn parse(text: &[u8]) -> Result<Value, JsonError> {
    let mut p = Parser { s: text, i: 0 };
    p.ws();
    let v = p.value(0)?;
    p.ws();
    if p.i != p.s.len() {
        return Err(JsonError::Syntax);
    }
    if !v.is_object() {
        return Err(JsonError::NotAnObject);
    }
    Ok(v)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }
    fn ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }
    fn eat(&mut self, c: u8) -> Result<(), JsonError> {
        if self.peek() == Some(c) {
            self.i += 1;
            Ok(())
        } else {
            Err(JsonError::Syntax)
        }
    }
    fn lit(&mut self, word: &[u8], v: Value) -> Result<Value, JsonError> {
        if self.s[self.i..].starts_with(word) {
            self.i += word.len();
            Ok(v)
        } else {
            Err(JsonError::Syntax)
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, JsonError> {
        match self.peek().ok_or(JsonError::Syntax)? {
            b'{' => self.object(depth + 1),
            b'[' => self.array(depth + 1),
            b'"' => Ok(Value::Str(self.string()?)),
            b't' => self.lit(b"true", Value::Bool(true)),
            b'f' => self.lit(b"false", Value::Bool(false)),
            b'n' => self.lit(b"null", Value::Null),
            b'-' | b'0'..=b'9' => self.number(),
            _ => Err(JsonError::Syntax),
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value, JsonError> {
        if depth > MAX_DEPTH {
            return Err(JsonError::TooDeep);
        }
        self.eat(b'{')?;
        let mut members: Vec<(String, Value)> = Vec::new();
        self.ws();
        if self.peek() == Some(b'}') {
            self.i += 1;
            return Ok(Value::Object(members));
        }
        loop {
            self.ws();
            if self.peek() != Some(b'"') {
                return Err(JsonError::Syntax);
            }
            let key = self.string()?;
            if members.iter().any(|(k, _)| *k == key) {
                return Err(JsonError::DuplicateKey);
            }
            self.ws();
            self.eat(b':')?;
            self.ws();
            let v = self.value(depth)?;
            members.push((key, v));
            self.ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    self.i += 1;
                    return Ok(Value::Object(members));
                }
                _ => return Err(JsonError::Syntax),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Value, JsonError> {
        if depth > MAX_DEPTH {
            return Err(JsonError::TooDeep);
        }
        self.eat(b'[')?;
        let mut items = Vec::new();
        self.ws();
        if self.peek() == Some(b']') {
            self.i += 1;
            return Ok(Value::Array(items));
        }
        loop {
            self.ws();
            items.push(self.value(depth)?);
            self.ws();
            match self.peek() {
                Some(b',') => self.i += 1,
                Some(b']') => {
                    self.i += 1;
                    return Ok(Value::Array(items));
                }
                _ => return Err(JsonError::Syntax),
            }
        }
    }

    fn number(&mut self) -> Result<Value, JsonError> {
        let neg = self.peek() == Some(b'-');
        if neg {
            self.i += 1;
        }
        let digits = self.i;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.i += 1;
        }
        let d = &self.s[digits..self.i];
        if d.is_empty() || (d.len() > 1 && d[0] == b'0') || d.len() > 16 {
            return Err(JsonError::Number);
        }
        if matches!(self.peek(), Some(b'.' | b'e' | b'E')) {
            return Err(JsonError::Number);
        }
        let mut n: i64 = 0;
        for &c in d {
            n = n * 10 + i64::from(c - b'0');
        }
        if n > MAX_SAFE || (neg && n == 0) {
            return Err(JsonError::Number);
        }
        Ok(Value::Int(if neg { -n } else { n }))
    }

    fn hex4(&mut self) -> Result<u32, JsonError> {
        let h = self.s.get(self.i..self.i + 4).ok_or(JsonError::BadString)?;
        let mut v = 0u32;
        for &c in h {
            let d = (c as char).to_digit(16).ok_or(JsonError::BadString)?;
            v = (v << 4) | d;
        }
        self.i += 4;
        Ok(v)
    }

    fn string(&mut self) -> Result<String, JsonError> {
        self.eat(b'"')?;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let c = self.peek().ok_or(JsonError::BadString)?;
            self.i += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let e = self.peek().ok_or(JsonError::BadString)?;
                    self.i += 1;
                    let ch = match e {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let hi = self.hex4()?;
                            let cp = if (0xD800..0xDC00).contains(&hi) {
                                if self.s.get(self.i..self.i + 2) != Some(b"\\u") {
                                    return Err(JsonError::BadString);
                                }
                                self.i += 2;
                                let lo = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&lo) {
                                    return Err(JsonError::BadString);
                                }
                                0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                            } else {
                                hi
                            };
                            match char::from_u32(cp) {
                                Some(ch) if cp != 0 => ch,
                                _ => return Err(JsonError::BadString),
                            }
                        }
                        _ => return Err(JsonError::BadString),
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                0x00..=0x1F => return Err(JsonError::BadString),
                _ => out.push(c),
            }
        }
        String::from_utf8(out).map_err(|_| JsonError::BadString)
    }
}

/// Append `s` to `out` as a JSON string, quotes included. Escapes `"`,
/// `\` and every control character; everything else is written as is.
pub fn push_str(out: &mut String, s: &str) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// `s` as a JSON string.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    push_str(&mut out, s);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_command_shape() {
        let v = parse(br#"{"v":1,"dev":"x","n":42,"c":"next","a":{"p":[true,false,null]}}"#).unwrap();
        assert_eq!(v.get("v").and_then(Value::as_int), Some(1));
        assert_eq!(v.get("n").and_then(Value::as_int), Some(42));
        assert_eq!(v.get("c").and_then(Value::as_str), Some("next"));
        assert!(v.get("a").unwrap().is_object());
    }

    #[test]
    fn refuses_what_parsers_disagree_on() {
        assert_eq!(parse(br#"{"c":"next","c":"reboot"}"#), Err(JsonError::DuplicateKey));
        assert_eq!(parse(br#"{"a":{"v":1,"v":2}}"#), Err(JsonError::DuplicateKey));
        // The same key spelled with an escape is still the same key.
        assert_eq!(parse(br#"{"c":"next","\u0063":"reboot"}"#), Err(JsonError::DuplicateKey));
        for n in ["1.0", "1e3", "-0", "01", "9007199254740992", "12345678901234567", "+1", ".5"] {
            assert!(parse(format!("{{\"n\":{n}}}").as_bytes()).is_err(), "{n}");
        }
        assert!(parse(br#"{"n":9007199254740991}"#).is_ok());
        assert!(parse(br#"{"n":-9007199254740991}"#).is_ok());
    }

    #[test]
    fn strings() {
        let v = parse(br#"{"s":"a\"b\\c\/d\u00e9\ud83d\ude00"}"#).unwrap();
        assert_eq!(v.get("s").and_then(Value::as_str), Some("a\"b\\c/d\u{e9}\u{1F600}"));
        assert_eq!(parse(b"{\"s\":\"a\nb\"}"), Err(JsonError::BadString), "raw control char");
        assert_eq!(parse(br#"{"s":"\ud800"}"#), Err(JsonError::BadString), "lone high surrogate");
        assert_eq!(parse(br#"{"s":"\udc00"}"#), Err(JsonError::BadString), "lone low surrogate");
        assert_eq!(parse(br#"{"s":"\u0000"}"#), Err(JsonError::BadString), "NUL");
        assert_eq!(parse(br#"{"s":"\x"}"#), Err(JsonError::BadString));
        assert_eq!(parse(b"{\"s\":\"\xff\"}"), Err(JsonError::BadString), "invalid UTF-8");
        assert_eq!(parse(br#"{"s":"abc}"#), Err(JsonError::BadString), "unterminated");
    }

    #[test]
    fn structure() {
        assert_eq!(parse(b"[]"), Err(JsonError::NotAnObject));
        assert_eq!(parse(b"\"x\""), Err(JsonError::NotAnObject));
        assert_eq!(parse(b"{} x"), Err(JsonError::Syntax));
        assert_eq!(parse(b"{}{}"), Err(JsonError::Syntax));
        assert_eq!(parse(b"{\"a\":1,}"), Err(JsonError::Syntax));
        assert_eq!(parse(b"{\"a\" 1}"), Err(JsonError::Syntax));
        assert_eq!(parse(b""), Err(JsonError::Syntax));
        assert_eq!(parse(b"{"), Err(JsonError::Syntax));
        assert!(parse(b" \n{ \"a\" : [ 1 , 2 ] }\r\n").is_ok());
        assert!(parse(br#"{"a":{"b":{"c":{}}}}"#).is_ok());
        assert_eq!(parse(br#"{"a":{"b":{"c":{"d":{}}}}}"#), Err(JsonError::TooDeep));
        assert_eq!(parse(br#"{"a":[[[[]]]]}"#), Err(JsonError::TooDeep));
        // A deep bomb is refused at the limit, not by the stack.
        let bomb = "[".repeat(100_000);
        assert!(parse(format!("{{\"a\":{bomb}").as_bytes()).is_err());
    }

    #[test]
    fn writer_round_trips() {
        for s in ["", "plain", "quo\"te", "back\\slash", "nl\nx", "\u{1}\u{1f}\u{7f}", "\u{e9}\u{1f600}"] {
            let doc = format!("{{\"s\":{}}}", quote(s));
            assert_eq!(parse(doc.as_bytes()).unwrap().get("s").and_then(Value::as_str), Some(s));
        }
    }
}
