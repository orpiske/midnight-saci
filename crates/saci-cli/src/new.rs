//! `saci new test-plan`: offline scaffolding from the embedded template.

use std::fs::OpenOptions;
use std::io::{ErrorKind, Write};
use std::path::Path;

use anyhow::{Context, bail};

/// The starter template, embedded so the command works after installation.
const TEMPLATE: &str = include_str!("../../../docs/templates/test-plan.md");
const TEMPLATE_TITLE: &str = "# Test Plan: Service health";

/// Creates a new plan at `path`, refusing to overwrite an existing file.
///
/// # Errors
///
/// Fails when the file exists, its directory does not exist, or it cannot
/// be written.
pub fn test_plan(path: &Path) -> anyhow::Result<()> {
    let contents = render(&title_from_path(path));
    let mut file = match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            bail!(
                "{} already exists; choose another name or remove it first",
                path.display()
            )
        }
        Err(error) if error.kind() == ErrorKind::NotFound => {
            let parent = path
                .parent()
                .map_or_else(|| ".".into(), |p| p.display().to_string());
            bail!(
                "cannot create {}: directory {parent} does not exist",
                path.display()
            )
        }
        Err(error) => {
            return Err(error).with_context(|| format!("cannot create {}", path.display()));
        }
    };
    file.write_all(contents.as_bytes())
        .with_context(|| format!("cannot write {}", path.display()))?;
    println!("Created {}", path.display());
    Ok(())
}

/// The template with its title replaced.
#[must_use]
pub fn render(title: &str) -> String {
    TEMPLATE.replacen(TEMPLATE_TITLE, &format!("# Test Plan: {title}"), 1)
}

/// Derives an editable title from a file name: `saci-on-the-cloud.md`
/// becomes `Saci on the cloud`.
#[must_use]
pub fn title_from_path(path: &Path) -> String {
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let words: Vec<&str> = stem
        .split(['-', '_', ' ', '.'])
        .filter(|word| !word.is_empty())
        .collect();
    let mut title = words.join(" ");
    if let Some(first) = title.get(..1) {
        title = first.to_uppercase() + &title[1..];
    }
    if title.is_empty() {
        "New test plan".to_owned()
    } else {
        title
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use saci_core::parser::{DocumentRole, parse_document};

    #[test]
    fn titles_come_from_file_names() {
        assert_eq!(
            title_from_path(Path::new("saci-on-the-cloud.md")),
            "Saci on the cloud"
        );
        assert_eq!(title_from_path(Path::new("dir/api_smoke.md")), "Api smoke");
        assert_eq!(title_from_path(Path::new("-.md")), "New test plan");
    }

    #[test]
    fn rendered_plans_parse_with_the_derived_title() {
        let rendered = render("Saci on the cloud");
        parse_document("plan.md", &rendered, DocumentRole::Entry).unwrap();
        assert!(rendered.starts_with("# Test Plan: Saci on the cloud\n"));
    }
}
