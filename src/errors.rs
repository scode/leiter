use thiserror::Error;

/// Structured errors for operations that callers need to match on.
///
/// Command handlers use `anyhow` for propagation; this enum is for cases where
/// the caller's control flow depends on the specific failure mode (e.g.,
/// distinguishing "soul not found" from "bad frontmatter").
#[derive(Debug, Error)]
pub enum LeiterError {
    #[error("soul file not found")]
    SoulNotFound,

    #[error("failed to parse frontmatter: {0}")]
    FrontmatterParse(String),

    #[error("invalid log filename: {0}")]
    LogFilenameParse(String),

    #[error("cannot resolve state directory `{0}`: {1}")]
    StateDir(String, std::io::Error),

    #[error("cannot determine home directory")]
    HomeNotFound,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soul_not_found_display() {
        let err = LeiterError::SoulNotFound;
        assert_eq!(err.to_string(), "soul file not found");
    }

    #[test]
    fn frontmatter_parse_display() {
        let err = LeiterError::FrontmatterParse("bad yaml".to_string());
        assert_eq!(err.to_string(), "failed to parse frontmatter: bad yaml");
    }

    #[test]
    fn log_filename_parse_display() {
        let err = LeiterError::LogFilenameParse("bad format".to_string());
        assert_eq!(err.to_string(), "invalid log filename: bad format");
    }

    #[test]
    fn home_not_found_display() {
        let err = LeiterError::HomeNotFound;
        assert_eq!(err.to_string(), "cannot determine home directory");
    }

    #[test]
    fn errors_implement_std_error() {
        fn assert_std_error<T: std::error::Error>() {}
        assert_std_error::<LeiterError>();
    }
}
