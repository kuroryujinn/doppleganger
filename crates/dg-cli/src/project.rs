use crate::schema::AccountMapping;
use dg_config::ProjectManifest;
use serde::Deserialize;
use solana_address::Address;
use std::{
    error::Error,
    path::{Path, PathBuf},
    str::FromStr,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Version {
    pub idl: PathBuf,
    pub binary: Option<PathBuf>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub name: String,
    pub program_id: String,
    pub v1: Version,
    pub v2: Version,
    pub accounts: Vec<AccountMapping>,
    pub scenario: Option<String>,
}

impl Project {
    pub fn artifact(&self, manifest: &Path, relative: &Path) -> Result<PathBuf, Box<dyn Error>> {
        let base = manifest.parent().ok_or("project manifest has no parent")?;
        Ok(base.join(relative).canonicalize()?)
    }

    fn validate(&self) -> Result<(), Box<dyn Error>> {
        if self.name.is_empty()
            || !self
                .name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err("project name must use letters, digits, '-' or '_'".into());
        }
        Address::from_str(&self.program_id)?;
        if self.accounts.is_empty() {
            return Err("project must map at least one account".into());
        }
        if self
            .accounts
            .iter()
            .any(|account| account.v1.is_empty() || account.v2.is_empty())
        {
            return Err("account mappings need V1 and V2 type names".into());
        }
        if self
            .scenario
            .as_deref()
            .is_some_and(|scenario| scenario != "vault")
        {
            return Err("supported scenario: vault".into());
        }
        Ok(())
    }
}

pub fn read_manifest(path: &Path) -> Result<Project, Box<dyn Error>> {
    let project: Project = toml::from_str(&String::from_utf8(dg_analysis::read_bounded(
        path,
        4 * 1024 * 1024,
    )?)?)?;
    project.validate()?;
    Ok(project)
}

pub fn add(path: &Path) -> Result<String, Box<dyn Error>> {
    let path = if path.is_dir() {
        path.join("doppelganger.toml")
    } else {
        path.to_owned()
    };
    let path = path.canonicalize()?;
    let name = manifest_name(&path)?;
    crate::registry::add(name.clone(), path)?;
    Ok(name)
}

pub fn list() -> Result<Vec<(String, PathBuf)>, Box<dyn Error>> {
    Ok(crate::registry::load()?.projects.into_iter().collect())
}

pub fn registered(name: &str) -> Result<(PathBuf, Project), Box<dyn Error>> {
    let path = crate::registry::load()?
        .projects
        .remove(name)
        .ok_or_else(|| format!("project '{name}' is not registered"))?;
    let project = read_manifest(&path)?;
    if project.name != name {
        return Err(format!("project name changed in {}", path.display()).into());
    }
    Ok((path, project))
}

fn manifest_name(path: &Path) -> Result<String, Box<dyn Error>> {
    let text = String::from_utf8(dg_analysis::read_bounded(path, 4 * 1024 * 1024)?)?;
    let value: toml::Value = toml::from_str(&text)?;
    if value.get("project").is_some() {
        Ok(ProjectManifest::load(path)?.manifest.project.name)
    } else {
        Ok(read_manifest(path)?.name)
    }
}
pub fn registered_manifest(name: &str) -> Result<PathBuf, Box<dyn Error>> {
    let path = crate::registry::load()?
        .projects
        .get(name)
        .cloned()
        .ok_or_else(|| format!("project '{name}' is not registered"))?;
    if manifest_name(&path)? != name {
        return Err(format!("project name changed in {}", path.display()).into());
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unsafe_names_and_empty_account_mapping() {
        let mut project: Project = toml::from_str(
            r#"
            name = "../outside"
            program_id = "Fg6PaFpoGXkYsidMpWTK6W2BeZ7FEfcYkg476zPFsLnS"
            [v1]
            idl = "v1.json"
            [v2]
            idl = "v2.json"
            [[accounts]]
            v1 = "VaultV1"
            v2 = "VaultV2"
        "#,
        )
        .unwrap();
        assert!(project.validate().is_err());
        project.name = "vault".to_string();
        project.accounts.clear();
        assert!(project.validate().is_err());
    }
}
