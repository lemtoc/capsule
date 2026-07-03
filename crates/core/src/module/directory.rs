//! Directory module — displays the current working directory.

use std::path::Path;

use super::{Module, ModuleOutput, ModuleSpeed, RenderContext};
use crate::sealed;

/// Displays the current working directory.
///
/// Shows a compact path while preserving the active git repository name.
#[derive(Debug, Default)]
#[allow(clippy::module_name_repetitions)]
pub struct DirectoryModule;

impl DirectoryModule {
    /// Creates a new `DirectoryModule`.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl sealed::Sealed for DirectoryModule {}

impl Module for DirectoryModule {
    fn name(&self) -> &'static str {
        "directory"
    }

    fn speed(&self) -> ModuleSpeed {
        ModuleSpeed::Fast
    }

    fn render(&self, ctx: &RenderContext<'_>) -> Option<ModuleOutput> {
        let content = format_directory(ctx.cwd, ctx.home_dir);
        Some(ModuleOutput { content })
    }
}

fn format_directory(cwd: &Path, home: &Path) -> String {
    let repo_root = find_git_root(cwd);
    compact_components(cwd, home, repo_root)
}

fn find_git_root(start: &Path) -> Option<&Path> {
    let mut dir = start;
    loop {
        if dir.join(".git").exists() {
            return Some(dir);
        }
        dir = dir.parent()?;
    }
}

fn compact_components(cwd: &Path, home: &Path, repo_root: Option<&Path>) -> String {
    if cwd == home {
        return "~".to_owned();
    }

    let (prefix, relative) = cwd.strip_prefix(home).map_or_else(
        |_| (root_prefix(cwd), cwd),
        |suffix| ("~".to_owned(), suffix),
    );
    let components = path_components(relative);
    if components.is_empty() {
        return if prefix.is_empty() {
            "/".to_owned()
        } else {
            prefix
        };
    }

    let full_from = repo_root
        .and_then(|root| root.strip_prefix(home).ok())
        .map(path_components)
        .and_then(|root_components| root_components.len().checked_sub(1))
        .unwrap_or_else(|| components.len().saturating_sub(1));

    let mut parts = Vec::with_capacity(components.len() + 1);
    parts.push(prefix);
    parts.extend(components.iter().enumerate().map(|(index, component)| {
        if index < full_from {
            shorten_component(component)
        } else {
            component.clone()
        }
    }));
    parts.join("/")
}

fn root_prefix(path: &Path) -> String {
    if path.is_absolute() {
        String::new()
    } else {
        ".".to_owned()
    }
}

fn path_components(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|component| match component {
            std::path::Component::Normal(value) => Some(value.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect()
}

fn shorten_component(component: &str) -> String {
    component.chars().take(1).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ctx<'a>(cwd: &'a Path, home: &'a Path) -> RenderContext<'a> {
        RenderContext {
            cwd,
            home_dir: home,
            last_exit_code: 0,
            duration_ms: None,
            keymap: "main",
            cols: 80,
        }
    }

    // -- Non-git paths (home abbreviation) --

    #[test]
    fn test_module_cwd_is_home() {
        let home = Path::new("/Users/testuser");
        let ctx = make_ctx(home, home);
        let output = DirectoryModule::new().render(&ctx);
        assert_eq!(output.map(|o| o.content), Some("~".to_owned()));
    }

    #[test]
    fn test_module_cwd_outside_home() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let home = Path::new("/Users/testuser");
        let ctx = make_ctx(dir.path(), home);
        let output = DirectoryModule::new().render(&ctx);
        // No .git above, so home-abbreviated
        let content = output.map(|o| o.content);
        assert!(
            content.as_ref().is_some_and(|c| !c.is_empty()),
            "should produce output: {content:?}"
        );
        Ok(())
    }

    #[test]
    fn test_module_cwd_is_root() {
        let home = Path::new("/Users/testuser");
        let cwd = Path::new("/");
        let ctx = make_ctx(cwd, home);
        let output = DirectoryModule::new().render(&ctx);
        assert_eq!(output.map(|o| o.content), Some("/".to_owned()));
    }

    // -- Git repo paths --

    #[test]
    fn test_module_at_git_repo_root() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        std::fs::create_dir(dir.path().join(".git"))?;
        let home = dir.path().parent().ok_or("temp dir has no parent")?;
        let ctx = make_ctx(dir.path(), home);
        let output = DirectoryModule::new().render(&ctx);
        let content = output.map(|o| o.content);
        let folder_name = dir
            .path()
            .file_name()
            .map(|n| n.to_string_lossy().into_owned());
        assert_eq!(content, folder_name.map(|name| format!("~/{name}")));
        Ok(())
    }

    #[test]
    fn test_module_inside_git_repo() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        std::fs::create_dir(dir.path().join(".git"))?;
        let sub = dir.path().join("src").join("module");
        std::fs::create_dir_all(&sub)?;
        let home = dir.path().parent().ok_or("temp dir has no parent")?;
        let ctx = make_ctx(&sub, home);
        let output = DirectoryModule::new().render(&ctx);
        let repo_name = dir
            .path()
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or("temp dir has no file name")?;
        assert_eq!(
            output.map(|o| o.content),
            Some(format!("~/{repo_name}/src/module")),
            "should preserve repo root and repo-relative suffix"
        );
        Ok(())
    }

    #[test]
    fn test_module_inside_nested_git_repo_shortens_ancestors()
    -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let home = dir.path();
        let repo_root = home
            .join("dev")
            .join("work")
            .join("tip-extra")
            .join("tipextra-frontend");
        std::fs::create_dir_all(repo_root.join(".git"))?;
        let cwd = repo_root.join("frontend");
        std::fs::create_dir_all(&cwd)?;
        let ctx = make_ctx(&cwd, home);
        let output = DirectoryModule::new().render(&ctx);
        assert_eq!(
            output.map(|o| o.content),
            Some("~/d/w/t/tipextra-frontend/frontend".to_owned()),
            "should shorten ancestors before the repo root"
        );
        Ok(())
    }

    #[test]
    fn test_module_cwd_under_home_no_git() {
        // Use a path that does not exist on disk (no .git can be found)
        let home = Path::new("/Users/testuser");
        let cwd = Path::new("/Users/testuser/nonexistent/projects/capsule");
        let ctx = make_ctx(cwd, home);
        let output = DirectoryModule::new().render(&ctx);
        assert_eq!(
            output.map(|o| o.content),
            Some("~/n/p/capsule".to_owned()),
            "should shorten ancestors and preserve the current directory when no .git"
        );
    }
}
