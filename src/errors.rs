use thiserror::Error;

#[derive(Debug, Error)]
pub enum LeiterError {
    #[error("soul file not found (~/.leiter/soul.md does not exist)")]
    SoulNotFound,

    #[error("failed to parse frontmatter: {0}")]
    FrontmatterParse(String),

    #[error("logs directory not found (~/.leiter/logs/ does not exist)")]
    LogsDirNotFound,

    #[error("cannot determine home directory")]
    HomeNotFound,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soul_not_found_display() {
        let err = LeiterError::SoulNotFound;
        assert_eq!(err.to_string(), "soul file not found (~/.leiter/soul.md does not exist)");
    }

    #[test]
    fn frontmatter_parse_display() {
        let err = LeiterError::FrontmatterParse("bad yaml".to_string());
        assert_eq!(err.to_string(), "failed to parse frontmatter: bad yaml");
    }

    #[test]
    fn logs_dir_not_found_display() {
        let err = LeiterError::LogsDirNotFound;
        assert_eq!(err.to_string(), "logs directory not found (~/.leiter/logs/ does not exist)");
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
