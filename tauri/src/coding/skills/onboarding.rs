use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};

use super::central_repo::resolve_central_repo_path;
use super::content_hash::hash_dir;
use super::skill_store;
use super::tool_adapters::{get_all_tool_adapters, RuntimeToolAdapter};
use super::types::{OnboardingGroup, OnboardingPlan, OnboardingVariant};
use crate::DbState;

/// Extra skill source directories to scan during onboarding discovery.
/// These are third-party skill stores outside the built-in tool adapters.
/// To add a new source, just append an entry here.
struct ExtraSkillSource {
    key: &'static str,
    display_name: &'static str,
    /// Path to the skills directory (supports ~/ and %APPDATA%/ prefixes)
    skills_dir: &'static str,
}

const EXTRA_SKILL_SOURCES: &[ExtraSkillSource] = &[ExtraSkillSource {
    key: "cc_switch",
    display_name: "CC Switch",
    skills_dir: "~/.cc-switch/skills",
}];

/// Build an onboarding plan by scanning installed tools for existing skills
pub async fn build_onboarding_plan(
    app: &tauri::AppHandle,
    state: &DbState,
) -> Result<OnboardingPlan> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("failed to resolve home directory"))?;
    let central = resolve_central_repo_path(app, state).await?;

    // Get custom tools
    let custom_tools = skill_store::get_custom_tools(state)
        .await
        .unwrap_or_default();

    // Get already managed target paths to exclude them
    let managed_targets = skill_store::list_all_skill_target_paths(state)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|(tool, path)| managed_target_key(&tool, Path::new(&path)))
        .collect::<std::collections::HashSet<_>>();

    // Run the blocking file system operations in a dedicated thread pool
    // to avoid blocking the tokio async runtime
    tokio::task::spawn_blocking(move || {
        build_onboarding_plan_in_home(&home, Some(&central), Some(&managed_targets), &custom_tools)
    })
    .await
    .map_err(|e| anyhow::anyhow!("spawn_blocking failed: {}", e))?
}

fn build_onboarding_plan_in_home(
    _home: &Path,
    exclude_root: Option<&Path>,
    exclude_managed_targets: Option<&std::collections::HashSet<String>>,
    custom_tools: &[super::types::CustomTool],
) -> Result<OnboardingPlan> {
    // Get all adapters (built-in + custom)
    let adapters = get_all_tool_adapters(custom_tools);
    let mut all_detected: Vec<super::types::DetectedSkill> = Vec::new();
    let mut scanned = 0usize;

    for adapter in &adapters {
        // Check if tool is installed using path_utils
        let detect_path =
            crate::coding::tools::path_utils::resolve_storage_path(&adapter.relative_detect_dir);
        if detect_path.is_none() || !detect_path.as_ref().unwrap().exists() {
            continue;
        }
        scanned += 1;
        // Resolve skills directory using path_utils to handle ~/  and %APPDATA%/ paths correctly
        let dir =
            crate::coding::tools::path_utils::resolve_storage_path(&adapter.relative_skills_dir);
        if let Some(skills_dir) = dir {
            let detected = scan_runtime_tool_dir(adapter, &skills_dir)?;
            all_detected.extend(filter_detected(
                detected,
                exclude_root,
                exclude_managed_targets,
            ));
        }
    }

    // Scan extra skill directories (third-party skill stores)
    for source in EXTRA_SKILL_SOURCES {
        let skills_dir = crate::coding::tools::path_utils::resolve_storage_path(source.skills_dir);
        if let Some(dir) = skills_dir {
            if dir.exists() {
                let adapter = RuntimeToolAdapter {
                    key: source.key.to_string(),
                    display_name: source.display_name.to_string(),
                    relative_skills_dir: source.skills_dir.to_string(),
                    relative_detect_dir: source.skills_dir.to_string(),
                    is_custom: false,
                    force_copy: false,
                };
                scanned += 1;
                let detected = scan_runtime_tool_dir(&adapter, &dir)?;
                all_detected.extend(filter_detected(
                    detected,
                    exclude_root,
                    exclude_managed_targets,
                ));
            }
        }
    }

    let mut grouped: HashMap<String, Vec<OnboardingVariant>> = HashMap::new();
    for skill in all_detected.iter() {
        let fingerprint = hash_dir(&skill.path).ok();
        let entry = grouped.entry(skill.name.clone()).or_default();
        entry.push(OnboardingVariant {
            tool: skill.tool.clone(),
            name: skill.name.clone(),
            path: skill.path.to_string_lossy().to_string(),
            fingerprint,
            is_link: skill.is_link,
            link_target: skill
                .link_target
                .as_ref()
                .map(|p| p.to_string_lossy().to_string()),
            conflicting_tools: Vec::new(), // Will be calculated later
        });
    }

    let groups: Vec<OnboardingGroup> = grouped
        .into_iter()
        .map(|(name, mut variants)| {
            // Build fingerprint -> tools mapping (owned data to avoid borrow conflict)
            let mut fingerprint_tools: HashMap<String, Vec<String>> = HashMap::new();
            for v in &variants {
                if let Some(ref fp) = v.fingerprint {
                    fingerprint_tools
                        .entry(fp.clone())
                        .or_default()
                        .push(v.tool.clone());
                }
            }

            let uniq_fingerprints = fingerprint_tools.len();
            let has_conflict = uniq_fingerprints > 1;

            // Calculate conflicting tools for each variant
            for v in &mut variants {
                if let Some(ref my_fp) = v.fingerprint {
                    // Find tools with different fingerprints
                    let mut conflicting: Vec<String> = Vec::new();
                    for (fp, tools) in &fingerprint_tools {
                        if fp != my_fp {
                            conflicting.extend(tools.iter().cloned());
                        }
                    }
                    v.conflicting_tools = conflicting;
                }
            }

            OnboardingGroup {
                name,
                has_conflict,
                variants,
            }
        })
        .collect();

    Ok(OnboardingPlan {
        total_tools_scanned: scanned,
        total_skills_found: all_detected.len(),
        groups,
    })
}

fn filter_detected(
    detected: Vec<super::types::DetectedSkill>,
    exclude_root: Option<&Path>,
    exclude_managed_targets: Option<&std::collections::HashSet<String>>,
) -> Vec<super::types::DetectedSkill> {
    if exclude_root.is_none() && exclude_managed_targets.is_none() {
        return detected;
    }
    detected
        .into_iter()
        .filter(|skill| {
            if let Some(exclude_root) = exclude_root {
                if is_under(&skill.path, exclude_root) {
                    return false;
                }
                if let Some(target) = &skill.link_target {
                    if is_under(target, exclude_root) {
                        return false;
                    }
                }
            }
            if let Some(exclude) = exclude_managed_targets {
                if exclude.contains(&managed_target_key(&skill.tool, &skill.path)) {
                    return false;
                }
            }
            true
        })
        .collect()
}

fn is_under(path: &Path, base: &Path) -> bool {
    path.starts_with(base)
}

fn managed_target_key(tool: &str, path: &Path) -> String {
    let tool = tool.to_ascii_lowercase();
    let normalized = normalize_path_for_key(path);
    format!("{tool}\n{normalized}")
}

fn normalize_path_for_key(path: &Path) -> String {
    let normalized: std::path::PathBuf = path.components().collect();
    let s = normalized.to_string_lossy().to_string();
    #[cfg(windows)]
    {
        s.to_lowercase()
    }
    #[cfg(not(windows))]
    {
        s
    }
}

/// Scan a tool directory for skills (using RuntimeToolAdapter)
fn scan_runtime_tool_dir(
    adapter: &RuntimeToolAdapter,
    dir: &Path,
) -> Result<Vec<super::types::DetectedSkill>> {
    let mut results = Vec::new();
    if !dir.exists() {
        return Ok(results);
    }

    // Ignore paths containing our central repo
    let ignore_hint = "Application Support/com.ai-toolbox/skills";

    for entry in std::fs::read_dir(dir).with_context(|| format!("read dir {:?}", dir))? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        let is_dir = file_type.is_dir() || (file_type.is_symlink() && path.is_dir());
        if !is_dir {
            continue;
        }

        let name = entry.file_name().to_string_lossy().to_string();
        // Skip system directories for codex
        if adapter.key == "codex" && name == ".system" {
            continue;
        }

        let (is_link, link_target) = detect_link(&path);
        if path.to_string_lossy().contains(ignore_hint)
            || link_target
                .as_ref()
                .map(|p| p.to_string_lossy().contains(ignore_hint))
                .unwrap_or(false)
        {
            continue;
        }

        results.push(super::types::DetectedSkill {
            tool: adapter.key.clone(),
            name,
            path,
            is_link,
            link_target,
        });
    }

    Ok(results)
}

fn detect_link(path: &Path) -> (bool, Option<std::path::PathBuf>) {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            let target = std::fs::read_link(path).ok();
            (true, target)
        }
        _ => {
            let target = std::fs::read_link(path).ok();
            if target.is_some() {
                (true, target)
            } else {
                (false, None)
            }
        }
    }
}
