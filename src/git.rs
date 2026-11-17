//! Обёртка над `git` CLI: лог, статус, теги, commit и push.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::commit::RawCommit;
use crate::error::{ReleaseError, Result};

/// Git-репозиторий проекта.
pub struct Git {
    root: PathBuf,
}

impl Git {
    /// Репозиторий в каталоге.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Корень репозитория.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn command(&self, args: &[&str]) -> Result<std::process::Output> {
        Command::new("git")
            .current_dir(&self.root)
            .args(args)
            .output()
            .map_err(|error| ReleaseError::Git(format!("git {}: {error}", args.join(" "))))
    }

    fn run(&self, args: &[&str]) -> Result<String> {
        let output = self.command(args)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ReleaseError::Git(format!(
                "git {}: {}",
                args.join(" "),
                stderr.trim()
            )));
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_string())
    }

    fn run_optional(&self, args: &[&str]) -> Result<Option<String>> {
        let output = self.command(args)?;
        if output.status.success() {
            Ok(Some(
                String::from_utf8_lossy(&output.stdout)
                    .trim_end()
                    .to_string(),
            ))
        } else {
            Ok(None)
        }
    }

    /// Является ли каталог git-репозиторием.
    #[must_use]
    pub fn is_repo(&self) -> bool {
        self.run_optional(&["rev-parse", "--is-inside-work-tree"])
            .map(|value| value.as_deref() == Some("true"))
            .unwrap_or(false)
    }

    /// Чисто ли рабочее дерево.
    ///
    /// # Errors
    ///
    /// [`ReleaseError::Git`] — git недоступен.
    pub fn is_clean(&self) -> Result<bool> {
        Ok(self.run(&["status", "--porcelain"])?.is_empty())
    }

    /// Текущая ветка.
    ///
    /// # Errors
    ///
    /// [`ReleaseError::Git`] — git недоступен или HEAD отсутствует.
    pub fn current_branch(&self) -> Result<String> {
        self.run(&["rev-parse", "--abbrev-ref", "HEAD"])
    }

    /// Существует ли тег.
    ///
    /// # Errors
    ///
    /// [`ReleaseError::Git`] — git недоступен.
    pub fn tag_exists(&self, tag: &str) -> Result<bool> {
        let reference = format!("refs/tags/{tag}");
        Ok(self
            .run_optional(&["rev-parse", "--verify", &reference])?
            .is_some())
    }

    /// Последний тег с префиксом (по semver-порядку).
    ///
    /// # Errors
    ///
    /// [`ReleaseError::Git`] — git недоступен.
    pub fn last_tag(&self, prefix: &str) -> Result<Option<String>> {
        let pattern = format!("{prefix}*");
        let output = self.run(&["tag", "--list", &pattern, "--sort=-v:refname"])?;
        Ok(output
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map(str::to_string))
    }

    /// Коммиты диапазона `from..to`.
    ///
    /// # Errors
    ///
    /// [`ReleaseError::Git`] — git недоступен или диапазон невалиден.
    pub fn log(&self, from: Option<&str>, to: &str) -> Result<Vec<RawCommit>> {
        let range = match from {
            Some(from) => format!("{from}..{to}"),
            None => to.to_string(),
        };
        let format = "--format=%H%x1f%h%x1f%cs%x1f%B%x1e";
        let output = self.run(&["log", format, &range])?;
        let mut commits = Vec::new();
        for record in output.split('\u{1e}') {
            let record = record.trim_start_matches(['\n', '\r']);
            if record.is_empty() {
                continue;
            }
            let mut fields = record.splitn(4, '\u{1f}');
            let (Some(hash), Some(short_hash), Some(date), Some(message)) =
                (fields.next(), fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            commits.push(RawCommit {
                hash: hash.to_string(),
                short_hash: short_hash.to_string(),
                date: date.to_string(),
                message: message.to_string(),
            });
        }
        Ok(commits)
    }

    /// Дата HEAD (`YYYY-MM-DD`).
    ///
    /// # Errors
    ///
    /// [`ReleaseError::Git`] — git недоступен.
    pub fn head_date(&self) -> Result<String> {
        self.run(&["log", "-1", "--format=%cs"])
    }

    /// Короткий хэш HEAD.
    ///
    /// # Errors
    ///
    /// [`ReleaseError::Git`] — git недоступен.
    pub fn head_short(&self) -> Result<String> {
        self.run(&["rev-parse", "--short", "HEAD"])
    }

    /// URL origin.
    ///
    /// # Errors
    ///
    /// [`ReleaseError::Git`] — git недоступен.
    pub fn remote_url(&self) -> Result<Option<String>> {
        self.run_optional(&["remote", "get-url", "origin"])
    }

    /// `owner/repo` для GitHub origin.
    ///
    /// # Errors
    ///
    /// [`ReleaseError::Git`] — git недоступен.
    pub fn repo_slug(&self) -> Result<Option<String>> {
        Ok(self.remote_url()?.as_deref().and_then(repo_slug))
    }

    /// Индексирует все изменения.
    ///
    /// # Errors
    ///
    /// [`ReleaseError::Git`] — git недоступен.
    pub fn add_all(&self) -> Result<()> {
        self.run(&["add", "-A"]).map(|_| ())
    }

    /// Создаёт коммит.
    ///
    /// # Errors
    ///
    /// [`ReleaseError::Git`] — коммит не создан.
    pub fn commit(&self, message: &str, no_verify: bool) -> Result<()> {
        let mut args = vec!["commit", "-m", message];
        if no_verify {
            args.push("--no-verify");
        }
        self.run(&args).map(|_| ())
    }

    /// Создаёт annotated tag.
    ///
    /// # Errors
    ///
    /// [`ReleaseError::Git`] — тег не создан (например, уже существует).
    pub fn create_tag(&self, tag: &str, message: &str) -> Result<()> {
        self.run(&["tag", "-a", tag, "-m", message]).map(|_| ())
    }

    /// Публикует ветку и тег.
    ///
    /// # Errors
    ///
    /// [`ReleaseError::Git`] — push не удался.
    pub fn push(&self, branch: &str, tag: Option<&str>) -> Result<()> {
        self.run(&["push", "origin", branch])?;
        if let Some(tag) = tag {
            self.run(&["push", "origin", tag])?;
        }
        Ok(())
    }
}

/// Извлекает `owner/repo` из GitHub remote URL.
#[must_use]
pub fn repo_slug(remote: &str) -> Option<String> {
    let base = remote.trim_end_matches(".git").trim_end_matches('/');
    let path = base
        .strip_prefix("https://github.com/")
        .or_else(|| base.strip_prefix("http://github.com/"))
        .or_else(|| base.strip_prefix("git@github.com:"))?;
    let mut parts = path.split('/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    if owner.is_empty() || repo.is_empty() || parts.next().is_some() {
        return None;
    }
    Some(format!("{owner}/{repo}"))
}

/// Преобразует remote URL в ссылку на коммит.
#[must_use]
pub fn commit_url(remote: &str, hash: &str) -> Option<String> {
    let base = remote.trim_end_matches(".git");
    let base = match base.strip_prefix("git@github.com:") {
        Some(rest) => format!("https://github.com/{rest}"),
        None => base.to_string(),
    };
    if base.starts_with("http://") || base.starts_with("https://") {
        Some(format!("{}/commit/{hash}", base.trim_end_matches('/')))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_url_supports_https_and_ssh() {
        assert_eq!(
            commit_url("https://github.com/o/r.git", "abc").as_deref(),
            Some("https://github.com/o/r/commit/abc")
        );
        assert_eq!(
            commit_url("git@github.com:o/r.git", "abc").as_deref(),
            Some("https://github.com/o/r/commit/abc")
        );
        assert_eq!(commit_url("file:///tmp/repo", "abc"), None);
    }

    #[test]
    fn repo_slug_supports_https_and_ssh() {
        assert_eq!(
            repo_slug("https://github.com/o/r.git").as_deref(),
            Some("o/r")
        );
        assert_eq!(repo_slug("git@github.com:o/r.git").as_deref(), Some("o/r"));
        assert_eq!(repo_slug("https://gitlab.com/o/r.git"), None);
        assert_eq!(repo_slug("https://github.com/o/r/extra"), None);
    }
}
