//! Reading a Claude Code transcript. `Line` is one typed, borrowed transcript line that every
//! reader shares (pace, the session store, the scanners). `rev_lines` and `context_size` read a
//! transcript from its end: the guard hooks for Claude Code and Cursor use them to find the
//! context size of the newest assistant turn without reading the whole file.
//!
//! A transcript is written by another program and its fields change type now and then. So every
//! field of `Line` is lenient: a value of an unexpected type reads as absent and never fails the
//! line. Only text that is not a JSON object fails.

use serde::Deserialize;
use serde::de::value::{MapAccessDeserializer, SeqAccessDeserializer};
use serde::de::{self, Deserializer, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::Value;
use std::borrow::Cow;
use std::fmt;
use std::fs;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

// --- lenient field types ---------------------------------------------------------------------

/// Read and drop whatever is next, so a lenient field can skip a value of the wrong type.
fn skip_map<'de, A: MapAccess<'de>>(mut m: A) -> Result<(), A::Error> {
    while m.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
    Ok(())
}

fn skip_seq<'de, A: SeqAccess<'de>>(mut s: A) -> Result<(), A::Error> {
    while s.next_element::<IgnoredAny>()?.is_some() {}
    Ok(())
}

/// The `visit_*` methods of a visitor that skips every kind of value named in the list.
macro_rules! skip {
    (bool) => {
        fn visit_bool<E: de::Error>(self, _: bool) -> Result<Self::Value, E> {
            Ok(Self::Value::default())
        }
    };
    (num) => {
        fn visit_i64<E: de::Error>(self, _: i64) -> Result<Self::Value, E> {
            Ok(Self::Value::default())
        }
        fn visit_u64<E: de::Error>(self, _: u64) -> Result<Self::Value, E> {
            Ok(Self::Value::default())
        }
        fn visit_f64<E: de::Error>(self, _: f64) -> Result<Self::Value, E> {
            Ok(Self::Value::default())
        }
    };
    (str) => {
        fn visit_str<E: de::Error>(self, _: &str) -> Result<Self::Value, E> {
            Ok(Self::Value::default())
        }
    };
    (null) => {
        fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(Self::Value::default())
        }
        fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
            Ok(Self::Value::default())
        }
        fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Self::Value, D::Error> {
            IgnoredAny::deserialize(d).map(|_| Self::Value::default())
        }
    };
    (seq) => {
        fn visit_seq<A: SeqAccess<'de>>(self, s: A) -> Result<Self::Value, A::Error> {
            skip_seq(s).map(|()| Self::Value::default())
        }
    };
    (map) => {
        fn visit_map<A: MapAccess<'de>>(self, m: A) -> Result<Self::Value, A::Error> {
            skip_map(m).map(|()| Self::Value::default())
        }
    };
}

/// A string field. A value of another type reads as absent.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Str<'a>(pub Option<Cow<'a, str>>);

impl Str<'_> {
    pub fn as_str(&self) -> Option<&str> {
        self.0.as_deref()
    }

    /// The text, or "" when absent.
    pub fn or_empty(&self) -> &str {
        self.as_str().unwrap_or_default()
    }
}

impl<'de: 'a, 'a> Deserialize<'de> for Str<'a> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<'a>(std::marker::PhantomData<&'a ()>);
        impl<'de: 'a, 'a> Visitor<'de> for V<'a> {
            type Value = Str<'a>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("any value")
            }
            fn visit_borrowed_str<E: de::Error>(self, s: &'de str) -> Result<Str<'a>, E> {
                Ok(Str(Some(Cow::Borrowed(s))))
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<Str<'a>, E> {
                Ok(Str(Some(Cow::Owned(s.to_owned()))))
            }
            skip!(bool);
            skip!(num);
            skip!(null);
            skip!(seq);
            skip!(map);
        }
        d.deserialize_any(V(std::marker::PhantomData))
    }
}

/// A flag field read with the truthiness of Python: false, 0, "", null, [] and {} are false.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Truthy(pub bool);

impl<'de> Deserialize<'de> for Truthy {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Truthy;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("any value")
            }
            fn visit_bool<E: de::Error>(self, b: bool) -> Result<Truthy, E> {
                Ok(Truthy(b))
            }
            fn visit_i64<E: de::Error>(self, n: i64) -> Result<Truthy, E> {
                Ok(Truthy(n != 0))
            }
            fn visit_u64<E: de::Error>(self, n: u64) -> Result<Truthy, E> {
                Ok(Truthy(n != 0))
            }
            fn visit_f64<E: de::Error>(self, n: f64) -> Result<Truthy, E> {
                Ok(Truthy(n != 0.0))
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<Truthy, E> {
                Ok(Truthy(!s.is_empty()))
            }
            skip!(null);
            fn visit_seq<A: SeqAccess<'de>>(self, mut s: A) -> Result<Truthy, A::Error> {
                let any = s.next_element::<IgnoredAny>()?.is_some();
                skip_seq(s)?;
                Ok(Truthy(any))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut m: A) -> Result<Truthy, A::Error> {
                let any = m.next_entry::<IgnoredAny, IgnoredAny>()?.is_some();
                skip_map(m)?;
                Ok(Truthy(any))
            }
        }
        d.deserialize_any(V)
    }
}

/// A number field. `present` tells a key that was there from one that was not, even when its
/// value is not a number (then `value` is None).
#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub struct Num {
    pub value: Option<f64>,
    pub present: bool,
}

impl Num {
    /// The number, or 0 when absent or not a number.
    pub fn get(self) -> f64 {
        self.value.unwrap_or(0.0)
    }
}

impl<'de> Deserialize<'de> for Num {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Num;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("any value")
            }
            fn visit_i64<E: de::Error>(self, n: i64) -> Result<Num, E> {
                Ok(Num { value: Some(n as f64), present: true })
            }
            fn visit_u64<E: de::Error>(self, n: u64) -> Result<Num, E> {
                Ok(Num { value: Some(n as f64), present: true })
            }
            fn visit_f64<E: de::Error>(self, n: f64) -> Result<Num, E> {
                Ok(Num { value: Some(n), present: true })
            }
            fn visit_bool<E: de::Error>(self, _: bool) -> Result<Num, E> {
                Ok(Num { value: None, present: true })
            }
            fn visit_str<E: de::Error>(self, _: &str) -> Result<Num, E> {
                Ok(Num { value: None, present: true })
            }
            fn visit_unit<E: de::Error>(self) -> Result<Num, E> {
                Ok(Num { value: None, present: true })
            }
            fn visit_none<E: de::Error>(self) -> Result<Num, E> {
                Ok(Num { value: None, present: true })
            }
            fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Num, D::Error> {
                IgnoredAny::deserialize(d).map(|_| Num { value: None, present: true })
            }
            fn visit_seq<A: SeqAccess<'de>>(self, s: A) -> Result<Num, A::Error> {
                skip_seq(s).map(|()| Num { value: None, present: true })
            }
            fn visit_map<A: MapAccess<'de>>(self, m: A) -> Result<Num, A::Error> {
                skip_map(m).map(|()| Num { value: None, present: true })
            }
        }
        d.deserialize_any(V)
    }
}

/// A timestamp: the transcript writes an ISO string, the status line cache a number.
#[derive(Debug, Default, Clone, PartialEq)]
pub enum Stamp<'a> {
    #[default]
    None,
    Text(Cow<'a, str>),
    Int(i64),
    Float(f64),
}

impl Stamp<'_> {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Stamp::Text(s) => Some(s),
            _ => None,
        }
    }
}

impl<'de: 'a, 'a> Deserialize<'de> for Stamp<'a> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<'a>(std::marker::PhantomData<&'a ()>);
        impl<'de: 'a, 'a> Visitor<'de> for V<'a> {
            type Value = Stamp<'a>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("any value")
            }
            fn visit_borrowed_str<E: de::Error>(self, s: &'de str) -> Result<Stamp<'a>, E> {
                Ok(Stamp::Text(Cow::Borrowed(s)))
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<Stamp<'a>, E> {
                Ok(Stamp::Text(Cow::Owned(s.to_owned())))
            }
            fn visit_i64<E: de::Error>(self, n: i64) -> Result<Stamp<'a>, E> {
                Ok(Stamp::Int(n))
            }
            fn visit_u64<E: de::Error>(self, n: u64) -> Result<Stamp<'a>, E> {
                Ok(i64::try_from(n).map_or(Stamp::Float(n as f64), Stamp::Int))
            }
            fn visit_f64<E: de::Error>(self, n: f64) -> Result<Stamp<'a>, E> {
                Ok(Stamp::Float(n))
            }
            skip!(bool);
            skip!(null);
            skip!(seq);
            skip!(map);
        }
        d.deserialize_any(V(std::marker::PhantomData))
    }
}

/// An object field. A value of another type reads as absent.
#[derive(Debug, Clone, PartialEq)]
pub struct Obj<T>(pub Option<T>);

impl<T> Default for Obj<T> {
    fn default() -> Self {
        Self(None)
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Obj<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<T>(std::marker::PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for V<T> {
            type Value = Obj<T>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("any value")
            }
            fn visit_map<A: MapAccess<'de>>(self, m: A) -> Result<Obj<T>, A::Error> {
                T::deserialize(MapAccessDeserializer::new(m)).map(|t| Obj(Some(t)))
            }
            skip!(bool);
            skip!(num);
            skip!(str);
            skip!(null);
            skip!(seq);
        }
        d.deserialize_any(V(std::marker::PhantomData))
    }
}

/// An array field. A value of another type reads as absent; an element of another type than
/// `T` reads as `T::default()` when `T` is lenient itself.
#[derive(Debug, Clone, PartialEq)]
pub struct Arr<T>(pub Option<Vec<T>>);

impl<T> Default for Arr<T> {
    fn default() -> Self {
        Self(None)
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Arr<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<T>(std::marker::PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for V<T> {
            type Value = Arr<T>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("any value")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, s: A) -> Result<Arr<T>, A::Error> {
                Vec::deserialize(SeqAccessDeserializer::new(s)).map(|v| Arr(Some(v)))
            }
            skip!(bool);
            skip!(num);
            skip!(str);
            skip!(null);
            skip!(map);
        }
        d.deserialize_any(V(std::marker::PhantomData))
    }
}

/// The `content` of a message: a string, or a list of blocks.
#[derive(Debug, Default, Clone, PartialEq)]
pub enum Content<'a> {
    #[default]
    None,
    Text(Cow<'a, str>),
    Blocks(Vec<Block<'a>>),
}

impl<'a> Content<'a> {
    pub fn blocks(&self) -> &[Block<'a>] {
        match self {
            Content::Blocks(b) => b,
            _ => &[],
        }
    }

    /// The text blocks joined with `sep`; a plain string as is; "" for anything else.
    pub fn text(&self, sep: &str) -> String {
        match self {
            Content::Text(s) => s.to_string(),
            Content::Blocks(b) => b
                .iter()
                .filter(|x| x.kind.as_str() == Some("text"))
                .map(|x| x.text.or_empty())
                .collect::<Vec<_>>()
                .join(sep),
            Content::None => String::new(),
        }
    }
}

impl<'de: 'a, 'a> Deserialize<'de> for Content<'a> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<'a>(std::marker::PhantomData<&'a ()>);
        impl<'de: 'a, 'a> Visitor<'de> for V<'a> {
            type Value = Content<'a>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("any value")
            }
            fn visit_borrowed_str<E: de::Error>(self, s: &'de str) -> Result<Content<'a>, E> {
                Ok(Content::Text(Cow::Borrowed(s)))
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<Content<'a>, E> {
                Ok(Content::Text(Cow::Owned(s.to_owned())))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut s: A) -> Result<Content<'a>, A::Error> {
                let mut blocks = Vec::new();
                while let Some(b) = s.next_element::<Obj<Block<'a>>>()? {
                    blocks.push(b.0.unwrap_or_default());
                }
                Ok(Content::Blocks(blocks))
            }
            skip!(bool);
            skip!(num);
            skip!(null);
            skip!(map);
        }
        d.deserialize_any(V(std::marker::PhantomData))
    }
}

/// The `content` of a tool result: a string, or any other JSON kept whole.
#[derive(Debug, Clone, PartialEq)]
pub enum Body<'a> {
    Text(Cow<'a, str>),
    Json(Value),
}

impl Default for Body<'_> {
    fn default() -> Self {
        Self::Json(Value::Null)
    }
}

impl<'de: 'a, 'a> Deserialize<'de> for Body<'a> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<'a>(std::marker::PhantomData<&'a ()>);
        impl<'de: 'a, 'a> Visitor<'de> for V<'a> {
            type Value = Body<'a>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("any value")
            }
            fn visit_borrowed_str<E: de::Error>(self, s: &'de str) -> Result<Body<'a>, E> {
                Ok(Body::Text(Cow::Borrowed(s)))
            }
            fn visit_str<E: de::Error>(self, s: &str) -> Result<Body<'a>, E> {
                Ok(Body::Text(Cow::Owned(s.to_owned())))
            }
            fn visit_bool<E: de::Error>(self, b: bool) -> Result<Body<'a>, E> {
                Ok(Body::Json(Value::Bool(b)))
            }
            fn visit_i64<E: de::Error>(self, n: i64) -> Result<Body<'a>, E> {
                Ok(Body::Json(n.into()))
            }
            fn visit_u64<E: de::Error>(self, n: u64) -> Result<Body<'a>, E> {
                Ok(Body::Json(n.into()))
            }
            fn visit_f64<E: de::Error>(self, n: f64) -> Result<Body<'a>, E> {
                Ok(Body::Json(Value::from(n)))
            }
            fn visit_unit<E: de::Error>(self) -> Result<Body<'a>, E> {
                Ok(Body::Json(Value::Null))
            }
            fn visit_none<E: de::Error>(self) -> Result<Body<'a>, E> {
                Ok(Body::Json(Value::Null))
            }
            fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Body<'a>, D::Error> {
                Body::deserialize(d)
            }
            fn visit_seq<A: SeqAccess<'de>>(self, s: A) -> Result<Body<'a>, A::Error> {
                Value::deserialize(SeqAccessDeserializer::new(s)).map(Body::Json)
            }
            fn visit_map<A: MapAccess<'de>>(self, m: A) -> Result<Body<'a>, A::Error> {
                Value::deserialize(MapAccessDeserializer::new(m)).map(Body::Json)
            }
        }
        d.deserialize_any(V(std::marker::PhantomData))
    }
}

// --- the transcript line ---------------------------------------------------------------------

/// One block of a message: text, a tool call or a tool result. A field a kind of block does not
/// have is absent. The input of a tool call is kept whole, because each tool shapes it alike.
#[derive(Debug, Default, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Block<'a> {
    #[serde(borrow, rename = "type")]
    pub kind: Str<'a>,
    #[serde(borrow)]
    pub text: Str<'a>,
    #[serde(borrow)]
    pub name: Str<'a>,
    #[serde(borrow)]
    pub id: Str<'a>,
    pub input: Value,
    #[serde(borrow)]
    pub tool_use_id: Str<'a>,
    pub is_error: Truthy,
    #[serde(borrow)]
    pub content: Body<'a>,
}

/// The token counts of an assistant message.
#[derive(Debug, Default, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Usage {
    pub input_tokens: Num,
    pub output_tokens: Num,
    pub cache_creation_input_tokens: Num,
    pub cache_read_input_tokens: Num,
    pub cache_creation: Obj<CacheCreation>,
}

impl Usage {
    /// True when the object has none of the four counts (an empty `usage` object).
    pub fn is_empty(&self) -> bool {
        !(self.input_tokens.present
            || self.output_tokens.present
            || self.cache_creation_input_tokens.present
            || self.cache_read_input_tokens.present)
    }
}

/// The cache writes of a message by time to live.
#[derive(Debug, Default, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct CacheCreation {
    pub ephemeral_5m_input_tokens: Num,
    pub ephemeral_1h_input_tokens: Num,
}

/// The `message` of a user or assistant line.
#[derive(Debug, Default, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Message<'a> {
    #[serde(borrow)]
    pub id: Str<'a>,
    #[serde(borrow)]
    pub model: Str<'a>,
    pub usage: Obj<Usage>,
    #[serde(borrow)]
    pub content: Content<'a>,
}

/// `compactMetadata` of a compact boundary.
#[derive(Debug, Default, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Compact {
    #[serde(rename = "preTokens")]
    pub pre_tokens: Value,
}

/// `attachment` of an attachment line: its kind and the tool names it added.
#[derive(Debug, Default, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Attachment<'a> {
    #[serde(rename = "type")]
    pub kind: Value,
    #[serde(borrow, rename = "addedNames")]
    pub added_names: Arr<Str<'a>>,
}

/// `origin` of a user line: who wrote it.
#[derive(Debug, Default, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Origin {
    pub kind: Value,
}

/// One line of a transcript. Fields not listed here (the big tool outputs among them) are
/// skipped without being copied.
#[derive(Debug, Default, Clone, PartialEq, Deserialize)]
#[serde(default)]
pub struct Line<'a> {
    #[serde(borrow, rename = "type")]
    pub kind: Str<'a>,
    #[serde(borrow)]
    pub subtype: Str<'a>,
    #[serde(borrow)]
    pub timestamp: Stamp<'a>,
    #[serde(borrow)]
    pub cwd: Str<'a>,
    pub version: Value,
    /// Text of a system line.
    #[serde(borrow)]
    pub content: Str<'a>,
    #[serde(rename = "isSidechain")]
    pub is_sidechain: Truthy,
    #[serde(rename = "isMeta")]
    pub is_meta: Truthy,
    #[serde(rename = "isCompactSummary")]
    pub is_compact_summary: Truthy,
    #[serde(rename = "isApiErrorMessage")]
    pub is_api_error: Truthy,
    #[serde(rename = "thinkingDurationMs")]
    pub thinking_ms: Num,
    #[serde(rename = "compactMetadata")]
    pub compact: Obj<Compact>,
    #[serde(borrow)]
    pub attachment: Obj<Attachment<'a>>,
    #[serde(borrow)]
    pub message: Obj<Message<'a>>,
    pub origin: Obj<Origin>,
}

impl<'a> Line<'a> {
    /// The line, or None when it is not a JSON object.
    pub fn parse(line: &'a [u8]) -> Option<Self> {
        if line.iter().find(|b| !b.is_ascii_whitespace()) != Some(&b'{') {
            return None;
        }
        serde_json::from_slice(line).ok()
    }

    /// Like `parse` for text.
    pub fn parse_str(line: &'a str) -> Option<Self> {
        Self::parse(line.as_bytes())
    }

    /// The message, or an empty one.
    pub fn msg(&self) -> &Message<'a> {
        static NONE: Message<'static> =
            Message { id: Str(None), model: Str(None), usage: Obj(None), content: Content::None };
        self.message.0.as_ref().unwrap_or(&NONE)
    }
}

// --- reading from the end --------------------------------------------------------------------

/// Newest-first lines of a file, read backwards in chunks, so a caller that stops early never
/// reads the rest of the file. Blank lines are skipped.
pub struct RevLines {
    f: fs::File,
    pos: u64,
    chunk: u64,
    buf: Vec<u8>,
    pending: Vec<Vec<u8>>, // forward order; pop() yields the newest
    finished: bool,
}

/// The lines of `path`, newest first.
pub fn rev_lines(path: &Path) -> io::Result<RevLines> {
    let mut f = fs::File::open(path)?;
    let len = f.seek(SeekFrom::End(0))?;
    Ok(RevLines { f, pos: len, chunk: 1 << 16, buf: Vec::new(), pending: Vec::new(), finished: false })
}

fn is_blank(b: &[u8]) -> bool {
    b.iter().all(|c| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c))
}

impl Iterator for RevLines {
    type Item = Vec<u8>;

    fn next(&mut self) -> Option<Vec<u8>> {
        loop {
            if let Some(l) = self.pending.pop() {
                return Some(l);
            }
            if self.pos == 0 {
                if self.finished {
                    return None;
                }
                self.finished = true;
                let b = std::mem::take(&mut self.buf);
                return if is_blank(&b) { None } else { Some(b) };
            }
            let n = self.chunk.min(self.pos);
            self.pos -= n;
            let mut data = vec![0u8; n as usize];
            if self.f.seek(SeekFrom::Start(self.pos)).is_err() || self.f.read_exact(&mut data).is_err() {
                self.pos = 0;
                self.buf.clear();
                continue;
            }
            data.extend_from_slice(&self.buf);
            let parts: Vec<&[u8]> = data.split(|&b| b == b'\n').collect();
            if parts.len() == 1 {
                self.chunk *= 2; // one giant line: read bigger pieces
            }
            self.buf = parts[0].to_vec();
            self.pending = parts[1..].iter().filter(|l| !is_blank(l)).map(|l| l.to_vec()).collect();
        }
    }
}

fn has(hay: &[u8], needle: &[u8]) -> bool {
    memchr::memmem::find(hay, needle).is_some()
}

/// The context size of the newest assistant record with usage: input plus cache tokens.
/// 0 when a compact comes first, when the file cannot be read, or when no record has usage.
pub fn context_size(path: &Path) -> i64 {
    let Ok(lines) = rev_lines(path) else { return 0 };
    for line in lines {
        let compact = has(&line, b"compact_boundary") || has(&line, b"isCompactSummary");
        let assistant = has(&line, b"\"assistant\"");
        if !compact && !assistant {
            continue;
        }
        let Some(rec) = Line::parse(&line) else { continue };
        if compact && (rec.subtype.as_str() == Some("compact_boundary") || rec.is_compact_summary.0) {
            return 0;
        }
        if !assistant || rec.kind.as_str() != Some("assistant") {
            continue;
        }
        let Some(usage) = rec.msg().usage.0.as_ref() else { continue };
        let total =
            usage.input_tokens.get() + usage.cache_creation_input_tokens.get() + usage.cache_read_input_tokens.get();
        if total > 0.0 {
            return total as i64;
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write as _;

    #[test]
    fn line_reads_an_assistant_turn() {
        let raw = r#"{"type":"assistant","timestamp":"2026-10-01T10:00:00Z","isSidechain":false,"message":{"id":"m1","model":"claude-opus-4","usage":{"input_tokens":3,"cache_read_input_tokens":10,"cache_creation":{"ephemeral_5m_input_tokens":2}},"content":[{"type":"text","text":"hi"},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}]},"big":{"x":[1,2,3]}}"#;
        let l = Line::parse_str(raw).unwrap();
        assert_eq!(l.kind.as_str(), Some("assistant"));
        assert_eq!(l.timestamp.as_str(), Some("2026-10-01T10:00:00Z"));
        assert!(!l.is_sidechain.0);
        let m = l.msg();
        assert_eq!(m.id.as_str(), Some("m1"));
        let u = m.usage.0.as_ref().unwrap();
        assert_eq!(u.input_tokens.get(), 3.0);
        assert!(!u.output_tokens.present);
        let cc = u.cache_creation.0.as_ref().unwrap();
        assert!(cc.ephemeral_5m_input_tokens.present && !cc.ephemeral_1h_input_tokens.present);
        let b = m.content.blocks();
        assert_eq!(b.len(), 2);
        assert_eq!(b[1].name.as_str(), Some("Bash"));
        assert_eq!(b[1].input["command"], "ls");
        assert_eq!(m.content.text(" "), "hi");
    }

    #[test]
    fn line_treats_a_field_of_the_wrong_type_as_absent() {
        let raw = r#"{"type":5,"timestamp":[1],"cwd":{"a":1},"isMeta":"yes","isSidechain":[],"thinkingDurationMs":"x","message":"text","attachment":3,"origin":[]}"#;
        let l = Line::parse_str(raw).unwrap();
        assert_eq!(l.kind.as_str(), None);
        assert_eq!(l.timestamp, Stamp::None);
        assert_eq!(l.cwd.as_str(), None);
        assert!(l.is_meta.0);
        assert!(!l.is_sidechain.0);
        assert!(l.thinking_ms.present && l.thinking_ms.value.is_none());
        assert!(l.message.0.is_none() && l.attachment.0.is_none() && l.origin.0.is_none());
        assert!(l.msg().usage.0.is_none());
    }

    #[test]
    fn line_keeps_a_tool_result_body() {
        let raw = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":[{"type":"text","text":"boom"}]},{"type":"tool_result","tool_use_id":"t2","content":"plain"},5]}}"#;
        let l = Line::parse_str(raw).unwrap();
        let b = l.msg().content.blocks().to_vec();
        assert_eq!(b.len(), 3);
        assert!(b[0].is_error.0);
        assert!(matches!(&b[0].content, Body::Json(v) if v[0]["text"] == "boom"));
        assert!(matches!(&b[1].content, Body::Text(s) if s == "plain"));
        assert_eq!(b[2], Block::default());
    }

    #[test]
    fn line_rejects_what_is_not_an_object() {
        for raw in ["", "[1,2]", "\"x\"", "7", "{\"type\":", "not json"] {
            assert!(Line::parse_str(raw).is_none(), "{raw}");
        }
        assert!(Line::parse_str(" {\"type\":\"user\"}\n").is_some());
    }

    #[test]
    fn line_escaped_strings_are_owned_and_plain_ones_borrowed() {
        let l = Line::parse_str(r#"{"type":"a\nb","cwd":"/x"}"#).unwrap();
        assert!(matches!(l.kind.0, Some(Cow::Owned(_))));
        assert!(matches!(l.cwd.0, Some(Cow::Borrowed(_))));
    }

    #[test]
    fn rev_lines_is_newest_first_and_lazy() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.txt");
        let mut body = String::new();
        for i in 0..20000 {
            writeln!(body, "line{i}\n").unwrap();
        }
        fs::write(&p, body).unwrap();
        let mut it = rev_lines(&p).unwrap();
        assert_eq!(it.next().as_deref(), Some(&b"line19999"[..]));
        assert_eq!(it.next().as_deref(), Some(&b"line19998"[..]));
        assert_eq!(it.count(), 19998);
    }
}
