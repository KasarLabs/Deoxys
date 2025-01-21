#[derive(Debug, thiserror::Error)]
pub enum BloomError {
    #[error("Filter size {0} is too small (minimum: {1})")]
    SizeTooSmall(usize, usize),
    #[error("Filter size {0} is too large (maximum: {1})")]
    SizeTooLarge(usize, usize),
    #[error("Hash count {0} is too small (minimum: {1})")]
    TooFewHashes(usize, usize),
    #[error("Hash count {0} is too large (maximum: {1})")]
    TooManyHashes(usize, usize),
}
