use super::types::{FileMapping, SyncResult, WSLDetectResult};
use std::path::Path;
use std::process::Command;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

/// Windows CREATE_NO_WINDOW flag to prevent console window from appearing
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Create a WSL command with proper flags for Windows GUI apps
/// This prevents console windows from flashing when running in release mode
fn create_wsl_command() -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new("wsl");
    #[cfg(target_os = "windows")]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// Check if bytes look like UTF-16 LE encoding.
///
/// WSL on Windows outputs UTF-16 LE for commands like `wsl --list`.
/// UTF-16 LE for ASCII text shows a pattern: each ASCII byte is followed by 0x00.
/// We check for BOM (FF FE) or sample multiple byte pairs to confirm the pattern.
fn looks_like_utf16_le(bytes: &[u8]) -> bool {
    if bytes.len() < 2 {
        return false;
    }

    // Check for UTF-16 LE BOM
    if bytes[0] == 0xFF && bytes[1] == 0xFE {
        return true;
    }

    // Sample up to 8 byte pairs: for ASCII-heavy content, every odd byte should be 0x00
    let sample_count = (bytes.len() / 2).min(8);
    if sample_count < 2 {
        return false;
    }

    let zero_count = bytes[..sample_count * 2]
        .chunks_exact(2)
        .filter(|pair| pair[0] != 0 && pair[1] == 0)
        .count();

    // If most sampled pairs match the "ASCII + 0x00" pattern, it's UTF-16 LE
    zero_count * 2 >= sample_count
}

/// Decode WSL command output which may be UTF-16 LE (Windows) or UTF-8 (Linux)
/// Also strips null characters to prevent SurrealDB panics
fn decode_wsl_output(bytes: &[u8]) -> String {
    let result = if looks_like_utf16_le(bytes) {
        let utf16_data: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
        String::from_utf16_lossy(&utf16_data)
    } else {
        String::from_utf8_lossy(bytes).to_string()
    };
    // Strip null characters to prevent SurrealDB panics
    result.replace('\0', "")
}

fn bash_single_quote(input: &str) -> String {
    format!("'{}'", input.replace('\'', "'\"'\"'"))
}

/// Get the effective distro to use: if configured distro doesn't exist,
/// try to find a matching one or use the first available distro
pub fn get_effective_distro(configured_distro: &str) -> Result<String, String> {
    let distros = get_wsl_distros()?;

    if distros.is_empty() {
        return Err("No WSL distros available".to_string());
    }

    // Check if configured distro exists exactly
    if distros.iter().any(|d| d == configured_distro) {
        return Ok(configured_distro.to_string());
    }

    // Try to find a distro that starts with the configured name (e.g., "Ubuntu" matches "Ubuntu-22.04")
    if let Some(matching) = distros.iter().find(|d| d.starts_with(configured_distro)) {
        log::info!(
            "WSL distro '{}' not found, using '{}' instead",
            configured_distro,
            matching
        );
        return Ok(matching.clone());
    }

    // Try to find a distro where configured name starts with it (e.g., "Ubuntu-22.04" matches "Ubuntu")
    if let Some(matching) = distros
        .iter()
        .find(|d| configured_distro.starts_with(d.as_str()))
    {
        log::info!(
            "WSL distro '{}' not found, using '{}' instead",
            configured_distro,
            matching
        );
        return Ok(matching.clone());
    }

    // Fall back to first available distro
    let first = distros.first().unwrap().clone();
    log::warn!(
        "WSL distro '{}' not found, falling back to '{}'",
        configured_distro,
        first
    );
    Ok(first)
}

/// Detect if WSL is available and get list of distros
pub fn detect_wsl() -> WSLDetectResult {
    // Check if WSL is installed by running wsl --status
    let output = create_wsl_command().args(["--status"]).output();

    match output {
        Ok(result) => {
            if result.status.success() {
                // WSL is available, get distros
                match get_wsl_distros() {
                    Ok(distros) => WSLDetectResult {
                        available: true,
                        distros,
                        error: None,
                    },
                    Err(e) => WSLDetectResult {
                        available: true,
                        distros: vec![],
                        error: Some(e),
                    },
                }
            } else {
                WSLDetectResult {
                    available: false,
                    distros: vec![],
                    error: Some("WSL command failed".to_string()),
                }
            }
        }
        Err(e) => WSLDetectResult {
            available: false,
            distros: vec![],
            error: Some(format!("Failed to run WSL command: {}", e)),
        },
    }
}

/// Get list of available WSL distros
pub fn get_wsl_distros() -> Result<Vec<String>, String> {
    let output = create_wsl_command()
        .args(["--list", "--quiet"])
        .output()
        .map_err(|e| format!("Failed to run wsl --list: {}", e))?;

    if !output.status.success() {
        return Err("WSL list command failed".to_string());
    }

    let stdout = decode_wsl_output(&output.stdout);

    let distros: Vec<String> = stdout
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect();

    Ok(distros)
}

/// Get running state of a specific WSL distro
/// Returns: "Running", "Stopped", or "Unknown"
pub fn get_wsl_distro_state(distro: &str) -> String {
    let output = match create_wsl_command().args(["--list", "--verbose"]).output() {
        Ok(o) => o,
        Err(_) => return "Unknown".to_string(),
    };

    if !output.status.success() {
        return "Unknown".to_string();
    }

    let stdout = decode_wsl_output(&output.stdout);

    // Parse output to find the distro's state
    // Format: "  NAME                   STATE           VERSION"
    //         "* Ubuntu                 Running         2"
    let mut is_header_skipped = false;
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Skip header line (starts with "NAME")
        if !is_header_skipped {
            let trimmed = line.trim_start();
            if trimmed.starts_with("NAME") {
                is_header_skipped = true;
            }
            continue;
        }

        // Parse line: [*] NAME STATE VERSION
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 3 {
            continue;
        }

        // Get name and state from the line
        // Format: NAME STATE VERSION or * NAME STATE VERSION
        let (name, state) = if parts.len() >= 4 && parts[0] == "*" {
            // Default distro with * prefix
            (parts[1], parts[2])
        } else {
            // Normal distro
            (parts[0], parts[1])
        };

        if name == distro {
            // Normalize state to "Running" or "Stopped"
            return match state.to_lowercase().as_str() {
                "running" => "Running".to_string(),
                _ => "Stopped".to_string(),
            };
        }
    }

    "Unknown".to_string()
}

/// Expand environment variables in a path
pub fn expand_env_vars(path: &str) -> Result<String, String> {
    let mut result = path.to_string();

    // Common Windows environment variables
    let vars = [
        ("USERPROFILE", std::env::var("USERPROFILE")),
        ("APPDATA", std::env::var("APPDATA")),
        ("LOCALAPPDATA", std::env::var("LOCALAPPDATA")),
        (
            "HOME",
            std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")),
        ),
    ];

    for (var, value) in vars {
        if let Ok(val) = value {
            result = result.replace(&format!("%{}%", var), &val);
            result = result.replace(&format!("${}", var), &val);
        }
    }

    Ok(result)
}

/// Convert Windows path to WSL path
pub fn windows_to_wsl_path(windows_path: &str) -> Result<String, String> {
    let expanded = expand_env_vars(windows_path)?;

    // Convert C:\Users\... to /mnt/c/Users/...
    let wsl_path = expanded.replace('\\', "/");

    // Convert drive letter (e.g., C: to /mnt/c)
    if wsl_path.len() >= 2 && wsl_path.as_bytes()[1] == b':' {
        let drive = wsl_path.chars().next().unwrap().to_lowercase();
        let rest = &wsl_path[2..];
        // rest starts with "/" (e.g., "/Users/..."), so no extra "/" needed
        return Ok(format!("/mnt/{}{}", drive, rest));
    }

    Ok(wsl_path)
}

/// Sync a single file mapping to WSL
pub fn sync_file_mapping(mapping: &FileMapping, distro: &str) -> Result<Vec<String>, String> {
    let windows_path = expand_env_vars(&mapping.windows_path)?;

    if mapping.is_directory {
        // Directory mode: copy entire directory
        if !Path::new(&windows_path).exists() {
            return Ok(vec![]);
        }
        sync_directory(&windows_path, &mapping.wsl_path, distro)
    } else if mapping.is_pattern {
        // Pattern mode: handle wildcards
        sync_pattern_files(&windows_path, &mapping.wsl_path, distro)
    } else {
        // Single file mode
        if !Path::new(&windows_path).exists() {
            return Ok(vec![]);
        }
        sync_single_file(&windows_path, &mapping.wsl_path, distro)
    }
}

/// Sync a single file
pub fn sync_single_file(
    windows_path: &str,
    wsl_path: &str,
    distro: &str,
) -> Result<Vec<String>, String> {
    let wsl_source_path = windows_to_wsl_path(windows_path)?;

    // Expand ~ in WSL path
    let wsl_target_path = wsl_path.replace("~", "$HOME");

    // Create the WSL command
    let command = format!(
        "target={}; source={}; mkdir -p \"$(dirname \"$target\")\" && cp -f \"$source\" \"$target\"",
        bash_single_quote(&wsl_target_path),
        bash_single_quote(&wsl_source_path)
    );

    let output = create_wsl_command()
        .args(["-d", distro, "--exec", "bash", "-c", &command])
        .output()
        .map_err(|e| format!("Failed to execute WSL command: {}", e))?;

    if output.status.success() {
        Ok(vec![format!("{} -> {}", windows_path, wsl_path)])
    } else {
        let stderr = decode_wsl_output(&output.stderr);
        Err(format!("WSL sync failed: {}", stderr.trim()))
    }
}

/// Sync a directory (recursive copy)
pub fn sync_directory(
    windows_path: &str,
    wsl_path: &str,
    distro: &str,
) -> Result<Vec<String>, String> {
    let wsl_source_path = windows_to_wsl_path(windows_path)?;

    // Expand ~ in WSL path
    let wsl_target_path = wsl_path.replace("~", "$HOME");

    // First, check if source path exists in WSL
    let check_command = format!(
        "source={}; if [ -e \"$source\" ]; then echo exists; else echo notfound; fi",
        bash_single_quote(&wsl_source_path)
    );
    let check_output = create_wsl_command()
        .args(["-d", distro, "--exec", "bash", "-c", &check_command])
        .output()
        .map_err(|e| format!("Failed to check WSL source path: {}", e))?;

    let check_result = decode_wsl_output(&check_output.stdout).trim().to_string();
    if check_result == "notfound" {
        let source_path_expanded = std::path::Path::new(windows_path);
        if source_path_expanded.exists() {
            return Err(format!(
                "WSL directory sync failed: Windows path '{}' does not exist or is not accessible from WSL. \
                 Converted WSL path: '{}'. Please check if WSL can access Windows drives.",
                windows_path, wsl_source_path
            ));
        } else {
            return Ok(vec![]); // Source doesn't exist, skip sync
        }
    }

    // Create the WSL command to copy directory recursively
    // Use cp -rL to copy directory contents and dereference symlinks
    // -L flag ensures symlinks are followed and actual file contents are copied
    // This is important because Windows skills may be managed via symlinks/hardlinks
    let command = format!(
        "target={}; source={}; mkdir -p \"$(dirname \"$target\")\" && rm -rf \"$target\" && cp -rL \"$source\" \"$target\" 2>&1",
        bash_single_quote(&wsl_target_path),
        bash_single_quote(&wsl_source_path)
    );

    let output = create_wsl_command()
        .args(["-d", distro, "--exec", "bash", "-c", &command])
        .output()
        .map_err(|e| format!("Failed to execute WSL directory command: {}", e))?;

    if output.status.success() {
        Ok(vec![format!("{} -> {}", windows_path, wsl_path)])
    } else {
        let stderr = decode_wsl_output(&output.stderr).trim().to_string();
        let stdout = decode_wsl_output(&output.stdout).trim().to_string();
        let exit_code = output.status.code().unwrap_or(-1);

        // Provide more detailed error information
        if stderr.is_empty() && stdout.is_empty() {
            Err(format!(
                "WSL directory sync failed: Command returned exit code {} but produced no output. \
                 Source: '{}', Target WSL: '{}', WSL converted source: '{}'",
                exit_code, windows_path, wsl_target_path, wsl_source_path
            ))
        } else if !stderr.is_empty() {
            Err(format!(
                "WSL directory sync failed: {}. Source: '{}', Target: '{}', Exit code: {}",
                stderr, windows_path, wsl_path, exit_code
            ))
        } else {
            Err(format!(
                "WSL directory sync failed: {}. Source: '{}', Target: '{}', Exit code: {}",
                stdout, windows_path, wsl_path, exit_code
            ))
        }
    }
}

/// Sync files matching a pattern
pub fn sync_pattern_files(
    windows_pattern: &str,
    wsl_target_dir: &str,
    distro: &str,
) -> Result<Vec<String>, String> {
    // Convert Windows path to WSL path
    let wsl_source_dir = windows_to_wsl_path(windows_pattern)?;

    // Extract the directory and pattern
    let (wsl_source_base, pattern) = if let Some(last_slash) = wsl_source_dir.rfind('/') {
        let base = &wsl_source_dir[..last_slash];
        let pattern = &wsl_source_dir[last_slash + 1..];
        (base, pattern)
    } else {
        (".", &wsl_source_dir[..])
    };

    // Expand ~ in WSL path
    let wsl_target_dir_expanded = wsl_target_dir.replace("~", "$HOME");

    // Create the WSL command to sync pattern files
    let command = format!(
        "src_base={}; pattern={}; target={}; \
         mkdir -p \"$target\" && \
         if [ -f \"$src_base/$pattern\" ]; then \
             cp -f \"$src_base/$pattern\" \"$target\" && echo \"synced\"; \
         else \
             shopt -s nullglob dotglob; \
             files=(\"$src_base\"/$pattern); \
             if [ ${{#files[@]}} -gt 0 ]; then \
                 cp -f \"${{files[@]}}\" \"$target\" 2>/dev/null && echo \"synced\" || true; \
             fi; \
         fi",
        bash_single_quote(wsl_source_base),
        bash_single_quote(pattern),
        bash_single_quote(&wsl_target_dir_expanded)
    );

    let output = create_wsl_command()
        .args(["-d", distro, "--exec", "bash", "-c", &command])
        .output()
        .map_err(|e| format!("Failed to execute WSL pattern command: {}", e))?;

    if output.status.success() {
        Ok(vec![format!("{} -> {}", windows_pattern, wsl_target_dir)])
    } else {
        let stderr = decode_wsl_output(&output.stderr).trim().to_string();
        let stdout = decode_wsl_output(&output.stdout).trim().to_string();
        let exit_code = output.status.code().unwrap_or(-1);

        // Pattern sync failures are often OK (just no files matching)
        if stderr.contains("cannot stat")
            || stderr.contains("No such file")
            || stderr.contains("No such file or directory")
        {
            Ok(vec![])
        } else if stderr.is_empty() && stdout.is_empty() {
            // Silent failure - might just be no files matching pattern
            Ok(vec![])
        } else if !stderr.is_empty() {
            Err(format!(
                "WSL pattern sync failed: {}. Pattern: '{}', Target: '{}', Exit code: {}",
                stderr, windows_pattern, wsl_target_dir, exit_code
            ))
        } else {
            Err(format!(
                "WSL pattern sync failed: {}. Pattern: '{}', Target: '{}', Exit code: {}",
                stdout, windows_pattern, wsl_target_dir, exit_code
            ))
        }
    }
}

/// Sync all enabled file mappings for a module (or all modules if module is None)
pub fn sync_mappings(
    mappings: &[FileMapping],
    distro: &str,
    module_filter: Option<&str>,
) -> SyncResult {
    let mut synced_files = vec![];
    let mut skipped_files = vec![];
    let mut errors = vec![];

    let filtered_mappings: Vec<_> = mappings
        .iter()
        .filter(|m| m.enabled)
        .filter(|m| module_filter.is_none() || Some(m.module.as_str()) == module_filter)
        .collect();

    for mapping in filtered_mappings {
        match sync_file_mapping(mapping, distro) {
            Ok(files) if files.is_empty() => {
                skipped_files.push(mapping.name.clone());
            }
            Ok(files) => {
                synced_files.extend(files);
            }
            Err(e) => {
                errors.push(format!("{}: {}", mapping.name, e));
            }
        }
    }

    SyncResult {
        success: errors.is_empty(),
        synced_files,
        skipped_files,
        errors,
    }
}

// ============================================================================
// WSL File Operations
// ============================================================================

/// Check if content looks like valid UTF-8 text config (not binary/corrupted/wrong encoding)
///
/// `read_wsl_file` uses `String::from_utf8_lossy`, which replaces invalid UTF-8 bytes
/// with U+FFFD (�). If the content contains replacement characters, it means the file
/// is not valid UTF-8 (likely GBK/GB2312 on Chinese Windows systems).
pub fn check_file_encoding(content: &str, file_path: &str) -> Result<(), String> {
    if content.contains('\u{FFFD}') {
        let msg = format!(
            "文件 {} 编码不是 UTF-8（可能是 GBK/GB2312），请手动转换后重试。\n\
             修复方法：\n\
             · WSL 中执行:  iconv -f GBK -t UTF-8 \"{}\" -o \"{}.tmp\" && mv \"{}.tmp\" \"{}\"\n\
             · Windows 中: 用 VS Code 打开文件 → 右下角点击编码 → 选择「通过编码重新打开」→ 选 GBK → 再选「通过编码保存」→ 选 UTF-8",
            file_path, file_path, file_path, file_path, file_path
        );
        log::warn!("{}", msg);
        return Err(msg);
    }

    // Check for binary/corrupted content: high ratio of non-printable characters
    let non_printable_count = content
        .chars()
        .take(256)
        .filter(|c| !c.is_ascii_graphic() && !c.is_ascii_whitespace())
        .count();
    let sample_len = content.chars().take(256).count().max(1);
    if non_printable_count * 10 >= sample_len {
        let msg = format!(
            "文件 {} 内容疑似二进制或已损坏，请检查文件内容是否正确",
            file_path
        );
        log::warn!("{}", msg);
        return Err(msg);
    }

    Ok(())
}

/// Read a file from WSL as raw string (no encoding check or conversion).
///
/// Uses `String::from_utf8_lossy` — suitable for files we control (hash files, etc.)
/// where encoding issues are not expected. For user-facing config files that may
/// have encoding problems (GBK, etc.), use `read_wsl_file` instead.
pub fn read_wsl_file_raw(distro: &str, wsl_path: &str) -> Result<String, String> {
    let wsl_target = wsl_path.replace("~", "$HOME");

    let command = format!(
        "if [ -f \"{}\" ]; then cat \"{}\"; else echo ''; fi",
        wsl_target, wsl_target
    );

    let output = create_wsl_command()
        .args(["-d", distro, "--exec", "bash", "-c", &command])
        .output()
        .map_err(|e| format!("Failed to read WSL file: {}", e))?;

    // Check for WSL errors (distro not found, etc.) - these come as UTF-16 on Windows
    if !output.status.success() {
        let stderr = decode_wsl_output(&output.stderr);
        if stderr.contains("WSL_E_DISTRO_NOT_FOUND") || stderr.contains("not found") {
            return Err(format!("WSL distro '{}' not found", distro));
        }
        return Err(format!("WSL command failed: {}", stderr.trim()));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Read a file from WSL, with automatic encoding detection and GBK-to-UTF-8 conversion.
///
/// Built on top of `read_wsl_file_raw`, adding encoding validation and auto-conversion.
/// Use this for user-facing config files (claude.json, opencode.json, config.toml, etc.)
///
/// Flow:
/// 1. Read raw content via `read_wsl_file_raw`
/// 2. Validate encoding via `check_file_encoding` — if valid UTF-8, return directly
/// 3. If non-UTF-8 detected, try iconv GBK→UTF-8
/// 4. If iconv succeeds and passes validation, return converted content
/// 5. If iconv fails, return error with instructions for the user
pub fn read_wsl_file(distro: &str, wsl_path: &str) -> Result<String, String> {
    let content = read_wsl_file_raw(distro, wsl_path)?;

    // Already valid UTF-8 — return directly
    if check_file_encoding(&content, wsl_path).is_ok() {
        return Ok(content);
    }

    // Non-UTF-8 detected, try iconv GBK→UTF-8 conversion
    log::warn!(
        "File {} is non-UTF-8, attempting iconv GBK→UTF-8...",
        wsl_path
    );

    let wsl_target = wsl_path.replace("~", "$HOME");
    let convert_command = format!("iconv -f GBK -t UTF-8 \"{}\" 2>/dev/null", wsl_target);

    let convert_output = create_wsl_command()
        .args(["-d", distro, "--exec", "bash", "-c", &convert_command])
        .output()
        .map_err(|e| format!("Failed to run iconv: {}", e))?;

    if !convert_output.status.success() {
        // iconv itself failed (command error)
        return Err(format!(
            "文件 {} 编码不是 UTF-8，自动转换失败。请手动转换后重试。\n\
             修复方法：\n\
             · WSL 中执行:  iconv -f GBK -t UTF-8 \"{}\" -o \"{}.tmp\" && mv \"{}.tmp\" \"{}\"\n\
             · Windows 中: 用 VS Code 打开文件 → 右下角点击编码 → 选择「通过编码重新打开」→ 选 GBK → 再选「通过编码保存」→ 选 UTF-8",
            wsl_path, wsl_path, wsl_path, wsl_path, wsl_path
        ));
    }

    let converted = String::from_utf8_lossy(&convert_output.stdout).to_string();

    // Verify conversion result
    if check_file_encoding(&converted, wsl_path).is_ok() {
        log::info!("Auto-converted {} from GBK to UTF-8", wsl_path);
        return Ok(converted);
    }

    // iconv ran but output is still not valid UTF-8 — encoding is not GBK either
    Err(format!(
        "文件 {} 编码不是 UTF-8 也不是 GBK，自动转换失败。请检查文件编码或内容是否损坏。",
        wsl_path
    ))
}

/// Write content to a WSL file
pub fn write_wsl_file(distro: &str, wsl_path: &str, content: &str) -> Result<(), String> {
    let wsl_target = wsl_path.replace("~", "$HOME");

    // Use heredoc to write content, avoiding escape issues
    let command = format!(
        "mkdir -p \"$(dirname \"{}\")\" && cat > \"{}\"",
        wsl_target, wsl_target
    );

    let mut child = create_wsl_command()
        .args(["-d", distro, "--exec", "bash", "-c", &command])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to spawn WSL command: {}", e))?;

    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin
            .write_all(content.as_bytes())
            .map_err(|e| format!("Failed to write to stdin: {}", e))?;
    }

    let status = child
        .wait()
        .map_err(|e| format!("Failed to wait for WSL command: {}", e))?;

    if status.success() {
        Ok(())
    } else {
        Err("WSL write command failed".to_string())
    }
}

/// Create a symlink in WSL
pub fn create_wsl_symlink(distro: &str, target: &str, link_path: &str) -> Result<(), String> {
    let target_expanded = target.replace("~", "$HOME");
    let link_expanded = link_path.replace("~", "$HOME");

    let command = format!(
        "mkdir -p \"$(dirname \"{}\")\" && rm -rf \"{}\" && ln -s \"{}\" \"{}\"",
        link_expanded, link_expanded, target_expanded, link_expanded
    );

    let output = create_wsl_command()
        .args(["-d", distro, "--exec", "bash", "-c", &command])
        .output()
        .map_err(|e| format!("Failed to create symlink: {}", e))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = decode_wsl_output(&output.stderr);
        Err(format!("WSL symlink failed: {}", stderr.trim()))
    }
}

/// Remove a file or directory in WSL
pub fn remove_wsl_path(distro: &str, wsl_path: &str) -> Result<(), String> {
    let wsl_target = wsl_path.replace("~", "$HOME");
    let command = format!("rm -rf \"{}\"", wsl_target);

    let output = create_wsl_command()
        .args(["-d", distro, "--exec", "bash", "-c", &command])
        .output()
        .map_err(|e| format!("Failed to remove WSL path: {}", e))?;

    if output.status.success() {
        Ok(())
    } else {
        let stderr = decode_wsl_output(&output.stderr);
        Err(format!("WSL remove failed: {}", stderr.trim()))
    }
}

/// List subdirectories in a WSL directory
pub fn list_wsl_dir(distro: &str, wsl_path: &str) -> Result<Vec<String>, String> {
    let wsl_target = wsl_path.replace("~", "$HOME");
    let command = format!(
        "if [ -d \"{}\" ]; then ls -1 \"{}\"; fi",
        wsl_target, wsl_target
    );

    let output = create_wsl_command()
        .args(["-d", distro, "--exec", "bash", "-c", &command])
        .output()
        .map_err(|e| format!("Failed to list WSL dir: {}", e))?;

    Ok(decode_wsl_output(&output.stdout)
        .lines()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .collect())
}

/// Check if a WSL symlink exists and points to the expected target
pub fn check_wsl_symlink_exists(distro: &str, link_path: &str, expected_target: &str) -> bool {
    let link_expanded = link_path.replace("~", "$HOME");
    let target_expanded = expected_target.replace("~", "$HOME");
    let command = format!(
        "[ -L \"{}\" ] && [ \"$(readlink \"{}\")\" = \"{}\" ] && echo yes || echo no",
        link_expanded, link_expanded, target_expanded
    );

    if let Ok(output) = create_wsl_command()
        .args(["-d", distro, "--exec", "bash", "-c", &command])
        .output()
    {
        decode_wsl_output(&output.stdout).trim() == "yes"
    } else {
        false
    }
}
