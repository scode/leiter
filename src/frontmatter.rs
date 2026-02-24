//! Parse and serialize the YAML frontmatter in `soul.md`.
//!
//! The soul file uses Jekyll-style frontmatter: a YAML block delimited by `---`
//! lines, followed by a markdown body. Only the first `---` pair is treated as
//! frontmatter; any `---` in the body (e.g., horizontal rules) is left alone.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::errors::LeiterError;

/// CLI-managed metadata stored in the soul file's YAML frontmatter.
///
/// The agent owns the soul body, but the CLI reads and writes these fields
/// to coordinate distillation timing and template upgrades.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoulFrontmatter {
    /// Used by `leiter distill` to select only unprocessed session logs.
    pub last_distilled: DateTime<Utc>,
    /// Tracks which soul template version this file was created from,
    /// so `leiter soul-upgrade` can detect drift.
    pub soul_version: u32,
}

/// Extract frontmatter and body from a soul file's content.
///
/// Splits on the first `\n---\n` after the opening `---\n`, so `---` lines
/// inside the body (markdown horizontal rules) are not mistaken for delimiters.
pub fn parse_soul(content: &str) -> Result<(SoulFrontmatter, &str), LeiterError> {
    let content = content.strip_prefix("---\n").ok_or_else(|| {
        LeiterError::FrontmatterParse("missing opening --- delimiter".to_string())
    })?;

    let (yaml, body) = content.split_once("\n---\n").ok_or_else(|| {
        LeiterError::FrontmatterParse("missing closing --- delimiter".to_string())
    })?;

    let frontmatter: SoulFrontmatter =
        serde_yaml::from_str(yaml).map_err(|e| LeiterError::FrontmatterParse(e.to_string()))?;

    Ok((frontmatter, body))
}

/// Reassemble a soul file from frontmatter and body.
///
/// `serde_yaml::to_string` always emits a trailing newline, so the output
/// naturally produces `---\n<yaml>\n---\n<body>`.
pub fn serialize_soul(frontmatter: &SoulFrontmatter, body: &str) -> String {
    let yaml = serde_yaml::to_string(frontmatter)
        .expect("SoulFrontmatter contains only simple scalar types");
    format!("---\n{yaml}---\n{body}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn epoch() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(1970, 1, 1, 0, 0, 0).unwrap()
    }

    fn sample_frontmatter() -> SoulFrontmatter {
        SoulFrontmatter {
            last_distilled: Utc.with_ymd_and_hms(2026, 2, 23, 17, 0, 0).unwrap(),
            soul_version: 1,
        }
    }

    fn sample_document() -> &'static str {
        "---\nlast_distilled: 2026-02-23T17:00:00Z\nsoul_version: 1\n---\nHello, world!\n"
    }

    #[test]
    fn parse_valid_frontmatter() {
        let (fm, _) = parse_soul(sample_document()).unwrap();
        assert_eq!(fm, sample_frontmatter());
    }

    #[test]
    fn parse_returns_correct_body() {
        let (_, body) = parse_soul(sample_document()).unwrap();
        assert_eq!(body, "Hello, world!\n");
    }

    #[test]
    fn round_trip() {
        let fm = sample_frontmatter();
        let body = "Some soul content.\n";
        let serialized = serialize_soul(&fm, body);
        let (parsed_fm, parsed_body) = parse_soul(&serialized).unwrap();
        assert_eq!(parsed_fm, fm);
        assert_eq!(parsed_body, body);
    }

    #[test]
    fn error_on_missing_opening_delimiter() {
        let err = parse_soul("last_distilled: 2026-02-23T17:00:00Z\nsoul_version: 1\n---\nbody\n")
            .unwrap_err();
        assert!(err.to_string().contains("opening ---"));
    }

    #[test]
    fn error_on_missing_closing_delimiter() {
        let err =
            parse_soul("---\nlast_distilled: 2026-02-23T17:00:00Z\nsoul_version: 1\n").unwrap_err();
        assert!(err.to_string().contains("closing ---"));
    }

    #[test]
    fn error_on_missing_last_distilled() {
        let result = parse_soul("---\nsoul_version: 1\n---\nbody\n");
        assert!(result.is_err());
    }

    #[test]
    fn error_on_missing_soul_version() {
        let result = parse_soul("---\nlast_distilled: 2026-02-23T17:00:00Z\n---\nbody\n");
        assert!(result.is_err());
    }

    #[test]
    fn error_on_invalid_yaml() {
        let result = parse_soul("---\n: : : not valid\n---\nbody\n");
        assert!(result.is_err());
    }

    #[test]
    fn error_on_empty_input() {
        let result = parse_soul("");
        assert!(result.is_err());
    }

    #[test]
    fn body_with_horizontal_rules() {
        let doc = "---\nlast_distilled: 1970-01-01T00:00:00Z\nsoul_version: 1\n---\nBefore rule\n\n---\n\nAfter rule\n";
        let (fm, body) = parse_soul(doc).unwrap();
        assert_eq!(fm.last_distilled, epoch());
        assert_eq!(body, "Before rule\n\n---\n\nAfter rule\n");
    }

    #[test]
    fn preserves_body_whitespace() {
        let body_content = "  indented\n\n\n  more indented  \n";
        let doc = format!(
            "---\nlast_distilled: 1970-01-01T00:00:00Z\nsoul_version: 1\n---\n{body_content}"
        );
        let (_, body) = parse_soul(&doc).unwrap();
        assert_eq!(body, body_content);
    }

    #[test]
    fn empty_body() {
        let doc = "---\nlast_distilled: 1970-01-01T00:00:00Z\nsoul_version: 1\n---\n";
        let (fm, body) = parse_soul(doc).unwrap();
        assert_eq!(fm.last_distilled, epoch());
        assert_eq!(body, "");

        let roundtripped = serialize_soul(&fm, body);
        let (fm2, body2) = parse_soul(&roundtripped).unwrap();
        assert_eq!(fm2, fm);
        assert_eq!(body2, "");
    }
}
