use super::spec::WorkflowSpec;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub fn default_workflow_dir() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("ovim").join("workflows")
}

pub fn load_workflows() -> Result<HashMap<String, WorkflowSpec>> {
    load_workflows_from_dir(&default_workflow_dir())
}

pub fn load_workflows_from_dir(dir: &Path) -> Result<HashMap<String, WorkflowSpec>> {
    if !dir.exists() {
        return Ok(HashMap::new());
    }
    if !dir.is_dir() {
        anyhow::bail!("workflow path '{}' is not a directory", dir.display());
    }

    let mut files = Vec::new();
    for entry in fs::read_dir(dir)
        .with_context(|| format!("failed to read workflow dir '{}'", dir.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if ext.eq_ignore_ascii_case("yaml") || ext.eq_ignore_ascii_case("yml") {
            files.push(path);
        }
    }
    files.sort();

    let mut out = HashMap::new();
    for path in files {
        let content = fs::read_to_string(&path)
            .with_context(|| format!("failed to read workflow file '{}'", path.display()))?;
        let spec: WorkflowSpec = serde_yaml::from_str(&content)
            .with_context(|| format!("failed to parse workflow file '{}'", path.display()))?;
        spec.validate()
            .with_context(|| format!("invalid workflow '{}'", path.display()))?;
        if out.contains_key(&spec.name) {
            anyhow::bail!(
                "duplicate workflow name '{}' (file '{}')",
                spec.name,
                path.display()
            );
        }
        out.insert(spec.name.clone(), spec);
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_workflows_from_empty_dir() {
        let dir = tempfile::tempdir().expect("tempdir");
        let workflows = load_workflows_from_dir(dir.path()).expect("load workflows");
        assert!(workflows.is_empty());
    }
}
