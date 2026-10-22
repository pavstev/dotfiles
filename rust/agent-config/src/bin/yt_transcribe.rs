//! yt_transcribe: get a YouTube transcript. Captions first (yt-dlp), local Whisper
//! (whisper.cpp) when there are none. Ported from a private Python tool; the CLI,
//! the link parser and the output formats match it.
//!
//! Needs on PATH: `yt-dlp` always; `ffmpeg` and `whisper-cli` for the Whisper fallback;
//! `curl` once, to fetch the Whisper model into the user cache folder.

use agent_config::pycompat::{py_is_space, py_repr_str as py_repr, py_strip};
use clap::{ArgAction, ArgGroup, Parser};
use regex::Regex;
use serde_json::{Map, Value};
use std::fmt::Write as _;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::LazyLock;

const DOC: &str = "Get a YouTube transcript. Uses captions; falls back to local Whisper.";

#[derive(Parser)]
#[command(name = "yt_transcribe", about = DOC)]
#[command(group(ArgGroup::new("mode").args(["force_whisper", "no_whisper"])))]
struct Cli {
    /// YouTube URL or 11-character video ID
    video: String,
    /// output format
    #[arg(short = 'f', long = "format", default_value = "text", value_parser = ["text", "json", "markdown"])]
    format: String,
    /// preferred language (repeatable), default en
    #[arg(short = 'l', long = "lang", action = ArgAction::Append)]
    lang: Vec<String>,
    /// Whisper model: tiny, base, small... (default base)
    #[arg(short = 'm', long = "model", default_value = "base")]
    model: String,
    /// write to this file instead of stdout
    #[arg(short = 'o', long = "output")]
    output: Option<String>,
    /// skip captions, always use Whisper
    #[arg(long = "force-whisper")]
    force_whisper: bool,
    /// captions only, never download audio
    #[arg(long = "no-whisper")]
    no_whisper: bool,
    /// print the video ID and exit (no network)
    #[arg(long = "print-id")]
    print_id: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct Segment {
    start: f64,
    duration: f64,
    text: String,
}

#[derive(Debug, Clone, PartialEq)]
struct Transcript {
    video_id: String,
    /// "captions" or "whisper"
    source: &'static str,
    language: Option<String>,
    segments: Vec<Segment>,
}

/// A captions failure: Fatal stops the run, Lookup lets Whisper take over.
#[derive(Debug, PartialEq)]
enum CapErr {
    Fatal(String),
    Lookup(String),
}

// ---------------------------------------------------------------------------
// Python-compat helpers (str.strip, urllib.parse, repr)
// ---------------------------------------------------------------------------

/// `" ".join(s.split())`
fn collapse_ws(s: &str) -> String {
    s.split(py_is_space).filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" ")
}

/// `urllib.parse.unquote`: percent escapes decode as UTF-8, bad bytes become U+FFFD.
fn unquote(s: &str) -> String {
    if !s.contains('%') {
        return s.to_string();
    }
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() && b[i + 1].is_ascii_hexdigit() && b[i + 2].is_ascii_hexdigit() {
            let h = |c: u8| (c as char).to_digit(16).unwrap() as u8;
            out.push(h(b[i + 1]) * 16 + h(b[i + 2]));
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ---------------------------------------------------------------------------
// ids: pull the 11-character video ID out of any YouTube link form (pure, no network)
// ---------------------------------------------------------------------------

static URL_IN_TEXT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)(?:https?:)?//[^\s<>"'`]+|(?:[\w-]+\.)+[a-z]{2,}/[^\s<>"'`]*"#).unwrap());
static PCT_3F: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)%3F").unwrap());

const YT_DOMAINS: [&str; 3] = ["youtube.com", "youtube-nocookie.com", "youtubekids.com"];
const SHORT_HOSTS: [&str; 2] = ["youtu.be", "yt.be"];
const IMG_HOSTS: [&str; 8] = [
    "img.youtube.com",
    "i.ytimg.com",
    "i1.ytimg.com",
    "i2.ytimg.com",
    "i3.ytimg.com",
    "i4.ytimg.com",
    "i9.ytimg.com",
    "s.ytimg.com",
];
const ID_PATH_PREFIXES: [&str; 8] = ["embed", "shorts", "live", "v", "e", "vi", "vi_webp", "watch"];
/// Query keys that carry another link (share redirects, oembed, login bounce).
const NESTED_KEYS: [&str; 7] = ["u", "q", "url", "continue", "next", "next_url", "redirect_uri"];
const MAX_DEPTH: u32 = 4;
/// Path words that are 11 characters long but are not video IDs.
const NOT_IDS: [&str; 2] = ["videoseries", "live_stream"];
/// Schemes for which `urlparse` splits `;params` off the path.
const USES_PARAMS: [&str; 15] = [
    "", "ftp", "hdl", "prospero", "http", "imap", "https", "shttp", "rtsp", "rtspu", "sip", "sips", "mms", "sftp",
    "tel",
];

/// `^[A-Za-z0-9_-]{11}$` the Python way: `$` also matches before one final newline.
fn id_match(c: &str) -> bool {
    let c = c.strip_suffix('\n').unwrap_or(c);
    c.len() == 11 && c.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn host_ok(host: &str, domain: &str) -> bool {
    host == domain || host.strip_suffix(domain).is_some_and(|p| p.ends_with('.'))
}

fn clean(raw: &str) -> String {
    let mut s = py_strip(py_strip(raw).trim_matches(|c| "<>\"'`".contains(c))).to_string();
    if !s.contains("://") && s.to_uppercase().contains("%3A%2F%2F") {
        s = unquote(&s);
    }
    s
}

struct Parsed {
    host: String,
    path: String,
    query: String,
    fragment: String,
}

/// `urllib.parse.urlparse` plus `.hostname`. None where Python raises ValueError.
fn urlparse(url: &str) -> Option<Parsed> {
    let url: String =
        url.trim_start_matches(|c: char| c <= ' ').chars().filter(|c| !matches!(c, '\t' | '\r' | '\n')).collect();
    let mut rest: &str = &url;
    let mut scheme = String::new();
    if let Some(i) = rest.find(':') {
        let first_ok = rest.chars().next().is_some_and(|c| c.is_ascii_alphabetic());
        if i > 0 && first_ok && rest[..i].chars().all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c)) {
            scheme = rest[..i].to_lowercase();
            rest = &rest[i + 1..];
        }
    }
    let mut netloc = "";
    if rest.starts_with("//") {
        let end = rest[2..].find(['/', '?', '#']).map_or(rest.len(), |p| p + 2);
        netloc = &rest[2..end];
        rest = &rest[end..];
        let open = netloc.contains('[');
        let close = netloc.contains(']');
        if open != close {
            return None;
        }
        if open {
            let inner = netloc.split_once('[').map_or("", |(_, r)| r.split(']').next().unwrap_or(""));
            if !inner.starts_with('v') && inner.parse::<std::net::Ipv6Addr>().is_err() {
                return None;
            }
        }
    }
    let mut fragment = String::new();
    let mut tail = rest.to_string();
    if let Some((a, b)) = rest.split_once('#') {
        tail = a.to_string();
        fragment = b.to_string();
    }
    let mut query = String::new();
    if let Some((a, b)) = tail.clone().split_once('?') {
        tail = a.to_string();
        query = b.to_string();
    }
    if USES_PARAMS.contains(&scheme.as_str()) && tail.contains(';') {
        let i = match tail.rfind('/') {
            Some(slash) => tail[slash..].find(';').map(|p| p + slash),
            None => tail.find(';'),
        };
        if let Some(i) = i {
            tail.truncate(i);
        }
    }
    // .hostname
    let hostinfo = netloc.rsplit_once('@').map_or(netloc, |(_, h)| h);
    let host = if let Some((_, br)) = hostinfo.split_once('[') {
        br.split(']').next().unwrap_or("").to_string()
    } else {
        hostinfo.split(':').next().unwrap_or("").to_string()
    };
    let host = match host.split_once('%') {
        Some((h, z)) => format!("{}%{}", h.to_lowercase(), z),
        None => host.to_lowercase(),
    };
    Some(Parsed { host, path: tail, query, fragment })
}

/// `urllib.parse.parse_qs`: decoded (key, value) pairs in order; blank values are dropped.
fn parse_qs(q: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for nv in q.split('&') {
        let Some((k, v)) = nv.split_once('=') else { continue };
        if v.is_empty() {
            continue;
        }
        out.push((unquote(&k.replace('+', " ")), unquote(&v.replace('+', " "))));
    }
    out
}

fn qget<'a>(q: &'a [(String, String)], key: &str) -> Vec<&'a str> {
    q.iter().filter(|(k, _)| k == key).map(|(_, v)| v.as_str()).collect()
}

fn ok_id(c: Option<&str>) -> Option<String> {
    c.filter(|c| id_match(c) && !NOT_IDS.contains(c)).map(str::to_string)
}

fn from_url(value: &str, depth: u32) -> Option<String> {
    if depth > MAX_DEPTH {
        return None;
    }
    let mut value = value.to_string();
    if value.starts_with('/') && !value.starts_with("//") {
        value = format!("//www.youtube.com{value}");
    }
    if PCT_3F.is_match(value.split('?').next().unwrap_or("")) {
        value = unquote(&value);
    }
    let s = if value.contains("://") || value.starts_with("//") { value } else { format!("//{value}") };
    let u = urlparse(&s)?;
    let host = u.host.to_lowercase();
    let host = host.trim_end_matches('.');
    if host.is_empty() {
        return None;
    }
    let parts: Vec<String> = u.path.split('/').filter(|p| !p.is_empty()).map(unquote).collect();
    let query = parse_qs(&u.query);
    let mut frag = u.fragment.as_str();
    if let Some(f) = frag.strip_prefix('!') {
        frag = f; // old "#!/watch?v=ID" style
    }
    if frag.starts_with('/') || frag.contains("v=") {
        let tail =
            if frag.starts_with('/') { frag.to_string() } else { format!("/watch?{}", frag.trim_start_matches('?')) };
        if let Some(inner) = from_url(&format!("//{host}{tail}"), depth + 1) {
            return Some(inner);
        }
    }
    if SHORT_HOSTS.contains(&host) {
        return ok_id(parts.first().map(String::as_str));
    }
    if IMG_HOSTS.contains(&host) {
        return if parts.len() >= 2 && (parts[0] == "vi" || parts[0] == "vi_webp") {
            ok_id(Some(&parts[1]))
        } else {
            None
        };
    }
    let is_yt = YT_DOMAINS.iter().any(|d| host_ok(host, d));
    if is_yt {
        if let Some(got) = ok_id(qget(&query, "v").first().copied()) {
            return Some(got);
        }
        if !parts.is_empty() && ID_PATH_PREFIXES.contains(&parts[0].as_str()) && parts.len() >= 2 {
            return ok_id(Some(&parts[1]));
        }
        if parts.len() == 1 && parts[0] == "embed" {
            return ok_id(qget(&query, "v").first().copied());
        }
    }
    // Nested link: attribution_link, redirect, oembed, ServiceLogin and similar.
    if is_yt || host == "google.com" || host.ends_with(".google.com") {
        for key in NESTED_KEYS {
            for inner in qget(&query, key) {
                if let Some(got) = from_url(inner, depth + 1) {
                    return Some(got);
                }
            }
        }
    }
    None
}

/// Return the video ID for a URL or bare ID, or the error text to show.
///
/// Accepts: bare ID; watch, youtu.be, shorts, live, embed, /v/, /e/, /vi/ thumbnails;
/// m., music., gaming., www. and nocookie hosts; http, https, no scheme, //host;
/// ports, extra query keys, playlists with v=, #! fragments, percent-encoded links,
/// share redirects (attribution_link, redirect, oembed), and a link inside pasted text.
/// Rejects: playlist-only, channel, clip and other non-video links (no guessing).
fn extract_video_id(value: &str) -> Result<String, String> {
    let text = clean(value);
    if id_match(&text) {
        return Ok(text);
    }
    let mut got = from_url(&text, 0);
    if got.is_none() {
        // A link inside a longer pasted line: try each URL-looking piece, first match wins.
        for m in URL_IN_TEXT.find_iter(value) {
            let piece = clean(m.as_str());
            let piece = piece.trim_end_matches(['.', ',', ';', ':', '!', '?', ')']);
            got = from_url(piece, 0);
            if got.is_some() {
                break;
            }
        }
    }
    if let Some(id) = got {
        return Ok(id);
    }
    let low = text.to_lowercase();
    let hint = if low.contains("list=") && !low.contains("v=") {
        " That link points to a playlist, not one video."
    } else if low.contains("/clip/") {
        " Clip links hide the video ID. Open the clip and copy the full video link."
    } else {
        ""
    };
    Err(format!(
        "Cannot find a YouTube video ID in {}.{hint} Pass a video link or the 11-character ID.",
        py_repr(value)
    ))
}

// ---------------------------------------------------------------------------
// formats
// ---------------------------------------------------------------------------

fn stamp(seconds: f64) -> String {
    let s = seconds as u64;
    let (h, rem) = (s / 3600, s % 3600);
    let (m, sec) = (rem / 60, rem % 60);
    if h > 0 { format!("{h:02}:{m:02}:{sec:02}") } else { format!("{m:02}:{sec:02}") }
}

/// Python `round(x, 2)`: round the exact binary value to 2 decimals.
fn round2(x: f64) -> f64 {
    format!("{x:.2}").parse().unwrap_or(x)
}

fn render(t: &Transcript, fmt: &str) -> Result<String, String> {
    match fmt {
        "text" => Ok(t.segments.iter().map(|s| s.text.as_str()).collect::<Vec<_>>().join("\n")),
        "json" => {
            let segs: Vec<Value> = t
                .segments
                .iter()
                .map(|s| {
                    let mut m = Map::new();
                    m.insert("start".into(), Value::from(round2(s.start)));
                    m.insert("duration".into(), Value::from(round2(s.duration)));
                    m.insert("text".into(), Value::from(s.text.clone()));
                    Value::Object(m)
                })
                .collect();
            let mut m = Map::new();
            m.insert("video_id".into(), Value::from(t.video_id.clone()));
            m.insert("source".into(), Value::from(t.source));
            m.insert("language".into(), t.language.clone().map_or(Value::Null, Value::from));
            m.insert("segments".into(), Value::Array(segs));
            serde_json::to_string_pretty(&Value::Object(m)).map_err(|e| e.to_string())
        }
        "markdown" => {
            let mut head = format!("# Transcript: {}\n\nSource: {}", t.video_id, t.source);
            if let Some(l) = t.language.as_deref().filter(|l| !l.is_empty()) {
                let _ = write!(head, " ({l})");
            }
            let lines: Vec<String> =
                t.segments.iter().map(|s| format!("- **[{}]** {}", stamp(s.start), s.text)).collect();
            Ok(format!("{head}\n\n{}", lines.join("\n")))
        }
        other => Err(format!("unknown format: {other}")),
    }
}

// ---------------------------------------------------------------------------
// tools on PATH
// ---------------------------------------------------------------------------

fn find_tool(name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    let exts: &[&str] = if cfg!(windows) { &["", ".exe", ".cmd", ".bat"] } else { &[""] };
    for dir in std::env::split_paths(&paths) {
        for ext in exts {
            let p = dir.join(format!("{name}{ext}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// The install line for this OS.
fn install_hint(tool: &str) -> String {
    let win = cfg!(windows);
    match tool {
        "yt-dlp" if win => "winget install yt-dlp.yt-dlp".into(),
        "yt-dlp" => "run `dots` (it installs yt-dlp from tools.list)".into(),
        "ffmpeg" if win => "winget install Gyan.FFmpeg".into(),
        "ffmpeg" => "run `dots` (it installs ffmpeg from tools.list)".into(),
        "whisper-cli" if win => {
            "download whisper-bin-x64.zip from https://github.com/ggml-org/whisper.cpp/releases and put whisper-cli.exe on PATH".into()
        }
        "whisper-cli" => "run `dots` (it installs whisper-cpp from tools.list; Alpine has no build)".into(),
        other => format!("install {other}"),
    }
}

fn missing_tool(tool: &str, why: &str) -> String {
    format!("{tool} is not installed, and {why} needs it. Install it: {}.", install_hint(tool))
}

fn need_tool(tool: &str, why: &str, finder: &dyn Fn(&str) -> Option<PathBuf>) -> Result<PathBuf, String> {
    finder(tool).ok_or_else(|| missing_tool(tool, why))
}

// ---------------------------------------------------------------------------
// captions via yt-dlp
// ---------------------------------------------------------------------------

const MSG_UNAVAILABLE: &str = "Video is unavailable (removed, private, or the link is wrong).";
const MSG_UNPLAYABLE: &str = "Video cannot be played (private, members-only, or removed).";
const MSG_AGE: &str = "Video is age-restricted. Captions need a signed-in session; try cookies with yt-dlp.";
// Captions and the Whisper audio both come through yt-dlp, so a block stops both.
const MSG_BLOCKED: &str = "YouTube blocked this IP (bot check or too many requests). \
     Wait an hour and try again, or open the video in a browser and read its captions there.";
const MSG_SIGNIN: &str = "YouTube asks for sign-in (age or bot check). Try again later.";

/// Map yt-dlp's stderr to the same user messages the Python tool gave.
fn map_ytdlp_error(stderr: &str) -> Option<&'static str> {
    let low = stderr.to_lowercase();
    let has = |ks: &[&str]| ks.iter().any(|k| low.contains(k));
    if has(&["private video", "members-only", "members only", "join this channel"]) {
        Some(MSG_UNPLAYABLE)
    } else if has(&["confirm your age", "age-restricted", "age restricted"]) {
        Some(MSG_AGE)
    } else if has(&["not a bot", "http error 429", "too many requests", "blocked"]) {
        Some(MSG_BLOCKED)
    } else if has(&[
        "video unavailable",
        "this video is not available",
        "has been removed",
        "terminated",
        "does not exist",
        "is unavailable",
    ]) {
        Some(MSG_UNAVAILABLE)
    } else if has(&["sign in"]) {
        Some(MSG_SIGNIN)
    } else {
        None
    }
}

/// The last `ERROR:` line of yt-dlp's stderr, else its last line.
fn ytdlp_message(stderr: &str) -> String {
    let lines: Vec<&str> = stderr.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    lines
        .iter()
        .rev()
        .find(|l| l.starts_with("ERROR:"))
        .or(lines.last())
        .map_or_else(|| "yt-dlp failed".into(), |l| l.trim_start_matches("ERROR:").trim().to_string())
}

/// Parse a json3 caption file: `events[].tStartMs`, `dDurationMs`, `segs[].utf8`.
/// Skips `aAppend` events and events that are empty or newline-only.
fn parse_json3(text: &str) -> Result<Vec<Segment>, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("bad caption file: {e}"))?;
    let mut out = Vec::new();
    for ev in v.get("events").and_then(Value::as_array).into_iter().flatten() {
        if ev.get("aAppend").is_some() {
            continue;
        }
        let Some(segs) = ev.get("segs").and_then(Value::as_array) else { continue };
        let raw: String = segs.iter().filter_map(|s| s.get("utf8").and_then(Value::as_str)).collect();
        let text = collapse_ws(&raw);
        if text.is_empty() {
            continue;
        }
        let ms = |k: &str| ev.get(k).and_then(Value::as_f64).unwrap_or(0.0);
        out.push(Segment { start: ms("tStartMs") / 1000.0, duration: ms("dDurationMs") / 1000.0, text });
    }
    Ok(out)
}

fn watch_url(video_id: &str) -> String {
    format!("https://www.youtube.com/watch?v={video_id}")
}

fn from_captions(video_id: &str, langs: &[String]) -> Result<Transcript, CapErr> {
    let ytdlp = need_tool("yt-dlp", "captions", &find_tool).map_err(CapErr::Fatal)?;
    let dir = tempfile::Builder::new()
        .prefix("yt-transcribe-")
        .tempdir()
        .map_err(|e| CapErr::Fatal(format!("Cannot make a temp folder: {e}")))?;
    let out = Command::new(&ytdlp)
        .args(["--skip-download", "--write-subs", "--write-auto-subs", "--sub-format", "json3"])
        .args(["--no-playlist", "--no-warnings", "--no-progress", "--quiet"])
        .arg("--sub-langs")
        .arg(langs.join(","))
        .arg("-o")
        .arg(dir.path().join("%(id)s.%(ext)s"))
        .arg(watch_url(video_id))
        .stdin(Stdio::null())
        .output()
        .map_err(|e| CapErr::Fatal(format!("Cannot run yt-dlp: {e}")))?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        if let Some(msg) = map_ytdlp_error(&stderr) {
            return Err(CapErr::Fatal(msg.to_string()));
        }
        return Err(CapErr::Lookup(ytdlp_message(&stderr)));
    }
    // Pick the first preferred language that has a track. yt-dlp already prefers a
    // manual track over an auto track for the same language.
    for lang in langs {
        let path = dir.path().join(format!("{video_id}.{lang}.json3"));
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let segments = parse_json3(&text).map_err(CapErr::Lookup)?;
        if segments.is_empty() {
            continue;
        }
        return Ok(Transcript {
            video_id: video_id.to_string(),
            source: "captions",
            language: Some(lang.clone()),
            segments,
        });
    }
    Err(CapErr::Lookup(format!("no caption track for {}", langs.join(", "))))
}

// ---------------------------------------------------------------------------
// Whisper fallback: yt-dlp audio -> ffmpeg 16 kHz mono wav -> whisper-cli
// ---------------------------------------------------------------------------

const MODEL_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

fn valid_model_name(m: &str) -> bool {
    !m.is_empty() && m.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

fn run_checked(cmd: &mut Command, what: &str) -> Result<std::process::Output, String> {
    let out = cmd.stdin(Stdio::null()).output().map_err(|e| format!("Cannot run {what}: {e}"))?;
    if out.status.success() {
        return Ok(out);
    }
    let err = String::from_utf8_lossy(&out.stderr);
    if let Some(msg) = map_ytdlp_error(&err).filter(|_| what == "yt-dlp") {
        return Err(msg.to_string());
    }
    let tail: Vec<&str> = err.lines().rev().filter(|l| !l.trim().is_empty()).take(3).collect();
    let tail = tail.into_iter().rev().collect::<Vec<_>>().join(" | ");
    Err(format!("{what} failed: {tail}"))
}

fn download_audio(ytdlp: &Path, video_id: &str, dir: &Path) -> Result<PathBuf, String> {
    let r = run_checked(
        Command::new(ytdlp)
            .args(["-f", "bestaudio/best", "--no-playlist", "--no-warnings", "--no-progress", "--quiet"])
            .arg("-o")
            .arg(dir.join("%(id)s.%(ext)s"))
            .arg(watch_url(video_id)),
        "yt-dlp",
    );
    if let Err(msg) = r {
        let known = [MSG_UNAVAILABLE, MSG_UNPLAYABLE, MSG_AGE, MSG_BLOCKED, MSG_SIGNIN];
        return Err(if known.contains(&msg.as_str()) { msg } else { format!("Audio download failed: {msg}") });
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let partial = matches!(p.extension().and_then(|x| x.to_str()), Some("part" | "ytdl"));
            n.starts_with(&format!("{video_id}.")) && !partial
        })
        .collect();
    files.sort();
    files.into_iter().next().ok_or_else(|| "Audio download produced no file.".to_string())
}

fn model_path(model: &str) -> Result<PathBuf, String> {
    let base = dirs::cache_dir().ok_or("Cannot find the user cache folder.")?;
    Ok(base.join("yt-transcribe").join("models").join(format!("ggml-{model}.bin")))
}

/// Download `ggml-<model>.bin` once into the user cache folder.
fn ensure_model(model: &str) -> Result<PathBuf, String> {
    if !valid_model_name(model) {
        return Err(format!("Bad Whisper model name {}. Use tiny, base, small, medium or large.", py_repr(model)));
    }
    let path = model_path(model)?;
    if path.metadata().is_ok_and(|m| m.len() > 0) {
        return Ok(path);
    }
    let curl = if cfg!(windows) { "curl.exe" } else { "curl" };
    let curl = find_tool(curl).or_else(|| find_tool("curl")).ok_or_else(|| {
        "curl is not installed, and the Whisper model download needs it. Install curl, or put ggml-<model>.bin in the models folder by hand.".to_string()
    })?;
    let dir = path.parent().ok_or("bad model path")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("Cannot make {}: {e}", dir.display()))?;
    let part = dir.join(format!("ggml-{model}.bin.part"));
    eprintln!("yt_transcribe: downloading the Whisper model ggml-{model}.bin once to {}", dir.display());
    let status = Command::new(&curl)
        .args(["-L", "--fail", "--progress-bar", "-o"])
        .arg(&part)
        .arg(format!("{MODEL_URL}/ggml-{model}.bin"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .status()
        .map_err(|e| format!("Cannot run curl: {e}"))?;
    if !status.success() {
        let _ = std::fs::remove_file(&part);
        return Err(format!(
            "Whisper model download failed. Check the name {} (tiny, base, small, medium, large) and your network.",
            py_repr(model)
        ));
    }
    std::fs::rename(&part, &path).map_err(|e| format!("Cannot save the model: {e}"))?;
    Ok(path)
}

/// Parse whisper.cpp `-oj` output: `transcription[].offsets.{from,to}` (ms) and `.text`.
fn parse_whisper_json(text: &str) -> Result<(Option<String>, Vec<Segment>), String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("bad Whisper output: {e}"))?;
    let lang = v.pointer("/result/language").and_then(Value::as_str).map(str::to_string);
    let mut segs = Vec::new();
    for t in v.get("transcription").and_then(Value::as_array).into_iter().flatten() {
        let text = py_strip(t.get("text").and_then(Value::as_str).unwrap_or("")).to_string();
        if text.is_empty() {
            continue;
        }
        let off = |k: &str| t.pointer(&format!("/offsets/{k}")).and_then(Value::as_f64).unwrap_or(0.0);
        let (from, to) = (off("from"), off("to"));
        segs.push(Segment { start: from / 1000.0, duration: (to - from) / 1000.0, text });
    }
    Ok((lang, segs))
}

fn from_whisper(video_id: &str, model: &str, language: Option<&str>) -> Result<Transcript, String> {
    let ffmpeg = need_tool("ffmpeg", "the Whisper fallback", &find_tool)?;
    let whisper = need_tool_any(&["whisper-cli", "whisper-cpp"], "the Whisper fallback", &find_tool)?;
    let ytdlp = need_tool("yt-dlp", "the Whisper fallback", &find_tool)?;
    let model_file = ensure_model(model)?;
    // The folder and the audio in it are deleted when `tmp` drops, on every path.
    let tmp = tempfile::Builder::new()
        .prefix("yt-transcribe-")
        .tempdir()
        .map_err(|e| format!("Cannot make a temp folder: {e}"))?;
    eprintln!("yt_transcribe: downloading the audio and running Whisper ({model})");
    let audio = download_audio(&ytdlp, video_id, tmp.path())?;
    let wav = tmp.path().join("audio.wav");
    run_checked(
        Command::new(&ffmpeg)
            .args(["-nostdin", "-loglevel", "error", "-y", "-i"])
            .arg(&audio)
            .args(["-ar", "16000", "-ac", "1", "-c:a", "pcm_s16le"])
            .arg(&wav),
        "ffmpeg",
    )?;
    let lang = language.map(|l| l.split(['-', '_']).next().unwrap_or(l).to_lowercase());
    let out_base = tmp.path().join("out");
    run_checked(
        Command::new(&whisper)
            .arg("-m")
            .arg(&model_file)
            .arg("-f")
            .arg(&wav)
            .args(["-oj", "-np", "-l"])
            .arg(lang.as_deref().unwrap_or("auto"))
            .arg("-of")
            .arg(&out_base),
        "whisper-cli",
    )?;
    let json = std::fs::read_to_string(tmp.path().join("out.json"))
        .map_err(|e| format!("Whisper wrote no output file: {e}"))?;
    let (language, segments) = parse_whisper_json(&json)?;
    Ok(Transcript { video_id: video_id.to_string(), source: "whisper", language, segments })
}

fn need_tool_any(names: &[&str], why: &str, finder: &dyn Fn(&str) -> Option<PathBuf>) -> Result<PathBuf, String> {
    names.iter().find_map(|n| finder(n)).ok_or_else(|| missing_tool(names[0], why))
}

// ---------------------------------------------------------------------------
// flow
// ---------------------------------------------------------------------------

type CaptionsFn<'a> = &'a dyn Fn(&str, &[String]) -> Result<Transcript, CapErr>;
type WhisperFn<'a> = &'a dyn Fn(&str, &str, Option<&str>) -> Result<Transcript, String>;

/// Return a transcript: captions first, local Whisper if there are none.
fn transcribe_with(
    video: &str,
    languages: &[String],
    model: &str,
    force_whisper: bool,
    no_whisper: bool,
    captions: CaptionsFn,
    whisper: WhisperFn,
) -> Result<Transcript, String> {
    let vid = extract_video_id(video)?;
    let langs: Vec<String> = if languages.is_empty() { vec!["en".into()] } else { languages.to_vec() };
    if !force_whisper {
        match captions(&vid, &langs) {
            Ok(t) => return Ok(t),
            Err(CapErr::Fatal(m)) => return Err(m),
            Err(CapErr::Lookup(m)) if no_whisper => {
                return Err(format!("No captions found ({m}) and Whisper is off."));
            }
            Err(CapErr::Lookup(_)) => {}
        }
    }
    whisper(&vid, model, languages.first().map(String::as_str))
}

fn fail(msg: &str) -> i32 {
    let _ = writeln!(std::io::stderr(), "error: {msg}");
    1
}

fn run(cli: &Cli) -> i32 {
    if cli.print_id {
        return match extract_video_id(&cli.video) {
            Ok(id) => {
                let _ = writeln!(std::io::stdout(), "{id}");
                0
            }
            Err(e) => fail(&e),
        };
    }
    let t = match transcribe_with(
        &cli.video,
        &cli.lang,
        &cli.model,
        cli.force_whisper,
        cli.no_whisper,
        &from_captions,
        &from_whisper,
    ) {
        Ok(t) => t,
        Err(e) => return fail(&e),
    };
    let out = match render(&t, &cli.format) {
        Ok(o) => o,
        Err(e) => return fail(&e),
    };
    if let Some(path) = &cli.output {
        if let Err(e) = std::fs::write(path, format!("{out}\n")) {
            return fail(&format!("Cannot write {path}: {e}"));
        }
        eprintln!("wrote {path} (source: {})", t.source);
    } else {
        let _ = writeln!(std::io::stdout(), "{out}");
    }
    0
}

const FETCH_USAGE: &str = "usage: yt_transcribe fetch [--force-whisper] <youtube url>";

/// What `fetch` was asked: help, or one link with its flags.
#[derive(Debug, PartialEq)]
enum FetchArgs {
    Help,
    Run { url: String, force_whisper: bool },
}

/// Parse the words after `fetch`. Flags may come before or after the link. Any other word is
/// the link, so an ID that starts with `-` still works. `None`: no link or two links.
fn parse_fetch_args(args: &[String]) -> Option<FetchArgs> {
    let mut url = None;
    let mut force_whisper = false;
    for a in args {
        match a.as_str() {
            "-h" | "--help" => return Some(FetchArgs::Help),
            "--force-whisper" => force_whisper = true,
            "" => return None,
            _ if url.is_some() => return None,
            _ => url = Some(a.clone()),
        }
    }
    url.map(|url| FetchArgs::Run { url, force_whisper })
}

/// `~/.cache/yt-learn`: the cache of `fetch` (HOME first, like the shell script it replaced).
fn fetch_dir() -> PathBuf {
    let home =
        std::env::var_os("HOME").map(PathBuf::from).filter(|p| !p.as_os_str().is_empty()).or_else(dirs::home_dir);
    home.unwrap_or_else(|| PathBuf::from(".")).join(".cache").join("yt-learn")
}

/// `fetch <url>`: print the path of a timestamped markdown transcript, `~/.cache/yt-learn/<id>.md`.
/// A cached, non-empty file is returned at once with no network, with or without `--force-whisper`.
fn fetch(url: &str, force_whisper: bool) -> i32 {
    fetch_with(url, &fetch_dir(), &|id| {
        transcribe_with(id, &[], "base", force_whisper, false, &from_captions, &from_whisper)
            .and_then(|t| render(&t, "markdown"))
    })
}

fn fetch_with(url: &str, dir: &Path, make: &dyn Fn(&str) -> Result<String, String>) -> i32 {
    let id = match extract_video_id(url) {
        Ok(id) => id,
        Err(e) => return fail(&e),
    };
    let out = dir.join(format!("{id}.md"));
    if out.metadata().is_ok_and(|m| m.is_file() && m.len() > 0) {
        println!("{}", out.display());
        return 0;
    }
    if let Err(e) = std::fs::create_dir_all(dir) {
        return fail(&format!("Cannot make {}: {e}", dir.display()));
    }
    let text = match make(&id) {
        Ok(t) => t,
        Err(e) => return fail(&e),
    };
    let tmp = match tempfile::Builder::new().prefix(&format!(".{id}.")).tempfile_in(dir) {
        Ok(t) => t,
        Err(e) => return fail(&format!("Cannot write in {}: {e}", dir.display())),
    };
    if let Err(e) = std::fs::write(tmp.path(), format!("{text}\n")) {
        return fail(&format!("Cannot write {}: {e}", tmp.path().display()));
    }
    if tmp.path().metadata().is_ok_and(|m| m.len() == 0) {
        return fail("transcript came back empty");
    }
    if let Err(e) = tmp.persist(&out) {
        return fail(&format!("Cannot save {}: {e}", out.display()));
    }
    println!("{}", out.display());
    0
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "fetch") {
        match parse_fetch_args(&args[1..]) {
            Some(FetchArgs::Help) => {
                println!("{FETCH_USAGE}");
                std::process::exit(0);
            }
            Some(FetchArgs::Run { url, force_whisper }) => std::process::exit(fetch(&url, force_whisper)),
            None => {
                eprintln!("{FETCH_USAGE}");
                std::process::exit(2);
            }
        }
    }
    std::process::exit(run(&Cli::parse()));
}

#[cfg(test)]
mod tests {
    use super::*;

    const V: &str = "dQw4w9WgXcQ";
    const W: &str = "a-b_C1d2E3f"; // dash and underscore in an ID

    fn good() -> Vec<String> {
        let (v, w) = (V, W);
        vec![
            v.into(),
            format!("  {v}\n"),
            w.into(),
            format!("https://www.youtube.com/watch?v={v}"),
            format!("http://youtube.com/watch?v={v}"),
            format!("youtube.com/watch?v={v}"),
            format!("//www.youtube.com/watch?v={v}"),
            format!("https://WWW.YouTube.COM/watch?v={v}"),
            format!("https://www.youtube.com:443/watch?v={v}"),
            format!("https://www.youtube.com/watch?v={v}&t=3s&feature=share"),
            format!("https://www.youtube.com/watch?feature=share&t=10&v={v}"),
            format!("https://www.youtube.com/watch?v={v}&list=PLabc&index=2"),
            format!("https://www.youtube.com/watch/{v}"),
            format!("https://m.youtube.com/watch?v={v}"),
            format!("https://music.youtube.com/watch?v={v}&si=x"),
            format!("https://gaming.youtube.com/watch?v={v}"),
            format!("https://www.youtube-nocookie.com/embed/{v}"),
            format!("https://www.youtube.com/embed/{v}?start=5"),
            format!("https://www.youtube.com/shorts/{v}"),
            format!("https://youtube.com/shorts/{v}?feature=share"),
            format!("https://www.youtube.com/live/{v}?si=x"),
            format!("https://www.youtube.com/v/{v}"),
            format!("https://www.youtube.com/e/{v}"),
            format!("https://youtu.be/{v}"),
            format!("https://youtu.be/{v}?t=30&si=abc"),
            format!("youtu.be/{v}/"),
            format!("https://yt.be/{v}"),
            format!("https://www.youtube.com/watch?v={v}#t=30"),
            format!("https://www.youtube.com/#!/watch?v={v}"),
            format!("https://www.youtube.com/watch%3Fv%3D{v}"),
            format!("https%3A%2F%2Fwww.youtube.com%2Fwatch%3Fv%3D{v}"),
            format!("https://img.youtube.com/vi/{v}/maxresdefault.jpg"),
            format!("https://i.ytimg.com/vi/{v}/hqdefault.jpg"),
            format!("https://www.youtube.com/attribution_link?a=x&u=%2Fwatch%3Fv%3D{v}%26feature%3Dshare"),
            format!("https://www.youtube.com/redirect?q=https%3A%2F%2Fyoutu.be%2F{v}&event=x"),
            format!("https://www.youtube.com/oembed?url=https%3A%2F%2Fwww.youtube.com%2Fwatch%3Fv%3D{v}&format=json"),
            format!(
                "https://accounts.google.com/ServiceLogin?continue=https%3A%2F%2Fwww.youtube.com%2Fwatch%3Fv%3D{v}"
            ),
            format!("<https://youtu.be/{v}>"),
            format!("\"https://youtu.be/{v}\""),
            format!("watch this: https://youtu.be/{v}, it is great."),
            format!("(see https://www.youtube.com/watch?v={v})"),
            format!("https://youtu.be/{w}"),
        ]
    }

    fn bad() -> Vec<String> {
        let v = V;
        vec![
            String::new(),
            "   ".into(),
            "nonsense".into(),
            "dQw4w9WgXc".into(),
            "dQw4w9WgXcQQ".into(),
            format!("https://example.com/watch?v={v}"),
            format!("https://notyoutube.com/watch?v={v}"),
            format!("https://youtube.com.evil.com/watch?v={v}"),
            format!("https://evil.com/youtu.be/{v}"),
            format!("https://evil.com/redirect?q=https%3A%2F%2Fyoutu.be%2F{v}"),
            "https://www.youtube.com/".into(),
            "https://www.youtube.com/playlist?list=PLabcdefghijk".into(),
            "https://www.youtube.com/embed/videoseries?list=PLabcdefghijk".into(),
            "https://www.youtube.com/embed/live_stream?channel=UCabc".into(),
            "https://www.youtube.com/@somechannel".into(),
            "https://www.youtube.com/channel/UCabcdefghijklmnopqrstuv".into(),
            "https://www.youtube.com/clip/UgkxAbcdefghijklmnop".into(),
            "https://www.youtube.com/watch?v=short".into(),
            format!("https://www.youtube.com/results?search_query={v}"),
        ]
    }

    #[test]
    fn ids_good() {
        for s in good() {
            let want = if s.contains(W) { W } else { V };
            assert_eq!(extract_video_id(&s).as_deref(), Ok(want), "input {s:?}");
        }
    }

    #[test]
    fn ids_bad() {
        for s in bad() {
            assert!(extract_video_id(&s).is_err(), "input {s:?} should fail");
        }
    }

    #[test]
    fn ids_hints() {
        let e = extract_video_id("https://www.youtube.com/playlist?list=PLabcdefghijk").unwrap_err();
        assert!(e.contains("playlist"), "{e}");
        let e = extract_video_id("https://www.youtube.com/clip/UgkxAbcdefghijklmnop").unwrap_err();
        assert!(e.contains("Clip links"), "{e}");
        let e = extract_video_id("nonsense").unwrap_err();
        assert_eq!(e, "Cannot find a YouTube video ID in 'nonsense'. Pass a video link or the 11-character ID.");
    }

    fn sample() -> Transcript {
        Transcript {
            video_id: "abc".into(),
            source: "captions",
            language: Some("en".into()),
            segments: vec![
                Segment { start: 0.0, duration: 2.0, text: "hi".into() },
                Segment { start: 3725.0, duration: 1.0, text: "bye".into() },
            ],
        }
    }

    #[test]
    fn formats_all() {
        let t = sample();
        assert_eq!(render(&t, "text").unwrap(), "hi\nbye");
        let j: Value = serde_json::from_str(&render(&t, "json").unwrap()).unwrap();
        assert_eq!(j["segments"][1]["text"], "bye");
        assert_eq!(j["language"], "en");
        let md = render(&t, "markdown").unwrap();
        assert!(md.contains("**[00:00]** hi"), "{md}");
        assert!(md.contains("**[01:02:05]** bye"), "{md}");
        assert!(md.starts_with("# Transcript: abc\n\nSource: captions (en)\n\n- "), "{md}");
        assert!(render(&t, "pdf").is_err());
    }

    #[test]
    fn json_rounds_to_two_decimals() {
        let mut t = sample();
        t.segments = vec![Segment { start: 1.23456, duration: 0.007, text: "x".into() }];
        t.language = None;
        let s = render(&t, "json").unwrap();
        assert!(s.contains("\"start\": 1.23,"), "{s}");
        assert!(s.contains("\"duration\": 0.01,"), "{s}");
        assert!(s.contains("\"language\": null"), "{s}");
        assert!(s.starts_with("{\n  \"video_id\": \"abc\",\n  \"source\": \"captions\","), "{s}");
    }

    #[test]
    fn stamps() {
        assert_eq!(stamp(0.0), "00:00");
        assert_eq!(stamp(59.9), "00:59");
        assert_eq!(stamp(3600.0), "01:00:00");
        assert_eq!(stamp(3725.0), "01:02:05");
    }

    fn cap(t: Transcript) -> impl Fn(&str, &[String]) -> Result<Transcript, CapErr> {
        move |_, _| Ok(t.clone())
    }

    fn none() -> impl Fn(&str, &[String]) -> Result<Transcript, CapErr> {
        |_, _| Err(CapErr::Lookup("none".into()))
    }

    fn whisper_ok() -> impl Fn(&str, &str, Option<&str>) -> Result<Transcript, String> {
        |id, _, _| Ok(Transcript { video_id: id.into(), source: "whisper", language: None, segments: vec![] })
    }

    #[test]
    fn flow_no_whisper_without_captions() {
        let w = |_: &str, _: &str, _: Option<&str>| -> Result<Transcript, String> { panic!("whisper must not run") };
        let e = transcribe_with(V, &[], "base", false, true, &none(), &w).unwrap_err();
        assert!(e.contains("No captions found") && e.contains("Whisper is off"), "{e}");
    }

    #[test]
    fn flow_fallback() {
        let t = transcribe_with(V, &[], "base", false, false, &none(), &whisper_ok()).unwrap();
        assert_eq!(t.source, "whisper");
    }

    #[test]
    fn flow_force_whisper_skips_captions() {
        let c = |_: &str, _: &[String]| -> Result<Transcript, CapErr> { panic!("captions must not run") };
        let t = transcribe_with(V, &[], "base", true, false, &c, &whisper_ok()).unwrap();
        assert_eq!(t.source, "whisper");
    }

    #[test]
    fn fetch_args_take_the_flag_in_any_position() {
        let v = |xs: &[&str]| xs.iter().map(ToString::to_string).collect::<Vec<_>>();
        let run = |url: &str, force_whisper| Some(FetchArgs::Run { url: url.into(), force_whisper });
        assert_eq!(parse_fetch_args(&v(&[V])), run(V, false));
        assert_eq!(parse_fetch_args(&v(&["--force-whisper", V])), run(V, true));
        assert_eq!(parse_fetch_args(&v(&[V, "--force-whisper"])), run(V, true));
        // an ID may start with a dash: it is the link, not an unknown flag
        assert_eq!(parse_fetch_args(&v(&["--force-whisper", "-abcdefghij"])), run("-abcdefghij", true));
        assert_eq!(parse_fetch_args(&v(&[V, "--help"])), Some(FetchArgs::Help));
        assert_eq!(parse_fetch_args(&v(&[])), None);
        assert_eq!(parse_fetch_args(&v(&["--force-whisper"])), None);
        assert_eq!(parse_fetch_args(&v(&[""])), None);
        assert_eq!(parse_fetch_args(&v(&[V, W])), None);
    }

    #[test]
    fn blocked_message_does_not_promise_whisper() {
        // the Whisper audio also comes through yt-dlp, so a block stops it too
        assert!(!MSG_BLOCKED.contains("whisper") && !MSG_BLOCKED.contains("Whisper"), "{MSG_BLOCKED}");
        assert!(MSG_BLOCKED.contains("Wait") && MSG_BLOCKED.contains("browser"), "{MSG_BLOCKED}");
    }

    #[test]
    fn flow_captions_win() {
        let w = |_: &str, _: &str, _: Option<&str>| -> Result<Transcript, String> { panic!("whisper must not run") };
        let t = transcribe_with(V, &[], "base", false, false, &cap(sample()), &w).unwrap();
        assert_eq!(t.source, "captions");
    }

    #[test]
    fn flow_fatal_not_retried() {
        let c = |_: &str, _: &[String]| -> Result<Transcript, CapErr> { Err(CapErr::Fatal("private".into())) };
        let w = |_: &str, _: &str, _: Option<&str>| -> Result<Transcript, String> { panic!("whisper must not run") };
        assert_eq!(transcribe_with(V, &[], "base", false, false, &c, &w).unwrap_err(), "private");
    }

    #[test]
    fn flow_default_language_and_first_language_for_whisper() {
        let c = |_: &str, langs: &[String]| -> Result<Transcript, CapErr> {
            assert_eq!(langs, ["en".to_string()]);
            Err(CapErr::Lookup("x".into()))
        };
        let w = |_: &str, m: &str, l: Option<&str>| -> Result<Transcript, String> {
            assert_eq!((m, l), ("tiny", None));
            Ok(Transcript { video_id: "x".into(), source: "whisper", language: None, segments: vec![] })
        };
        transcribe_with(V, &[], "tiny", false, false, &c, &w).unwrap();
        let c2 = |_: &str, langs: &[String]| -> Result<Transcript, CapErr> {
            assert_eq!(langs, ["de".to_string(), "en".to_string()]);
            Err(CapErr::Lookup("x".into()))
        };
        let w2 = |_: &str, _: &str, l: Option<&str>| -> Result<Transcript, String> {
            assert_eq!(l, Some("de"));
            Ok(Transcript { video_id: "x".into(), source: "whisper", language: None, segments: vec![] })
        };
        transcribe_with(V, &["de".into(), "en".into()], "base", false, false, &c2, &w2).unwrap();
    }

    #[test]
    fn flow_bad_link_stops_early() {
        let e = transcribe_with("nonsense", &[], "base", false, false, &none(), &whisper_ok()).unwrap_err();
        assert!(e.starts_with("Cannot find a YouTube video ID"), "{e}");
    }

    #[test]
    fn missing_tools_name_the_install_line() {
        let nothing = |_: &str| -> Option<PathBuf> { None };
        let e = need_tool("ffmpeg", "the Whisper fallback", &nothing).unwrap_err();
        assert!(e.contains("ffmpeg") && e.contains("Install it:") && e.contains("install"), "{e}");
        let e = need_tool_any(&["whisper-cli", "whisper-cpp"], "the Whisper fallback", &nothing).unwrap_err();
        assert!(e.contains("whisper-cli") && e.contains("whisper"), "{e}");
        let some = |n: &str| -> Option<PathBuf> { (n == "whisper-cpp").then(|| PathBuf::from("/x/whisper-cpp")) };
        assert_eq!(
            need_tool_any(&["whisper-cli", "whisper-cpp"], "w", &some).unwrap(),
            PathBuf::from("/x/whisper-cpp")
        );
    }

    #[test]
    fn ytdlp_errors_map_to_user_messages() {
        assert_eq!(
            map_ytdlp_error("ERROR: [youtube] x: Private video. Sign in if you've been granted access"),
            Some(MSG_UNPLAYABLE)
        );
        assert_eq!(map_ytdlp_error("ERROR: [youtube] x: Video unavailable"), Some(MSG_UNAVAILABLE));
        assert_eq!(
            map_ytdlp_error("ERROR: [youtube] x: Sign in to confirm your age. This video may be inappropriate"),
            Some(MSG_AGE)
        );
        assert_eq!(map_ytdlp_error("ERROR: [youtube] x: Sign in to confirm you're not a bot"), Some(MSG_BLOCKED));
        assert_eq!(map_ytdlp_error("ERROR: unable to download: HTTP Error 429: Too Many Requests"), Some(MSG_BLOCKED));
        assert_eq!(map_ytdlp_error("ERROR: Sign in required"), Some(MSG_SIGNIN));
        assert_eq!(map_ytdlp_error("ERROR: something odd"), None);
        assert_eq!(ytdlp_message("WARNING: a\nERROR: [youtube] x: boom\n"), "[youtube] x: boom");
    }

    #[test]
    fn json3_parses_and_skips() {
        let j = r#"{"events":[
            {"tStartMs":0,"dDurationMs":5000,"id":1},
            {"tStartMs":160,"dDurationMs":2500,"segs":[{"utf8":"hello"},{"utf8":"  big\nworld ","tOffsetMs":200}]},
            {"tStartMs":2000,"dDurationMs":10,"aAppend":1,"segs":[{"utf8":"\n"}]},
            {"tStartMs":2600,"dDurationMs":900,"segs":[{"utf8":"\n"}]},
            {"tStartMs":3000,"dDurationMs":1500,"segs":[{"utf8":"again"}]}
        ]}"#;
        let s = parse_json3(j).unwrap();
        assert_eq!(
            s,
            vec![
                Segment { start: 0.16, duration: 2.5, text: "hello big world".into() },
                Segment { start: 3.0, duration: 1.5, text: "again".into() },
            ]
        );
        assert!(parse_json3("not json").is_err());
        assert_eq!(parse_json3("{}").unwrap(), [] as [Segment; 0]);
    }

    #[test]
    fn whisper_json_parses() {
        let j = r#"{"result":{"language":"en"},"transcription":[
            {"offsets":{"from":0,"to":2500},"text":" Hello there."},
            {"offsets":{"from":2500,"to":2600},"text":"  "},
            {"offsets":{"from":3000,"to":4250},"text":" Bye."}]}"#;
        let (lang, segs) = parse_whisper_json(j).unwrap();
        assert_eq!(lang.as_deref(), Some("en"));
        assert_eq!(
            segs,
            vec![
                Segment { start: 0.0, duration: 2.5, text: "Hello there.".into() },
                Segment { start: 3.0, duration: 1.25, text: "Bye.".into() },
            ]
        );
    }

    #[test]
    fn model_names() {
        assert!(valid_model_name("tiny"));
        assert!(valid_model_name("base.en"));
        assert!(valid_model_name("large-v3-turbo"));
        assert!(!valid_model_name(""));
        assert!(!valid_model_name("../x"));
        assert!(!valid_model_name("a b"));
    }

    #[test]
    fn collapse_whitespace() {
        assert_eq!(collapse_ws(" a \n b\t\tc "), "a b c");
    }

    #[test]
    fn unquote_cases() {
        assert_eq!(unquote("a%20b"), "a b");
        assert_eq!(unquote("%E2%82%AC"), "€");
        assert_eq!(unquote("100%"), "100%");
        assert_eq!(unquote("%41"), "A");
    }
}
