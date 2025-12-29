//! Config parsing API.
//!
//! Provides high-level interface for parsing ld.so.conf files and getting directory lists.

use crate::Error;
use camino::{Utf8Path, Utf8PathBuf};
use std::fs;
use tracing::warn;

/// Library configuration containing directories to scan
#[derive(Debug, Clone)]
pub struct LibraryConfig {
    directories: Vec<Utf8PathBuf>,
}

impl LibraryConfig {
    /// Create config from file path with optional prefix
    pub fn from_file(
        path: impl AsRef<Utf8Path>,
        prefix: Option<&Utf8Path>,
    ) -> Result<Self, Error> {
        let path = path.as_ref();

        // Parse the main config file
        let mut config = parse_config_file(path)?;

        // Expand includes
        let included_dirs = expand_includes(&config)?;
        config.directories.extend(included_dirs);

        // Apply prefix if provided
        if let Some(prefix) = prefix {
            config.directories = config
                .directories
                .into_iter()
                .map(|dir| prefix.join(dir.strip_prefix("/").unwrap_or(&dir)))
                .collect();
        }

        Ok(Self {
            directories: config.directories,
        })
    }

    /// Create default config (standard system directories)
    pub fn default() -> Self {
        Self {
            directories: vec![
                Utf8PathBuf::from("/lib"),
                Utf8PathBuf::from("/usr/lib"),
                Utf8PathBuf::from("/lib64"),
                Utf8PathBuf::from("/usr/lib64"),
            ],
        }
    }

    /// Create config from explicit directory list
    pub fn from_directories(directories: Vec<Utf8PathBuf>) -> Self {
        Self { directories }
    }

    /// Get directories to scan
    pub fn directories(&self) -> &[Utf8PathBuf] {
        &self.directories
    }
}

impl Default for LibraryConfig {
    fn default() -> Self {
        Self::default()
    }
}

// Internal parsing structures and functions

#[derive(Debug, Clone)]
struct RawConfig {
    directories: Vec<Utf8PathBuf>,
    include_patterns: Vec<String>,
}

impl Default for RawConfig {
    fn default() -> Self {
        Self {
            directories: vec![
                Utf8PathBuf::from("/lib"),
                Utf8PathBuf::from("/usr/lib"),
                Utf8PathBuf::from("/lib64"),
                Utf8PathBuf::from("/usr/lib64"),
            ],
            include_patterns: Vec::new(),
        }
    }
}

fn parse_config_file(path: &Utf8Path) -> Result<RawConfig, Error> {
    let content = fs::read_to_string(path)
        .map_err(|e| Error::Config(format!("Failed to read config file: {}", e)))?;

    parse_config_content(&content)
}

fn parse_config_content(content: &str) -> Result<RawConfig, Error> {
    let mut config = RawConfig::default();

    for line in content.lines() {
        let line = line.trim();

        // Skip empty lines and comments
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Handle include directives
        if line.starts_with("include ") {
            let pattern = line[8..].trim();
            config.include_patterns.push(pattern.to_string());
        } else {
            // Add directory
            config.directories.push(Utf8PathBuf::from(line));
        }
    }

    Ok(config)
}

fn expand_includes(config: &RawConfig) -> Result<Vec<Utf8PathBuf>, Error> {
    let mut included_dirs = Vec::new();

    for pattern in &config.include_patterns {
        // Use glob to expand the pattern
        for entry in
            glob::glob(pattern).map_err(|e| Error::Config(format!("Glob pattern error: {}", e)))?
        {
            match entry {
                Ok(path) => {
                    if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("conf") {
                        // This is a config file, parse it
                        let content = std::fs::read_to_string(&path).map_err(|e| {
                            Error::Config(format!(
                                "Failed to read included config file {}: {}",
                                path.display(),
                                e
                            ))
                        })?;

                        // Parse the content as a config file
                        let included_config = parse_config_content(&content)?;

                        // Add the directories from this included config
                        for dir in included_config.directories {
                            included_dirs.push(dir);
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to process glob pattern {}: {}", pattern, e);
                }
            }
        }
    }

    Ok(included_dirs)
}

// Re-exports for backwards compatibility (temporary)
pub use LibraryConfig as Config;

pub fn parse_config_file_compat(path: &Utf8Path) -> Result<Config, Error> {
    let content = fs::read_to_string(path)
        .map_err(|e| Error::Config(format!("Failed to read config file: {}", e)))?;

    let raw = parse_config_content(&content)?;
    let included = expand_includes(&raw)?;
    let mut dirs = raw.directories;
    dirs.extend(included);

    Ok(Config::from_directories(dirs))
}

pub fn parse_config_content_compat(content: &str) -> Result<Config, Error> {
    let raw = parse_config_content(content)?;
    let included = expand_includes(&raw)?;
    let mut dirs = raw.directories;
    dirs.extend(included);

    Ok(Config::from_directories(dirs))
}

pub fn expand_includes_compat(config: &Config) -> Result<Vec<Utf8PathBuf>, Error> {
    Ok(config.directories().to_vec())
}
