use std::fs;
use std::path::PathBuf;

/// Get environment variable value from shell configuration files
///
/// Searches through common shell config files and parses export statements
/// to find the specified environment variable
pub fn get_env_from_shell_config(var_name: &str) -> Option<String> {
    let config_files = get_shell_config_files()?;

    for config_file in config_files {
        if let Some(value) = parse_env_from_file(&config_file, var_name) {
            return Some(value);
        }
    }

    None
}

/// Get list of shell configuration files to check (in priority order)
#[allow(unused_variables)]
fn get_shell_config_files() -> Option<Vec<PathBuf>> {
    let home_dir = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()?;

    let home_path = PathBuf::from(home_dir);

    // Platform-specific configuration files in priority order
    #[cfg(target_os = "macos")]
    let config_files = vec![
        home_path.join(".zshrc"),
        home_path.join(".zprofile"),
        home_path.join(".bashrc"),
        home_path.join(".bash_profile"),
        home_path.join(".profile"),
    ];

    #[cfg(target_os = "linux")]
    let config_files = vec![
        home_path.join(".bashrc"),
        home_path.join(".bash_profile"),
        home_path.join(".profile"),
        home_path.join(".zshrc"),
        home_path.join(".zprofile"),
    ];

    #[cfg(target_os = "windows")]
    let config_files = vec![
        // Windows typically uses system environment variables
        // PowerShell profiles are more complex and less commonly used for this
    ];

    Some(config_files)
}

/// Parse environment variable from a shell configuration file
fn parse_env_from_file(file_path: &PathBuf, var_name: &str) -> Option<String> {
    // Check if file exists and is readable
    if !file_path.exists() {
        return None;
    }

    let content = fs::read_to_string(file_path).ok()?;

    // Parse the file line by line
    // We want the LAST occurrence of the variable (like shell behavior)
    let mut result = None;

    for line in content.lines() {
        let trimmed = line.trim();

        // Skip comments
        if trimmed.starts_with('#') {
            continue;
        }

        // Look for export statements: export VAR_NAME=value
        if let Some(value) = parse_export_line(trimmed, var_name) {
            result = Some(value);
        }
    }

    result
}

/// Parse a single export line and extract the value if it matches the variable name
fn parse_export_line(line: &str, var_name: &str) -> Option<String> {
    // Match patterns like:
    // export VAR_NAME=value
    // export VAR_NAME="value"
    // export VAR_NAME='value'
    // export VAR_NAME=$HOME/path

    let line = line.trim();

    // Check if line starts with export
    let without_export = if line.starts_with("export ") {
        line.strip_prefix("export ")?.trim()
    } else {
        // Also support lines without explicit export keyword
        line
    };

    // Check if this is the variable we're looking for
    let target = format!("{}=", var_name);
    if !without_export.starts_with(&target) {
        return None;
    }

    // Extract the value part
    let value_part = without_export.strip_prefix(&target)?.trim();

    // Remove quotes and expand variables
    let cleaned_value = clean_and_expand_value(value_part)?;

    // Only return non-empty values
    if cleaned_value.is_empty() {
        None
    } else {
        Some(cleaned_value)
    }
}

/// Clean quotes and expand environment variables in the value
fn clean_and_expand_value(value: &str) -> Option<String> {
    let value = value.trim();

    if value.is_empty() {
        return None;
    }

    // Remove surrounding quotes
    let unquoted = if (value.starts_with('"') && value.ends_with('"'))
        || (value.starts_with('\'') && value.ends_with('\''))
    {
        &value[1..value.len() - 1]
    } else {
        value
    };

    // Expand common environment variables
    let expanded = expand_env_vars(unquoted);

    Some(expanded)
}

/// Expand environment variables like $HOME, $USERPROFILE, etc.
fn expand_env_vars(value: &str) -> String {
    let mut result = value.to_string();

    // Expand $HOME
    if let Ok(home) = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")) {
        result = result.replace("$HOME", &home);
        result = result.replace("${HOME}", &home);
        result = result.replace("$USERPROFILE", &home);
        result = result.replace("${USERPROFILE}", &home);
    }

    // Expand $USER
    if let Ok(user) = std::env::var("USER").or_else(|_| std::env::var("USERNAME")) {
        result = result.replace("$USER", &user);
        result = result.replace("${USER}", &user);
        result = result.replace("$USERNAME", &user);
        result = result.replace("${USERNAME}", &user);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_export_line() {
        assert_eq!(
            parse_export_line(
                "export OPENCODE_CONFIG=/path/to/config.json",
                "OPENCODE_CONFIG"
            ),
            Some("/path/to/config.json".to_string())
        );

        assert_eq!(
            parse_export_line(
                "export OPENCODE_CONFIG=\"/path/to/config.json\"",
                "OPENCODE_CONFIG"
            ),
            Some("/path/to/config.json".to_string())
        );

        assert_eq!(
            parse_export_line(
                "export OPENCODE_CONFIG='/path/to/config.json'",
                "OPENCODE_CONFIG"
            ),
            Some("/path/to/config.json".to_string())
        );

        assert_eq!(
            parse_export_line("OPENCODE_CONFIG=/path/to/config.json", "OPENCODE_CONFIG"),
            Some("/path/to/config.json".to_string())
        );

        assert_eq!(
            parse_export_line("export OTHER_VAR=value", "OPENCODE_CONFIG"),
            None
        );
    }

    #[test]
    fn test_clean_and_expand_value() {
        assert_eq!(
            clean_and_expand_value("\"/path/to/file\""),
            Some("/path/to/file".to_string())
        );

        assert_eq!(
            clean_and_expand_value("'/path/to/file'"),
            Some("/path/to/file".to_string())
        );

        assert_eq!(
            clean_and_expand_value("/path/to/file"),
            Some("/path/to/file".to_string())
        );

        assert_eq!(clean_and_expand_value("\"\""), None);
    }
}
