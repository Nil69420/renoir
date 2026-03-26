//! Tests for error types

#[cfg(test)]
mod tests {
    use renoir::error::RenoirError;

    #[test]
    fn test_error_creation() {
        let err = RenoirError::memory("Out of memory");
        assert!(matches!(err, RenoirError::Memory { .. }));

        let err = RenoirError::region_not_found("test_region");
        assert!(matches!(err, RenoirError::RegionNotFound { .. }));

        let err = RenoirError::insufficient_space(1024, 512);
        assert!(matches!(err, RenoirError::InsufficientSpace { .. }));
    }

    #[test]
    fn test_error_display() {
        let err = RenoirError::memory("Test message");
        let display = format!("{}", err);
        assert!(display.contains("Memory error"));
        assert!(display.contains("Test message"));
    }
}
