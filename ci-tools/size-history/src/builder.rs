// Licensed under the Apache-2.0 license

use std::path::Path;

/// Trait for building artifacts and measuring their size.
///
/// Implement this trait for each type of artifact you want to track.
/// Each implementation is responsible for building the artifact and
/// returning its size in bytes.
///
/// # Example
///
/// ```ignore
/// use caliptra_size_history::ArtifactBuilder;
/// use std::path::Path;
///
/// struct MyFirmwareBuilder {
///     name: String,
/// }
///
/// impl ArtifactBuilder for MyFirmwareBuilder {
///     fn name(&self) -> &str {
///         &self.name
///     }
///
///     fn build_and_measure(&self, workspace: &Path) -> Option<u64> {
///         // Build your artifact and return its size
///         Some(1337)
///     }
/// }
/// ```
pub trait ArtifactBuilder {
    /// Unique name for this artifact (used in reports and cache keys).
    fn name(&self) -> &str;

    /// Build the artifact and return its size in bytes.
    ///
    /// Returns `None` if the build fails (graceful degradation).
    /// The implementation should handle errors internally and log them
    /// as appropriate.
    fn build_and_measure(&self, workspace: &Path) -> Option<u64>;
}
