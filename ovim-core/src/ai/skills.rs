use serde::Deserialize;
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) const ACTIVATE_SKILL_TOOL: &str = "activate_skill";
pub(crate) const ACTIVATED_SKILL_MARKER: &str = "OVIM_SKILL_ACTIVATED:";

const MAX_SKILL_FILE_BYTES: u64 = 32 * 1024;
const MAX_SKILL_NAME_CHARS: usize = 64;
const MAX_SKILL_DESCRIPTION_CHARS: usize = 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub instructions: String,
    pub source: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillDiagnostic {
    pub source: PathBuf,
    pub message: String,
}

#[derive(Debug, Clone, Default)]
pub struct SkillCatalog {
    skills: BTreeMap<String, Skill>,
    diagnostics: Vec<SkillDiagnostic>,
}

impl SkillCatalog {
    pub fn discover() -> Self {
        Self::load_from_dir(&default_skills_dir())
    }

    pub fn load_from_dir(dir: &Path) -> Self {
        let mut catalog = Self::default();
        if !dir.exists() {
            return catalog;
        }
        if !dir.is_dir() {
            catalog.diagnostics.push(SkillDiagnostic {
                source: dir.to_path_buf(),
                message: "skills path is not a directory".into(),
            });
            return catalog;
        }

        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) => {
                catalog.diagnostics.push(SkillDiagnostic {
                    source: dir.to_path_buf(),
                    message: format!("failed to read skills directory: {error}"),
                });
                return catalog;
            }
        };
        let mut files = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.is_file()
                    && path
                        .extension()
                        .and_then(OsStr::to_str)
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
            })
            .collect::<Vec<_>>();
        files.sort();

        for path in files {
            match load_skill(&path) {
                Ok(skill) => {
                    if let Some(first) = catalog.skills.get(&skill.name) {
                        catalog.diagnostics.push(SkillDiagnostic {
                            source: path,
                            message: format!(
                                "duplicate skill name {:?}; first declared by {}",
                                skill.name,
                                first.source.display()
                            ),
                        });
                    } else {
                        catalog.skills.insert(skill.name.clone(), skill);
                    }
                }
                Err(message) => catalog.diagnostics.push(SkillDiagnostic {
                    source: path,
                    message,
                }),
            }
        }
        catalog
    }

    pub fn get(&self, name: &str) -> Option<&Skill> {
        self.skills.get(name)
    }

    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.skills.keys().map(String::as_str)
    }

    pub fn skills(&self) -> impl Iterator<Item = &Skill> {
        self.skills.values()
    }

    pub fn diagnostics(&self) -> &[SkillDiagnostic] {
        &self.diagnostics
    }

    pub fn discovery_prompt(&self) -> Option<String> {
        if self.is_empty() {
            return None;
        }
        let mut prompt = String::from(
            "## Available skills\n\nSkills are user-configured reusable workflows. Only their metadata is shown here. Call `activate_skill` before following a skill's instructions. Activate a skill when the user names it or when the request clearly matches its description.\n",
        );
        for skill in self.skills() {
            prompt.push_str("- `");
            prompt.push_str(&skill.name);
            prompt.push_str("`: ");
            prompt.push_str(&compact_description(&skill.description));
            prompt.push('\n');
        }
        Some(prompt)
    }

    pub fn activated_prompt<'a>(&self, names: impl IntoIterator<Item = &'a str>) -> Option<String> {
        let mut prompt = String::new();
        let mut included = std::collections::BTreeSet::new();
        for name in names {
            if !included.insert(name) {
                continue;
            }
            let Some(skill) = self.get(name) else {
                continue;
            };
            if prompt.is_empty() {
                prompt.push_str("## Activated skills\n");
            }
            prompt.push_str("\n### ");
            prompt.push_str(&skill.name);
            prompt.push_str("\n\nThe following instructions were explicitly configured by the user for this workflow:\n\n");
            prompt.push_str(&skill.instructions);
            prompt.push('\n');
        }
        (!prompt.is_empty()).then_some(prompt)
    }
}

#[derive(Deserialize)]
struct SkillFrontmatter {
    name: String,
    description: String,
}

fn load_skill(path: &Path) -> Result<Skill, String> {
    let metadata = fs::metadata(path).map_err(|error| format!("failed to stat skill: {error}"))?;
    if metadata.len() > MAX_SKILL_FILE_BYTES {
        return Err(format!(
            "skill is {} bytes; maximum supported size is {MAX_SKILL_FILE_BYTES} bytes",
            metadata.len()
        ));
    }
    let content =
        fs::read_to_string(path).map_err(|error| format!("failed to read skill: {error}"))?;
    let (yaml, instructions) = split_frontmatter(&content)?;
    let frontmatter: SkillFrontmatter =
        serde_yaml::from_str(yaml).map_err(|error| format!("invalid YAML frontmatter: {error}"))?;
    validate_name(&frontmatter.name)?;
    let description = frontmatter.description.trim();
    if description.is_empty() {
        return Err("skill description must not be empty".into());
    }
    if description.chars().count() > MAX_SKILL_DESCRIPTION_CHARS {
        return Err(format!(
            "skill description exceeds {MAX_SKILL_DESCRIPTION_CHARS} characters"
        ));
    }

    Ok(Skill {
        name: frontmatter.name,
        description: description.to_string(),
        instructions: instructions.trim().to_string(),
        source: path.to_path_buf(),
    })
}

fn split_frontmatter(content: &str) -> Result<(&str, &str), String> {
    let rest = content
        .strip_prefix("---\n")
        .or_else(|| content.strip_prefix("---\r\n"))
        .ok_or_else(|| "skill must begin with YAML frontmatter (`---`)".to_string())?;
    let mut offset = 0;
    for segment in rest.split_inclusive('\n') {
        let line = segment.trim_end_matches(['\r', '\n']);
        if line == "---" {
            return Ok((&rest[..offset], &rest[offset + segment.len()..]));
        }
        offset += segment.len();
    }
    Err("skill YAML frontmatter is missing its closing `---`".into())
}

fn validate_name(name: &str) -> Result<(), String> {
    let length = name.chars().count();
    if length == 0 || length > MAX_SKILL_NAME_CHARS {
        return Err(format!(
            "skill name must contain 1 to {MAX_SKILL_NAME_CHARS} characters"
        ));
    }
    if name.starts_with('-') || name.ends_with('-') {
        return Err("skill name must not start or end with a hyphen".into());
    }
    if !name
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(
            "skill name may contain only lowercase ASCII letters, digits, and hyphens".into(),
        );
    }
    Ok(())
}

fn compact_description(description: &str) -> String {
    description.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn default_skills_dir() -> PathBuf {
    skills_dir_from_locations(
        std::env::var_os("OVIM_CONFIG"),
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
    )
}

fn skills_dir_from_locations(
    ovim_config: Option<OsString>,
    xdg_config_home: Option<OsString>,
    home: Option<OsString>,
) -> PathBuf {
    if let Some(root) = nonempty_path(ovim_config) {
        return root.join("skills");
    }
    if let Some(root) = nonempty_path(xdg_config_home) {
        return root.join("ovim").join("skills");
    }
    if let Some(root) = nonempty_path(home) {
        return root.join(".config").join("ovim").join("skills");
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ovim")
        .join("skills")
}

fn nonempty_path(value: Option<OsString>) -> Option<PathBuf> {
    value.filter(|value| !value.is_empty()).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_open_format_flat_markdown_skills_in_name_order() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("review.md"),
            "---\nname: review-code\ndescription: |\n  Review changes carefully.\n  Use for pull requests.\nlicense: MIT\ncompatibility: Requires ovim.\nmetadata:\n  author: test\n---\n\n1. Read the diff.\n2. Report findings.\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("learn.md"),
            "---\r\nname: learn-codebase\r\ndescription: Teach one concept at a time.\r\n---\r\nStart with the highest-impact concept.\r\n",
        )
        .unwrap();
        fs::write(dir.path().join("ignored.txt"), "not a skill").unwrap();

        let catalog = SkillCatalog::load_from_dir(dir.path());

        assert_eq!(
            catalog.names().collect::<Vec<_>>(),
            vec!["learn-codebase", "review-code"]
        );
        assert_eq!(
            catalog.get("learn-codebase").unwrap().instructions,
            "Start with the highest-impact concept."
        );
        assert!(catalog.diagnostics().is_empty());
        assert!(catalog
            .discovery_prompt()
            .unwrap()
            .contains("Review changes carefully. Use for pull requests."));
    }

    #[test]
    fn invalid_and_duplicate_skills_do_not_hide_valid_skills() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("a.md"),
            "---\nname: useful-skill\ndescription: First declaration.\n---\nDo useful work.\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("b.md"),
            "---\nname: useful-skill\ndescription: Duplicate.\n---\nWrong body.\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("bad.md"),
            "---\nname: Bad_Name\ndescription: Invalid name.\n---\nBody.\n",
        )
        .unwrap();

        let catalog = SkillCatalog::load_from_dir(dir.path());

        assert_eq!(catalog.names().collect::<Vec<_>>(), vec!["useful-skill"]);
        assert_eq!(catalog.diagnostics().len(), 2);
        assert_eq!(
            catalog.get("useful-skill").unwrap().instructions,
            "Do useful work."
        );
    }

    #[test]
    fn skills_directory_honors_ovim_then_xdg_then_home() {
        let ovim = Some(OsString::from("/custom/ovim"));
        let xdg = Some(OsString::from("/xdg"));
        let home = Some(OsString::from("/home/me"));

        assert_eq!(
            skills_dir_from_locations(ovim, xdg.clone(), home.clone()),
            PathBuf::from("/custom/ovim/skills")
        );
        assert_eq!(
            skills_dir_from_locations(None, xdg, home.clone()),
            PathBuf::from("/xdg/ovim/skills")
        );
        assert_eq!(
            skills_dir_from_locations(None, None, home),
            PathBuf::from("/home/me/.config/ovim/skills")
        );
    }

    #[test]
    fn activated_prompt_includes_only_requested_known_skills_once() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("learn.md"),
            "---\nname: learn-codebase\ndescription: Teach code.\n---\nOne concept at a time.\n",
        )
        .unwrap();
        let catalog = SkillCatalog::load_from_dir(dir.path());

        let prompt = catalog
            .activated_prompt(["missing", "learn-codebase", "learn-codebase"])
            .unwrap();

        assert_eq!(prompt.matches("### learn-codebase").count(), 1);
        assert!(prompt.contains("One concept at a time."));
        assert!(!prompt.contains("missing"));
    }
}
