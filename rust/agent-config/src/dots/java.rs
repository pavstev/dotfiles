//! Find the Java homes on this machine, for the JAVA_HOME setting of the settings menu.

use super::os;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The Java home at or in `dir` (the folder with bin/java), or None. Homebrew keeps it in
/// libexec/openjdk.jdk/Contents/Home, a .jdk folder in Contents/Home. /usr is skipped: its
/// java is the macOS stub.
pub fn home_of(dir: &Path) -> Option<PathBuf> {
    let java = if os::is_windows() { "java.exe" } else { "java" };
    [dir.join("libexec/openjdk.jdk/Contents/Home"), dir.join("Contents/Home"), dir.to_path_buf()].into_iter()
        .find(|d| d != Path::new("/usr") && os::is_executable(&d.join("bin").join(java)))
}

/// The real folder behind a home, links followed. Two homes with the same real folder are
/// the same Java.
pub fn real(home: &Path) -> PathBuf {
    let java = home.join("bin").join("java");
    fs::canonicalize(&java).ok()
        .and_then(|j| j.parent().and_then(|p| p.parent()).map(Path::to_path_buf))
        .unwrap_or_else(|| home.to_path_buf())
}

/// The entries of `dir` whose names start with `prefix`, sorted.
fn glob(dir: &Path, prefix: &str) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(dir).into_iter().flatten().flatten()
        .filter(|e| { let n = e.file_name().to_string_lossy().into_owned(); n.starts_with(prefix) && !n.starts_with('.') })
        .map(|e| e.path()).collect();
    v.sort();
    v
}

fn env_or(name: &str, fallback: PathBuf) -> PathBuf {
    std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or(fallback)
}

/// The folders where a Java may live, in the order they are tried.
fn candidates() -> Vec<PathBuf> {
    let h = os::home();
    let mut v = Vec::new();
    let jh = Path::new("/usr/libexec/java_home");
    if os::is_executable(jh) && let Ok(o) = Command::new(jh).arg("-V").stdout(Stdio::null()).output() {
        // "    21.0.5 (arm64) "Eclipse Adoptium" - "OpenJDK 21.0.5" /Library/Java/.../Home"
        for line in String::from_utf8_lossy(&o.stderr).lines() {
            if !line.starts_with([' ', '\t']) { continue; }
            if let Some(i) = line.rfind([' ', '\t']) && line[i + 1..].starts_with('/') { v.push(PathBuf::from(&line[i + 1..])); }
        }
    }
    v.extend(glob(Path::new("/Library/Java/JavaVirtualMachines"), ""));
    v.extend(glob(&h.join("Library/Java/JavaVirtualMachines"), ""));
    v.extend(glob(&env_or("HOMEBREW_PREFIX", PathBuf::from("/opt/homebrew")).join("opt"), "openjdk"));
    v.extend(glob(Path::new("/usr/local/opt"), "openjdk"));
    v.extend(glob(&env_or("MISE_DATA_DIR", h.join(".local/share/mise")).join("installs/java"), ""));
    v.extend(glob(&env_or("SDKMAN_DIR", h.join(".sdkman")).join("candidates/java"), ""));
    v.extend(glob(&env_or("ASDF_DATA_DIR", h.join(".asdf")).join("installs/java"), ""));
    v.extend(glob(&env_or("JENV_ROOT", h.join(".jenv")).join("versions"), ""));
    let gradle = glob(&h.join(".gradle/jdks"), "");
    v.extend(gradle.iter().cloned());
    for g in &gradle { v.extend(glob(g, "")); }
    v.extend(glob(&h.join(".jbang/cache/jdks"), ""));
    v.extend(glob(Path::new("/usr/lib/jvm"), ""));
    v.push(h.join(".local/share/JetBrains/Toolbox/apps/android-studio/jbr"));
    v.push(PathBuf::from("/Applications/Android Studio.app/Contents/jbr"));
    if let Some(j) = std::env::var_os("JAVA_HOME").filter(|j| !j.is_empty()) { v.push(PathBuf::from(j)); }
    v
}

/// The Java homes found, each Java once. The first path wins, so a link that survives
/// updates (Homebrew's opt/openjdk@21, mise's java/21) beats the versioned folder behind it.
pub fn homes() -> Vec<PathBuf> {
    let mut seen = Vec::new();
    let mut out = Vec::new();
    for d in candidates() {
        let Some(h) = home_of(&d) else { continue };
        let r = real(&h);
        if seen.contains(&r) { continue; }
        seen.push(r);
        out.push(h);
    }
    out
}

/// (major, version, vendor), from the release file, else from `java -version`.
pub fn info(home: &Path) -> (u32, String, String) {
    let rel = fs::read_to_string(home.join("release")).unwrap_or_default();
    let field = |k: &str| rel.lines().find_map(|l| l.strip_prefix(&format!("{}=\"", k)).and_then(|r| r.strip_suffix('"')).map(String::from));
    let mut v = field("JAVA_VERSION").unwrap_or_default();
    let vendor = field("IMPLEMENTOR").unwrap_or_default();
    if v.is_empty() && let Ok(o) = Command::new(home.join("bin").join("java")).arg("-version").output() {
        let text = String::from_utf8_lossy(&o.stderr).into_owned();
        if let Some(first) = text.lines().next() && let Some(i) = first.find("version \"") {
            let rest = &first[i + 9..];
            v = rest.split('"').next().unwrap_or("").to_string();
        }
    }
    (major_of(&v), if v.is_empty() { "unknown".into() } else { v }, vendor)
}

const SEPS: [char; 4] = ['.', '_', '+', '-'];

/// The major version: "21.0.5" is 21, "1.8.0_432" is 8. Not a number: 0.
pub fn major_of(v: &str) -> u32 {
    let first = v.split(SEPS).next().unwrap_or("");
    let m = if first == "1" { v.strip_prefix("1.").unwrap_or("").split(SEPS).next().unwrap_or("") } else { first };
    m.parse().unwrap_or(0)
}

/// Where a Java home came from, in a few words.
pub fn where_from(home: &Path) -> String {
    let s = home.to_string_lossy().into_owned();
    let h = os::home().to_string_lossy().into_owned();
    let has = |p: &str| s.contains(p);
    if has("/Cellar/openjdk") || has("/opt/openjdk") { "Homebrew".into() }
    else if s.starts_with(&format!("{}/Library/Java/", h)) { "~/Library/Java".into() }
    else if s.starts_with("/Library/Java/") { "/Library/Java".into() }
    else if has("/mise/installs/") { "mise".into() }
    else if has("/.sdkman/") || has("/candidates/java/") { "SDKMAN".into() }
    else if has("/.asdf/") { "asdf".into() }
    else if has("/.jenv/") { "jenv".into() }
    else if has("/.gradle/jdks/") { "Gradle".into() }
    else if has("/.jbang/") { "jbang".into() }
    else if has("/Android Studio") { "Android Studio".into() }
    else { s }
}

/// "21.0.5 · Eclipse Adoptium · /Library/Java".
pub fn label(home: &Path) -> String {
    let (_, v, vendor) = info(home);
    let w = where_from(home);
    if vendor.is_empty() || vendor == w { format!("{} · {}", v, w) } else { format!("{} · {} · {}", v, vendor, w) }
}

/// Long-term-support versions: 8, 11, 17, 21, 25, ...
pub fn is_lts(major: u32) -> bool { major == 8 || major == 11 || (major >= 17 && (major - 17).is_multiple_of(4)) }

/// A sort key: LTS first, then the newest.
pub fn rank_key(v: &str) -> (bool, u64) {
    let major = major_of(v);
    let rest = v.strip_prefix("1.").unwrap_or(v);
    let parts: Vec<&str> = rest.split(SEPS).collect();
    let num = |i: usize| parts.get(i).and_then(|p| p.parse::<u64>().ok()).unwrap_or(0);
    (is_lts(major), major as u64 * 1_000_000 + num(1) * 1000 + num(2))
}

/// The Java homes, best first. Equal keys keep their order.
pub fn ranked() -> Vec<PathBuf> {
    let mut v: Vec<(bool, u64, PathBuf)> = homes().into_iter().map(|h| { let (l, k) = rank_key(&info(&h).1); (l, k, h) }).collect();
    v.sort_by_key(|t| std::cmp::Reverse((t.0, t.1)));
    v.into_iter().map(|t| t.2).collect()
}

/// A typed path made usable: no quotes or spaces around it, "\ " as a space, ~ as the home
/// folder, no / or /bin/java or /bin at the end.
pub fn clean_path(p: &str) -> String {
    let mut p = p.trim().to_string();
    if p.len() >= 2 && ((p.starts_with('"') && p.ends_with('"')) || (p.starts_with('\'') && p.ends_with('\''))) {
        p = p[1..p.len() - 1].to_string();
    }
    p = p.replace("\\ ", " ");
    if p == "~" || p.starts_with("~/") { p = format!("{}{}", os::home().display(), &p[1..]); }
    while p.ends_with('/') && p != "/" { p.pop(); }
    if let Some(s) = p.strip_suffix("/bin/java") { p = s.to_string(); }
    if let Some(s) = p.strip_suffix("/bin") { p = s.to_string(); }
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn majors() {
        assert_eq!(major_of("21.0.5"), 21);
        assert_eq!(major_of("1.8.0_432"), 8);
        assert_eq!(major_of("17"), 17);
        assert_eq!(major_of("unknown"), 0);
        assert_eq!(major_of("25-ea"), 25);
    }
    #[test]
    fn ranking() {
        assert!(is_lts(8) && is_lts(11) && is_lts(17) && is_lts(21) && is_lts(25) && !is_lts(22) && !is_lts(9));
        assert!(rank_key("21.0.5") > rank_key("17.0.9"));
        assert!(rank_key("17.0.9") > rank_key("23.0.1")); // LTS first
        assert_eq!(rank_key("1.8.0_432"), (true, 8_000_432));
        assert_eq!(rank_key("21.0.5").1, 21_000_005);
    }
    #[test]
    fn paths() {
        assert_eq!(clean_path("  '/opt/jdk/bin/java' "), "/opt/jdk");
        assert_eq!(clean_path("/opt/my\\ jdk/bin/"), "/opt/my jdk");
        assert_eq!(clean_path("/"), "/");
        assert!(clean_path("~/x").ends_with("/x") && !clean_path("~/x").starts_with('~'));
    }
    #[cfg(unix)]
    #[test]
    fn finds_a_fake_home() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let jdk = t.path().join("jdk-21.jdk/Contents/Home");
        fs::create_dir_all(jdk.join("bin")).unwrap();
        fs::write(jdk.join("bin/java"), "#!/bin/sh\n").unwrap();
        fs::set_permissions(jdk.join("bin/java"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(jdk.join("release"), "JAVA_VERSION=\"21.0.5\"\nIMPLEMENTOR=\"Eclipse Adoptium\"\n").unwrap();
        assert_eq!(home_of(&t.path().join("jdk-21.jdk")).unwrap(), jdk);
        assert_eq!(info(&jdk), (21, "21.0.5".into(), "Eclipse Adoptium".into()));
        assert!(home_of(t.path()).is_none());
    }
}
