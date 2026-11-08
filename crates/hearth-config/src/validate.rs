use crate::{ConfigError, Settings, unresolved};

fn invalid(message: impl Into<String>) -> ConfigError {
    ConfigError::Invalid(message.into())
}

fn require(field: &str, value: &str) -> Result<(), ConfigError> {
    if let Some(name) = unresolved(value) {
        return Err(invalid(format!(
            "environment variable {name} is not set (referenced by {field})"
        )));
    }
    if value.is_empty() {
        return Err(invalid(format!("{field} must not be empty")));
    }
    Ok(())
}

impl Settings {
    /// Validate the shape of a parsed settings file.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if let Some(models) = &self.models {
            require("models.base_url", &models.base_url)?;
        }
        Ok(())
    }
}
