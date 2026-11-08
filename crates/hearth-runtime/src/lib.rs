//! Hearth runtime wiring.

use std::path::Path;

use hearth_config::ConfigError;

/// Check the home folder without printing secrets.
pub fn doctor(home: &Path) -> Result<(), ConfigError> {
    hearth_config::load(home)?;
    Ok(())
}
