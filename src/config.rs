//! Configuration resolution for zellij-tools plugin.
//!
//! This module handles resolving configuration paths, including:
//! - Reading environment variables from `/host/proc/self/environ`
//! - Finding the zellij config directory following zellij's search order
//! - Resolving relative include paths

use std::collections::HashMap;
use std::path::PathBuf;

/// Read environment variables from /host/proc/self/environ.
/// This file contains null-separated KEY=VALUE pairs.
pub fn read_host_environ() -> HashMap<String, String> {
    read_environ_from_path("/host/proc/self/environ")
}

/// Read environment variables from a given path (for testing).
fn read_environ_from_path(path: &str) -> HashMap<String, String> {
    let mut env = HashMap::new();

    if let Ok(contents) = std::fs::read_to_string(path) {
        env = parse_environ(&contents);
    }

    env
}

/// Parse null-separated environment variables.
pub fn parse_environ(contents: &str) -> HashMap<String, String> {
    let mut env = HashMap::new();

    for var in contents.split('\0') {
        if let Some((key, value)) = var.split_once('=') {
            env.insert(key.to_string(), value.to_string());
        }
    }

    env
}

/// Get the zellij config directory following zellij's search order.
///
/// Search order:
/// 1. `ZELLIJ_CONFIG_DIR` environment variable
/// 2. `$XDG_CONFIG_HOME/zellij` (if XDG_CONFIG_HOME is set)
/// 3. `$HOME/.config/zellij`
/// 4. `/etc/zellij` (system fallback)
pub fn get_zellij_config_dir() -> PathBuf {
    let env = read_host_environ();
    get_zellij_config_dir_with_env(&env)
}

/// Get the zellij config directory using provided environment variables (for testing).
pub fn get_zellij_config_dir_with_env(env: &HashMap<String, String>) -> PathBuf {
    // 1. Try ZELLIJ_CONFIG_DIR
    if let Some(config_dir) = env.get("ZELLIJ_CONFIG_DIR") {
        return PathBuf::from(config_dir);
    }

    // 2. Try $XDG_CONFIG_HOME/zellij
    if let Some(xdg_config) = env.get("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg_config).join("zellij");
    }

    // 3. Try $HOME/.config/zellij (Linux default)
    if let Some(home) = env.get("HOME") {
        return PathBuf::from(home).join(".config").join("zellij");
    }

    // 4. System fallback
    PathBuf::from("/etc/zellij")
}

/// Detect the home directory from environment variables.
pub fn detect_home_dir() -> Option<String> {
    let env = read_host_environ();
    detect_home_dir_with_env(&env)
}

/// Detect the home directory using provided environment variables (for testing).
pub fn detect_home_dir_with_env(env: &HashMap<String, String>) -> Option<String> {
    env.get("HOME").cloned()
}

/// Resolve an include path to an absolute path.
///
/// - Absolute paths (starting with `/`) are returned as-is
/// - Paths starting with `~` are expanded to the home directory
/// - Relative paths are resolved against the provided `config_dir` or the auto-detected config directory
///
/// Note: This function assumes `/host` is mounted to `/` for reading environment variables.
pub fn resolve_include_path(path: &str, config_dir: Option<&str>) -> PathBuf {
    let env = read_host_environ();
    resolve_include_path_with_env(path, config_dir, &env)
}

/// Resolve an include path using provided environment variables (for testing).
pub fn resolve_include_path_with_env(
    path: &str,
    config_dir: Option<&str>,
    env: &HashMap<String, String>,
) -> PathBuf {
    let path = path.trim();

    // Absolute path - use as-is
    if path.starts_with('/') {
        return PathBuf::from(path);
    }

    let home_dir = detect_home_dir_with_env(env);

    // Expand ~ to home directory
    if path.starts_with('~') {
        if let Some(ref home) = home_dir {
            let expanded = path.replacen('~', home, 1);
            return PathBuf::from(expanded);
        }
        // If HOME not available, return path as-is (will likely fail later)
        return PathBuf::from(path);
    }

    // Relative path - resolve against config directory
    let base_dir = if let Some(dir) = config_dir {
        // User-provided config_dir (resolve ~ in it too)
        if dir.starts_with('~') {
            if let Some(ref home) = home_dir {
                PathBuf::from(dir.replacen('~', home, 1))
            } else {
                PathBuf::from(dir)
            }
        } else {
            PathBuf::from(dir)
        }
    } else {
        // Auto-detect config directory
        get_zellij_config_dir_with_env(env)
    };

    base_dir.join(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_env(vars: &[(&str, &str)]) -> HashMap<String, String> {
        vars.iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn parse_environ_basic() {
        let contents = "HOME=/home/user\0USER=user\0PATH=/bin";
        let env = parse_environ(contents);

        assert_eq!(env.get("HOME"), Some(&"/home/user".to_string()));
        assert_eq!(env.get("USER"), Some(&"user".to_string()));
        assert_eq!(env.get("PATH"), Some(&"/bin".to_string()));
    }

    #[test]
    fn parse_environ_empty() {
        let env = parse_environ("");
        assert!(env.is_empty());
    }

    #[test]
    fn parse_environ_with_equals_in_value() {
        let contents = "FOO=bar=baz\0";
        let env = parse_environ(contents);
        assert_eq!(env.get("FOO"), Some(&"bar=baz".to_string()));
    }

    // get_zellij_config_dir_with_env tests

    #[test]
    fn config_dir_uses_zellij_config_dir() {
        let env = make_env(&[
            ("ZELLIJ_CONFIG_DIR", "/custom/zellij"),
            ("HOME", "/home/user"),
        ]);
        assert_eq!(
            get_zellij_config_dir_with_env(&env),
            PathBuf::from("/custom/zellij")
        );
    }

    #[test]
    fn config_dir_uses_xdg_config_home() {
        let env = make_env(&[
            ("XDG_CONFIG_HOME", "/custom/config"),
            ("HOME", "/home/user"),
        ]);
        assert_eq!(
            get_zellij_config_dir_with_env(&env),
            PathBuf::from("/custom/config/zellij")
        );
    }

    #[test]
    fn config_dir_uses_home() {
        let env = make_env(&[("HOME", "/home/user")]);
        assert_eq!(
            get_zellij_config_dir_with_env(&env),
            PathBuf::from("/home/user/.config/zellij")
        );
    }

    #[test]
    fn config_dir_falls_back_to_etc() {
        let env = make_env(&[]);
        assert_eq!(
            get_zellij_config_dir_with_env(&env),
            PathBuf::from("/etc/zellij")
        );
    }

    // resolve_include_path_with_env tests

    #[test]
    fn resolve_absolute_path() {
        let env = make_env(&[("HOME", "/home/user")]);
        assert_eq!(
            resolve_include_path_with_env("/absolute/path.kdl", None, &env),
            PathBuf::from("/absolute/path.kdl")
        );
    }

    #[test]
    fn resolve_tilde_path() {
        let env = make_env(&[("HOME", "/home/user")]);
        assert_eq!(
            resolve_include_path_with_env("~/config/file.kdl", None, &env),
            PathBuf::from("/home/user/config/file.kdl")
        );
    }

    #[test]
    fn resolve_relative_path_with_home() {
        let env = make_env(&[("HOME", "/home/user")]);
        assert_eq!(
            resolve_include_path_with_env("file.kdl", None, &env),
            PathBuf::from("/home/user/.config/zellij/file.kdl")
        );
    }

    #[test]
    fn resolve_relative_path_with_zellij_config_dir() {
        let env = make_env(&[
            ("ZELLIJ_CONFIG_DIR", "/custom/zellij"),
            ("HOME", "/home/user"),
        ]);
        assert_eq!(
            resolve_include_path_with_env("file.kdl", None, &env),
            PathBuf::from("/custom/zellij/file.kdl")
        );
    }

    #[test]
    fn resolve_relative_path_with_explicit_config_dir() {
        let env = make_env(&[("HOME", "/home/user")]);
        assert_eq!(
            resolve_include_path_with_env("file.kdl", Some("/explicit/dir"), &env),
            PathBuf::from("/explicit/dir/file.kdl")
        );
    }

    #[test]
    fn resolve_relative_path_with_tilde_config_dir() {
        let env = make_env(&[("HOME", "/home/user")]);
        assert_eq!(
            resolve_include_path_with_env("file.kdl", Some("~/myconfig"), &env),
            PathBuf::from("/home/user/myconfig/file.kdl")
        );
    }

    #[test]
    fn resolve_path_trims_whitespace() {
        let env = make_env(&[("HOME", "/home/user")]);
        assert_eq!(
            resolve_include_path_with_env("  file.kdl  ", None, &env),
            PathBuf::from("/home/user/.config/zellij/file.kdl")
        );
    }

    #[test]
    fn resolve_dotslash_relative_path() {
        let env = make_env(&[("HOME", "/home/user")]);
        assert_eq!(
            resolve_include_path_with_env("./file.kdl", None, &env),
            PathBuf::from("/home/user/.config/zellij/./file.kdl")
        );
    }
}
