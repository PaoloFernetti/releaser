//! Оркестрация релиза: preview (read-only) и apply (запись).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use semver::Version;
use serde::Serialize;

use crate::bump::{self, BumpPlan};
use crate::changelog;
use crate::commit::{self, ConventionalCommit};
use crate::config::{Bump, Locale, PackageMode, ReleaseConfig};
use crate::error::{ReleaseError, Result};
use crate::git::{commit_url, Git};
use crate::manifest;

/// Параметры запуска.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Корень проекта.
    pub root: PathBuf,
    /// Явный путь к конфигу.
    pub config_path: Option<PathBuf>,
    /// Ничего не писать.
    pub dry_run: bool,
    /// Машинный вывод.
    pub json: bool,
    /// Начало диапазона коммитов.
    pub from: Option<String>,
    /// Конец диапазона коммитов.
    pub to: Option<String>,
    /// Диапазон `from..to` для `check`.
    pub commits: Option<String>,
    /// Pre-release-канал (`alpha`, `beta`, `rc`).
    pub prerelease: Option<String>,
    /// Переопределение префикса тегов.
    pub tag_prefix: Option<String>,
    /// Внешний шаблон changelog.
    pub template: Option<PathBuf>,
    /// Не запускать git-хуки.
    pub no_verify: bool,
    /// Не проверять git-состояние (repo/branch/clean).
    pub no_git_checks: bool,
    /// `cargo package` после тега.
    pub package: bool,
    /// `cargo publish` после тега.
    pub publish: bool,
    /// `git push` после тега.
    pub push: bool,
    /// Создать GitHub Release.
    pub github_release: bool,
    /// Запустить `cargo-semver-checks`.
    pub semver_checks: bool,
    /// Первый релиз: взять `initial_version`.
    pub first_release: bool,
    /// CI-режим: не запускать git-хуки; push/publish/gh остаются opt-in.
    pub ci: bool,
}

/// Предпросмотр релиза.
#[derive(Clone, Debug, Serialize)]
pub struct Preview {
    /// Текущая версия.
    pub current: String,
    /// Следующая версия.
    pub next: Option<String>,
    /// Вклад коммитов.
    pub bump: Bump,
    /// Тег.
    pub tag: Option<String>,
    /// Число релевантных коммитов.
    pub commits: usize,
    /// Готовая запись changelog.
    pub changelog: Option<String>,
    /// Тег уже существует.
    pub already_released: bool,
    /// Текущая ветка.
    pub branch: Option<String>,
    /// Пакеты (independent).
    pub packages: Vec<PackagePreview>,
}

/// Предпросмотр версии пакета.
#[derive(Clone, Debug, Serialize)]
pub struct PackagePreview {
    /// Имя пакета.
    pub name: Option<String>,
    /// Путь к манифесту.
    pub manifest: String,
    /// Текущая версия.
    pub current: String,
    /// Следующая версия.
    pub next: Option<String>,
}

/// Результат применения релиза.
#[derive(Clone, Debug, Serialize)]
pub struct Outcome {
    /// Предыдущая версия.
    pub previous: String,
    /// Новая версия.
    pub next: Option<String>,
    /// Вклад коммитов.
    pub bump: Bump,
    /// Тег.
    pub tag: Option<String>,
    /// Число коммитов.
    pub commits: usize,
    /// Changelog обновлён.
    pub changelog_written: bool,
    /// Создан коммит.
    pub committed: bool,
    /// Создан тег.
    pub tagged: bool,
    /// Выполнена публикация.
    pub published: bool,
    /// Релиз пропущен (нет изменений/уже существует).
    pub skipped: bool,
    /// Пробный прогон.
    pub dry_run: bool,
    /// Пакеты.
    pub packages: Vec<PackagePreview>,
}

/// Отчёт `check`.
#[derive(Clone, Debug, Serialize)]
pub struct CheckReport {
    /// Всего коммитов в диапазоне.
    pub total: usize,
    /// Conventional Commits.
    pub conventional: usize,
    /// Нарушения (первая строка сообщения).
    pub invalid: Vec<String>,
    /// Конфиг валиден.
    pub config_ok: bool,
}

/// Отчёт `doctor`.
#[derive(Clone, Debug, Serialize)]
pub struct DoctorReport {
    /// git-репозиторий.
    pub git_repo: bool,
    /// Чистое дерево.
    pub clean: bool,
    /// Ветка.
    pub branch: Option<String>,
    /// Конфиг валиден.
    pub config_ok: bool,
    /// Текущая версия.
    pub version: Option<String>,
    /// Тегов с префиксом.
    pub tags: usize,
    /// `cargo` доступен.
    pub cargo: bool,
    /// `gh` доступен.
    pub gh: bool,
    /// `cargo semver-checks` доступен.
    pub semver_checks: bool,
}

struct Context {
    config: ReleaseConfig,
    git: Git,
    current: Version,
    commits: Vec<ConventionalCommit>,
    plan: BumpPlan,
    tag: Option<String>,
    already_released: bool,
    repo_url: Option<String>,
    branch: Option<String>,
    packages: Vec<PackagePreview>,
}

fn tag_prefix(options: &Options, config: &ReleaseConfig) -> String {
    options
        .tag_prefix
        .clone()
        .unwrap_or_else(|| config.tag_prefix().to_string())
}

fn load_context(options: &Options) -> Result<Context> {
    let mut config = ReleaseConfig::load(&options.root, options.config_path.as_deref())?;
    if let Some(template) = &options.template {
        config.template = Some(template.clone());
    }
    let git = Git::new(&options.root);

    let mut branch = None;
    if !options.no_git_checks {
        if !git.is_repo() {
            return Err(ReleaseError::Git(format!(
                "{} is not a git repository",
                options.root.display()
            )));
        }
        if !git.is_clean()? {
            return Err(ReleaseError::Git(
                "working tree is dirty: commit or stash changes first".into(),
            ));
        }
        let current_branch = git.current_branch()?;
        if !config
            .branches
            .iter()
            .any(|allowed| allowed == &current_branch)
        {
            return Err(ReleaseError::Config(format!(
                "branch '{current_branch}' is not allowed (configured: {:?})",
                config.branches
            )));
        }
        branch = Some(current_branch);
    }

    let current = manifest::read_version(&options.root.join("Cargo.toml")).or_else(|error| {
        if let Some(package) = config.packages.first() {
            manifest::read_version(&options.root.join(&package.manifest))
        } else {
            Err(error)
        }
    })?;

    let prefix = tag_prefix(options, &config);
    let from = match &options.from {
        Some(from) => Some(from.clone()),
        None => git.last_tag(&prefix)?,
    };
    let to = options.to.clone().unwrap_or_else(|| "HEAD".to_string());
    let raw = git.log(from.as_deref(), &to)?;
    let mut commits = Vec::new();
    for raw_commit in raw {
        let Some(parsed) = commit::parse_commit(&raw_commit) else {
            continue;
        };
        if config.is_skipped(&parsed.description) || parsed.is_release_commit() {
            continue;
        }
        commits.push(parsed);
    }

    let plan = bump::plan(
        &commits,
        &config,
        &current,
        options.prerelease.as_deref(),
        options.first_release,
    )?;
    let tag = plan.next.as_ref().map(|next| config.tag_for(next));
    let already_released = match &tag {
        Some(tag) => git.tag_exists(tag)?,
        None => false,
    };
    let repo_url = git
        .remote_url()?
        .and_then(|remote| commit_url(&remote, "0000000"))
        .and_then(|url| url.strip_suffix("/commit/0000000").map(str::to_string));

    let packages = package_previews(&config, options, &plan)?;

    Ok(Context {
        config,
        git,
        current,
        commits,
        plan,
        tag,
        already_released,
        repo_url,
        branch,
        packages,
    })
}

fn package_previews(
    config: &ReleaseConfig,
    options: &Options,
    plan: &BumpPlan,
) -> Result<Vec<PackagePreview>> {
    if config.mode != PackageMode::Independent {
        return Ok(Vec::new());
    }
    let mut packages = Vec::new();
    for package in &config.packages {
        let path = options.root.join(&package.manifest);
        let current = manifest::read_version(&path)?;
        let next = if plan.bump == Bump::None && options.prerelease.is_none() {
            None
        } else {
            Some(bump::next_version(
                &current,
                if plan.bump == Bump::None {
                    Bump::Patch
                } else {
                    plan.bump
                },
                options.prerelease.as_deref(),
            )?)
        };
        packages.push(PackagePreview {
            name: package
                .name
                .clone()
                .or_else(|| manifest::read_package_name(&path).ok().flatten()),
            manifest: path.display().to_string(),
            current: current.to_string(),
            next: next.map(|version| version.to_string()),
        });
    }
    Ok(packages)
}

fn changelog_entry(context: &Context, version: &Version, date: &str) -> Result<String> {
    let Some(template) = &context.config.template else {
        return Ok(changelog::render_entry(
            version,
            date,
            &context.commits,
            &context.config,
            context.repo_url.as_deref(),
        ));
    };
    let path = context.git.root().join(template);
    let template = std::fs::read_to_string(&path)
        .map_err(|error| ReleaseError::Changelog(format!("{}: {error}", path.display())))?;
    let sections = changelog::render_sections(
        &context.commits,
        &context.config,
        context.repo_url.as_deref(),
    );
    let has_sections = template.contains("{sections}");
    let mut rendered = template
        .replace("{version}", &version.to_string())
        .replace("{date}", date)
        .replace("{repo_url}", context.repo_url.as_deref().unwrap_or(""))
        .replace("{sections}", sections.trim_start());
    if !has_sections {
        rendered.push_str(&sections);
    }
    Ok(rendered)
}

/// Строит предпросмотр без изменения файлов.
///
/// # Errors
///
/// [`ReleaseError`] — конфиг, git или манифест некорректны.
pub fn preview(options: &Options) -> Result<Preview> {
    let context = load_context(options)?;
    let date = context
        .git
        .head_date()
        .unwrap_or_else(|_| "1970-01-01".to_string());
    let changelog = context
        .plan
        .next
        .as_ref()
        .map(|next| changelog_entry(&context, next, &date))
        .transpose()?;
    Ok(Preview {
        current: context.current.to_string(),
        next: context.plan.next.as_ref().map(Version::to_string),
        bump: context.plan.bump,
        tag: context.tag.clone(),
        commits: context.commits.len(),
        changelog,
        already_released: context.already_released,
        branch: context.branch,
        packages: context.packages,
    })
}

fn run_command(command: &mut Command, description: &str) -> Result<()> {
    let output = command
        .output()
        .map_err(|error| ReleaseError::Command(format!("{description}: {error}")))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(ReleaseError::Command(format!(
            "{description}: {}",
            stderr.trim()
        )));
    }
    Ok(())
}

fn run_semver_checks(root: &Path) -> Result<()> {
    let mut probe = Command::new("cargo");
    probe.args(["semver-checks", "--version"]);
    if probe
        .output()
        .map(|output| !output.status.success())
        .unwrap_or(true)
    {
        return Err(ReleaseError::Command(
            "cargo-semver-checks is not installed (cargo install cargo-semver-checks)".into(),
        ));
    }
    run_command(
        Command::new("cargo")
            .args(["semver-checks", "check-release", "--manifest-path"])
            .arg(root.join("Cargo.toml")),
        "cargo semver-checks",
    )
}

fn refresh_lockfile(root: &Path) -> Result<()> {
    if !root.join("Cargo.lock").exists() {
        return Ok(());
    }
    run_command(
        Command::new("cargo")
            .arg("update")
            .arg("--workspace")
            .current_dir(root),
        "cargo update --workspace",
    )
}

fn package(manifests: &[PathBuf]) -> Result<()> {
    let order = manifest::publish_order(manifests)?;
    for manifest in &order {
        run_command(
            Command::new("cargo")
                .arg("package")
                .arg("--manifest-path")
                .arg(manifest),
            &format!("cargo package {}", manifest.display()),
        )?;
    }
    Ok(())
}

fn publish(manifests: &[PathBuf], dry_run: bool) -> Result<()> {
    let order = manifest::publish_order(manifests)?;
    for manifest in &order {
        let mut command = Command::new("cargo");
        command.arg("publish").arg("--manifest-path").arg(manifest);
        if dry_run {
            command.arg("--dry-run");
        }
        run_command(
            &mut command,
            &format!("cargo publish {}", manifest.display()),
        )?;
    }
    Ok(())
}

/// Создаёт GitHub Release для тега; повторный вызов для существующего релиза — no-op.
fn github_release(tag: &str, notes: &str, root: &Path) -> Result<()> {
    if command_available("gh", &["--version"]) {
        if command_available("gh", &["release", "view", tag]) {
            return Ok(());
        }
        return run_command(
            Command::new("gh").args(["release", "create", tag, "--title", tag, "--notes", notes]),
            "gh release create",
        );
    }
    let token = std::env::var("GITHUB_TOKEN")
        .or_else(|_| std::env::var("GH_TOKEN"))
        .map_err(|_| {
            ReleaseError::Command("gh is not installed and GITHUB_TOKEN/GH_TOKEN is not set".into())
        })?;
    let slug = Git::new(root).repo_slug()?.ok_or_else(|| {
        ReleaseError::Command("cannot derive a GitHub repository from origin".into())
    })?;
    if github_release_exists(&slug, tag, &token) {
        return Ok(());
    }
    let body = serde_json::json!({
        "tag_name": tag,
        "name": tag,
        "body": notes,
    })
    .to_string();
    let payload = std::env::temp_dir().join(format!("releaser-rs-{}.json", tag.replace('/', "-")));
    std::fs::write(&payload, body)?;
    let result = run_command(
        Command::new("curl")
            .args([
                "-sS",
                "-f",
                "-X",
                "POST",
                "-H",
                "Accept: application/vnd.github+json",
                "-H",
                "Content-Type: application/json",
            ])
            .arg("-H")
            .arg(format!("Authorization: Bearer {token}"))
            .arg("--data-binary")
            .arg(format!("@{}", payload.display()))
            .arg(format!("https://api.github.com/repos/{slug}/releases")),
        "GitHub API release",
    );
    let _ = std::fs::remove_file(&payload);
    result
}

/// Проверяет существование релиза по тегу через GitHub API (curl).
fn github_release_exists(slug: &str, tag: &str, token: &str) -> bool {
    let probe =
        std::env::temp_dir().join(format!("releaser-rs-probe-{}.json", tag.replace('/', "-")));
    let found = Command::new("curl")
        .args(["-sS", "-f", "-o"])
        .arg(&probe)
        .args(["-H", "Accept: application/vnd.github+json"])
        .arg("-H")
        .arg(format!("Authorization: Bearer {token}"))
        .arg(format!(
            "https://api.github.com/repos/{slug}/releases/tags/{tag}"
        ))
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false);
    let _ = std::fs::remove_file(&probe);
    found
}

/// Применяет релиз: версия, changelog, commit, tag, opt-in publish/push/release.
///
/// # Errors
///
/// [`ReleaseError`] — конфиг, git, манифест или внешняя команда.
pub fn apply(options: &Options) -> Result<Outcome> {
    let context = load_context(options)?;
    let Some(next) = context.plan.next.clone() else {
        return Ok(Outcome {
            previous: context.current.to_string(),
            next: None,
            bump: context.plan.bump,
            tag: None,
            commits: context.commits.len(),
            changelog_written: false,
            committed: false,
            tagged: false,
            published: false,
            skipped: true,
            dry_run: options.dry_run,
            packages: context.packages,
        });
    };
    if context.already_released {
        let tag = context
            .tag
            .clone()
            .unwrap_or_else(|| context.config.tag_for(&next));
        // Версия уже выпущена, но GitHub Release мог не создаться (например, job
        // отменили после пуша тега). Повторный прогон дозаполняет релиз.
        if options.github_release && !options.dry_run {
            let date = context
                .git
                .head_date()
                .unwrap_or_else(|_| "1970-01-01".to_string());
            let entry = changelog_entry(&context, &next, &date)?;
            github_release(&tag, &entry, &options.root)?;
        }
        return Ok(Outcome {
            previous: context.current.to_string(),
            next: Some(next.to_string()),
            bump: context.plan.bump,
            tag: Some(tag),
            commits: context.commits.len(),
            changelog_written: false,
            committed: false,
            tagged: false,
            published: false,
            skipped: true,
            dry_run: options.dry_run,
            packages: context.packages,
        });
    }
    let manifests = context.config.manifests(&options.root);
    if options.dry_run {
        if options.publish {
            publish(&manifests, true)?;
        }
        return Ok(Outcome {
            previous: context.current.to_string(),
            next: Some(next.to_string()),
            bump: context.plan.bump,
            tag: context.tag.clone(),
            commits: context.commits.len(),
            changelog_written: false,
            committed: false,
            tagged: false,
            published: options.publish,
            skipped: false,
            dry_run: true,
            packages: context.packages,
        });
    }

    if options.semver_checks || context.config.semver_checks {
        run_semver_checks(&options.root)?;
    }

    let date = context
        .git
        .head_date()
        .unwrap_or_else(|_| "1970-01-01".to_string());
    let entry = changelog_entry(&context, &next, &date)?;

    let mut versions: BTreeMap<String, Version> = BTreeMap::new();
    if context.config.mode == PackageMode::Independent {
        for package in &context.packages {
            let Some(package_next) = package
                .next
                .as_ref()
                .and_then(|raw| Version::parse(raw).ok())
            else {
                continue;
            };
            manifest::write_version(Path::new(&package.manifest), &package_next)?;
            if let Some(name) = &package.name {
                versions.insert(name.clone(), package_next);
            }
        }
    } else {
        manifest::write_version(&options.root.join("Cargo.toml"), &next)?;
        if let Ok(Some(name)) = manifest::read_package_name(&options.root.join("Cargo.toml")) {
            versions.insert(name, next.clone());
        }
    }
    for manifest_path in &manifests {
        if let Ok(Some(name)) = manifest::read_package_name(manifest_path) {
            versions.entry(name).or_insert_with(|| next.clone());
        }
        manifest::update_path_dependencies(manifest_path, &versions)?;
    }
    refresh_lockfile(&options.root)?;

    let changelog_path = context.config.changelog_path(&options.root);
    let existing = std::fs::read_to_string(&changelog_path).unwrap_or_default();
    let locale = context.config.locale;
    let changelog_written = match changelog::insert_entry(&existing, &entry, &next, locale) {
        Some(updated) => {
            std::fs::write(&changelog_path, updated)?;
            true
        }
        None => false,
    };

    let tag = context
        .tag
        .clone()
        .unwrap_or_else(|| context.config.tag_for(&next));
    context.git.add_all()?;
    let message = format!("chore(release): {tag}");
    context
        .git
        .commit(&message, options.no_verify || options.ci)?;
    context.git.create_tag(&tag, &tag)?;

    if options.push {
        let branch = match &context.branch {
            Some(branch) => branch.clone(),
            None => context.git.current_branch()?,
        };
        context.git.push(&branch, Some(&tag))?;
    }
    if options.package {
        package(&manifests)?;
    }
    if options.publish {
        publish(&manifests, false)?;
    }
    if options.github_release {
        github_release(&tag, &entry, &options.root)?;
    }

    Ok(Outcome {
        previous: context.current.to_string(),
        next: Some(next.to_string()),
        bump: context.plan.bump,
        tag: Some(tag),
        commits: context.commits.len(),
        changelog_written,
        committed: true,
        tagged: true,
        published: options.publish,
        skipped: false,
        dry_run: false,
        packages: context.packages,
    })
}

/// Проверяет Conventional Commits в диапазоне.
///
/// # Errors
///
/// [`ReleaseError`] — git/конфиг.
pub fn check_commits(options: &Options) -> Result<CheckReport> {
    let config = ReleaseConfig::load(&options.root, options.config_path.as_deref())?;
    let git = Git::new(&options.root);
    let prefix = tag_prefix(options, &config);
    let (from, to) = match &options.commits {
        Some(range) => {
            let Some((from, to)) = range.split_once("..") else {
                return Err(ReleaseError::Config(format!(
                    "--commits expects 'from..to', got '{range}'"
                )));
            };
            (
                if from.is_empty() {
                    None
                } else {
                    Some(from.to_string())
                },
                if to.is_empty() {
                    "HEAD".to_string()
                } else {
                    to.to_string()
                },
            )
        }
        None => {
            let from = match &options.from {
                Some(from) => Some(from.clone()),
                None => git.last_tag(&prefix)?,
            };
            (
                from,
                options.to.clone().unwrap_or_else(|| "HEAD".to_string()),
            )
        }
    };
    let raw = git.log(from.as_deref(), &to)?;
    let mut conventional = 0;
    let mut invalid = Vec::new();
    for raw_commit in &raw {
        if commit::is_merge_commit(&raw_commit.message) {
            continue;
        }
        match commit::parse_commit(raw_commit) {
            Some(parsed) => {
                if config.is_skipped(&parsed.description) || parsed.is_release_commit() {
                    continue;
                }
                conventional += 1;
            }
            None => {
                let header = raw_commit
                    .message
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .to_string();
                invalid.push(header);
            }
        }
    }
    Ok(CheckReport {
        total: raw.len(),
        conventional,
        invalid,
        config_ok: true,
    })
}

/// Создаёт `.releaserc.toml` и `CHANGELOG.md`.
///
/// # Errors
///
/// [`ReleaseError::Config`] — файл уже существует без `force`; io-ошибки.
pub fn init(root: &Path, force: bool, locale: Locale) -> Result<Vec<PathBuf>> {
    const DEFAULT_CONFIG: &str = "# releaser-rs\n\
branches = [\"dev\", \"main\", \"master\"]\n\
tag_format = \"v{version}\"\n\
changelog = \"CHANGELOG.md\"\n\
initial_version = \"0.1.0\"\n\
mode = \"fixed\"\n\
locale = \"en\"\n\
\n\
# Монорепо: раскомментируйте и задайте mode = \"independent\".\n\
# [[packages]]\n\
# name = \"demo\"\n\
# manifest = \"crates/demo/Cargo.toml\"\n";
    let config_path = root.join(".releaserc.toml");
    if config_path.exists() && !force {
        return Err(ReleaseError::Config(format!(
            "{} already exists (use --force)",
            config_path.display()
        )));
    }
    std::fs::write(&config_path, DEFAULT_CONFIG)?;
    let changelog_path = root.join("CHANGELOG.md");
    if !changelog_path.exists() || force {
        std::fs::write(&changelog_path, changelog::header(locale))?;
    }
    Ok(vec![config_path, changelog_path])
}

fn command_available(name: &str, args: &[&str]) -> bool {
    Command::new(name)
        .args(args)
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Диагностика окружения.
#[must_use]
pub fn doctor(root: &Path, config_path: Option<&Path>) -> DoctorReport {
    let git = Git::new(root);
    let config = ReleaseConfig::load(root, config_path);
    let version = manifest::read_version(&root.join("Cargo.toml"))
        .ok()
        .map(|version| version.to_string());
    let prefix = config
        .as_ref()
        .map(|config| config.tag_prefix().to_string())
        .unwrap_or_default();
    let tags = git.last_tag(&prefix).ok().flatten().map(|_| 1).unwrap_or(0);
    DoctorReport {
        git_repo: git.is_repo(),
        clean: git.is_clean().unwrap_or(false),
        branch: git.current_branch().ok(),
        config_ok: config.is_ok(),
        version,
        tags,
        cargo: command_available("cargo", &["--version"]),
        gh: command_available("gh", &["--version"]),
        semver_checks: command_available("cargo", &["semver-checks", "--version"]),
    }
}
