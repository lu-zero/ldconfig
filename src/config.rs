use crate::Error;
use camino::{Utf8Path, Utf8PathBuf};
use std::fs;

#[derive(Debug, Clone)]
pub struct Config {
    pub directories: Vec<Utf8PathBuf>,
    pub include_patterns: Vec<String>,
}

impl Default for Config {
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

pub fn parse_config_file(path: &Utf8Path) -> Result<Config, Error> {
    let content = fs::read_to_string(path)
        .map_err(|e| Error::Config(format!("Failed to read config file: {}", e)))?;

    parse_config_content(&content)
}

pub fn parse_config_content(content: &str) -> Result<Config, Error> {
    let mut config = Config::default();

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

pub fn expand_includes(config: &Config) -> Result<Vec<Utf8PathBuf>, Error> {
    let mut included_dirs = Vec::new();

    for pattern in &config.include_patterns {
        // Use glob to expand the pattern
        for entry in glob::glob(pattern)
            .map_err(|e| Error::Config(format!("Glob pattern error: {}", e)))?
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
                        // The directories in the config files are absolute paths (like /lib, /usr/lib)
                        // We return them as-is, and the caller will apply the appropriate prefix
                        for dir in included_config.directories {
                            included_dirs.push(dir);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("Warning: Failed to process glob pattern {}: {}", pattern, e);
                }
            }
        }
    }

    Ok(included_dirs)
}
