//! One redactor for every tool: no secret is printed or written to an index.

use regex::{Regex, RegexSet};
use std::borrow::Cow;
use std::sync::LazyLock;

const MARK: &str = "[REDACTED]";

/// Each rule is a pattern and its replacement. The named-value rule keeps the name
/// (group 1) and replaces only the value (group 2).
const RULES: &[(&str, &str)] = &[
    // Private key blocks, also a block cut off before its END line.
    (r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?(?:-----END [A-Z ]*PRIVATE KEY-----|\z)", MARK),
    // OpenAI and Anthropic style keys (`sk-`, `sk-ant-`).
    (r"\bsk-[A-Za-z0-9_-]{20,}", MARK),
    // GitHub tokens: classic (`ghp_` and kin) and fine-grained.
    (r"\bgh[pousr]_[A-Za-z0-9]{36,}", MARK),
    (r"\bgithub_pat_[A-Za-z0-9_]{22,}", MARK),
    // AWS access key ids.
    (r"\b(?:AKIA|ASIA)[A-Z0-9]{16}\b", MARK),
    // Slack tokens.
    (r"\bxox[abprs]-[A-Za-z0-9-]{10,}", MARK),
    // Bearer values.
    (r"(?i)\bBearer\s+[A-Za-z0-9._~+/=-]{20,}", MARK),
    // A name that holds key, token, secret or password, assigned 16+ chars.
    (
        r#"(?i)\b([A-Za-z0-9_.-]*(?:key|token|secret|password)[A-Za-z0-9_.-]*["']?\s*[=:]\s*["']?)[^\s"',;]{16,}"#,
        "${1}[REDACTED]",
    ),
];

struct Rules {
    set: RegexSet,
    list: Vec<(Regex, &'static str)>,
}

static COMPILED: LazyLock<Rules> = LazyLock::new(|| Rules {
    set: RegexSet::new(RULES.iter().map(|(p, _)| *p)).expect("redact patterns compile"),
    list: RULES.iter().map(|(p, r)| (Regex::new(p).expect("redact pattern compiles"), *r)).collect(),
});

/// Replace secrets in `text` with `[REDACTED]`. Returns `Cow::Borrowed` when nothing matched.
pub fn redact(text: &str) -> Cow<'_, str> {
    let rules = &*COMPILED;
    let hits = rules.set.matches(text);
    if !hits.matched_any() {
        return Cow::Borrowed(text);
    }
    let mut out = text.to_string();
    for i in hits.iter() {
        let (re, rep) = &rules.list[i];
        if let Cow::Owned(next) = re.replace_all(&out, *rep) {
            out = next;
        }
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rep(unit: &str, n: usize) -> String {
        unit.repeat(n)
    }

    fn gone(shape: &str) {
        let input = format!("before {shape} after");
        let out = redact(&input);
        assert!(!out.contains(shape), "shape left in: {out}");
        assert!(out.contains("[REDACTED]"), "no marker in: {out}");
        assert!(out.starts_with("before "), "context lost: {out}");
        assert!(out.ends_with(" after"), "context lost: {out}");
    }

    #[test]
    fn openai_and_anthropic_style_keys() {
        gone(&("sk-".to_string() + &rep("a1B2", 10)));
        gone(&("sk-ant-".to_string() + &rep("aB3_-", 18)));
    }

    #[test]
    fn github_classic_token() {
        gone(&("ghp_".to_string() + &rep("aB3d", 9)));
    }

    #[test]
    fn github_fine_grained_token() {
        gone(&("github_pat_".to_string() + &rep("aB3_", 20) + "xy"));
    }

    #[test]
    fn aws_access_key_id() {
        gone(&("AKIA".to_string() + &rep("AB12", 4)));
    }

    #[test]
    fn slack_tokens() {
        gone(&("xoxb-".to_string() + "1234567890-abcdefghij"));
        gone(&("xoxp-".to_string() + "1234567890-abcdefghij"));
    }

    #[test]
    fn bearer_value() {
        gone(&("Bearer ".to_string() + &rep("abcDEF1234", 3)));
    }

    #[test]
    fn private_key_block() {
        let begin = "-----BEGIN ".to_string() + "RSA PRIVATE KEY-----";
        let end = "-----END ".to_string() + "RSA PRIVATE KEY-----";
        let body = "MIIEowIBAAKCAQEA\nabcdefghijklmnop\n";
        let input = format!("see\n{begin}\n{body}{end}\ndone");
        let out = redact(&input);
        assert!(!out.contains("MIIEowIBAAKCAQEA"));
        assert!(!out.contains("abcdefghijklmnop"));
        assert!(!out.contains("PRIVATE KEY"));
        assert!(out.starts_with("see\n"));
        assert!(out.ends_with("\ndone"));
    }

    #[test]
    fn truncated_private_key_block_goes_to_the_end() {
        let begin = "-----BEGIN ".to_string() + "PRIVATE KEY-----";
        let input = format!("x\n{begin}\nMIIEvQIBADANBgkqhkiG9w0B");
        let out = redact(&input);
        assert!(!out.contains("MIIEvQ"));
        assert!(out.starts_with("x\n"));
    }

    #[test]
    fn named_value_keeps_the_name() {
        let value = rep("q7Zx", 5);
        for (name, sep) in
            [("api_key", "="), ("API_KEY", ": "), ("auth_token", " = "), ("client_secret", ":"), ("DB_PASSWORD", "=")]
        {
            let input = format!("{name}{sep}{value}");
            let out = redact(&input);
            assert!(!out.contains(&value), "value left in: {out}");
            assert!(out.starts_with(name), "name lost: {out}");
            assert!(out.ends_with("[REDACTED]"), "no marker: {out}");
        }
    }

    #[test]
    fn named_value_in_json_and_quotes() {
        let value = rep("m4Kp", 6);
        let input = format!("{{\"password\": \"{value}\", \"user\": \"bob\"}}");
        let out = redact(&input);
        assert!(!out.contains(&value), "value left in: {out}");
        assert!(out.contains("\"password\""));
        assert!(out.contains("\"user\": \"bob\""));
    }

    #[test]
    fn short_named_value_is_kept() {
        let input = "password".to_string() + "=" + &rep("a", 15);
        let out = redact(&input);
        assert!(matches!(out, Cow::Borrowed(_)));
        assert_eq!(out, input);
    }

    #[test]
    fn sixteen_char_named_value_is_redacted() {
        let name = "tok".to_string() + "en" + "=";
        let input = name.clone() + &rep("a", 16);
        let out = redact(&input);
        assert_eq!(out, name + "[REDACTED]");
    }

    #[test]
    fn plain_prose_is_borrowed() {
        let input = "The task-implementer agent reads the key file and asks about a token count.";
        let out = redact(input);
        assert!(matches!(out, Cow::Borrowed(_)));
        assert_eq!(out, input);
    }

    #[test]
    fn short_lookalikes_are_borrowed() {
        for input in ["sk-short", "ghp_abc", "Bearer abc", "AKIA1234", "xoxb-1"] {
            assert!(matches!(redact(input), Cow::Borrowed(_)), "{input}");
        }
    }

    #[test]
    fn redacting_twice_changes_nothing() {
        let input = "api_key".to_string() + "=" + &rep("q7Zx", 5) + " and sk-" + &rep("a1B2", 10);
        let once = redact(&input).into_owned();
        let twice = redact(&once);
        assert!(matches!(twice, Cow::Borrowed(_)));
        assert_eq!(twice, once);
    }

    #[test]
    fn several_secrets_in_one_text() {
        let a = "ghp_".to_string() + &rep("aB3d", 9);
        let b = "AKIA".to_string() + &rep("AB12", 4);
        let input = format!("one {a} two {b} three");
        let out = redact(&input);
        assert_eq!(out, "one [REDACTED] two [REDACTED] three");
    }
}
