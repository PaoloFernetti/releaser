//! Конфигурация релиза: `.releaserc.toml` и импорт subset `.releaserc.json`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use semver::Version;
use serde::{Deserialize, Serialize};

use crate::error::{ReleaseError, Result};

/// Режим версионирования пакетов.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PackageMode {
    /// Одна версия на все пакеты.
    #[default]
    Fixed,
    /// Независимые версии пакетов.
    Independent,
}

/// Язык changelog.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Locale {
    /// Английский.
    #[default]
    En,
    /// Русский.
    Ru,
}

/// Вклад коммита в версию.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Bump {
    /// Не влияет на версию.
    #[default]
    None,
    /// Патч.
    Patch,
    /// Минор.
    Minor,
    /// Мажор.
    Major,
}

/// Настройка типа коммита.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommitTypeConfig {
    /// Секция changelog (`Added`, `Fixed`, ... или произвольная).
    pub section: String,
    /// Вклад в версию.
    pub bump: Bump,
}

/// Пакет монорепозитория.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackageConfig {
    /// Имя пакета (для path-зависимостей и отчётов).
    pub name: Option<String>,
    /// Путь к `Cargo.toml`.
    pub manifest: PathBuf,
}

/// Конфигурация релиза.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ReleaseConfig {
    /// Ветки, на которых разрешён релиз.
    pub branches: Vec<String>,
    /// Формат тега; обязан содержать `{version}`.
    pub tag_format: String,
    /// Путь к changelog относительно корня.
    pub changelog: PathBuf,
    /// Версия для первого релиза.
    pub initial_version: String,
    /// Режим версионирования пакетов.
    pub mode: PackageMode,
    /// Язык changelog.
    pub locale: Locale,
    /// Внешний шаблон changelog (заменяет встроенный).
    pub template: Option<PathBuf>,
    /// Пакеты монорепозитория (пусто — корневой `Cargo.toml`).
    pub packages: Vec<PackageConfig>,
    /// Типы коммитов и их секции.
    pub commit_types: BTreeMap<String, CommitTypeConfig>,
    /// Маркеры пропуска релиза в заголовке коммита.
    pub skip_markers: Vec<String>,
    /// Для `0.x`: breaking даёт major (иначе minor).
    pub bump_major_on_breaking_below_1_0: bool,
    /// Разрешить `cargo-semver-checks` (включается флагом CLI).
    pub semver_checks: bool,
}

impl Default for ReleaseConfig {
    fn default() -> Self {
        Self {
            branches: vec!["dev".to_string(), "main".to_string(), "master".to_string()],
            tag_format: "v{version}".to_string(),
            changelog: PathBuf::from("CHANGELOG.md"),
            initial_version: "0.1.0".to_string(),
            mode: PackageMode::Fixed,
            locale: Locale::En,
            template: None,
            packages: Vec::new(),
            commit_types: default_commit_types(),
            skip_markers: vec!["[skip release]".to_string()],
            bump_major_on_breaking_below_1_0: false,
            semver_checks: false,
        }
    }
}

/// Стандартная таблица типов Conventional Commits.
#[must_use]
pub fn default_commit_types() -> BTreeMap<String, CommitTypeConfig> {
    let mut types = BTreeMap::new();
    let mut insert = |kind: &str, section: &str, bump: Bump| {
        types.insert(
            kind.to_string(),
            CommitTypeConfig {
                section: section.to_string(),
                bump,
            },
        );
    };
    insert("feat", "Features", Bump::Minor);
    insert("fix", "Fixes", Bump::Patch);
    insert("perf", "Performance", Bump::Patch);
    insert("revert", "Reverts", Bump::Patch);
    insert("refactor", "Refactoring", Bump::None);
    insert("docs", "Documentation", Bump::None);
    insert("build", "Internal", Bump::None);
    insert("ci", "Internal", Bump::None);
    insert("test", "Internal", Bump::None);
    insert("chore", "Internal", Bump::None);
    insert("style", "Internal", Bump::None);
    types
}

impl ReleaseConfig {
    /// Загружает конфиг: явный путь, `.releaserc.toml`/`release.toml`, иначе
    /// умолчания с импортом subset `.releaserc.json`.
    ///
    /// # Errors
    ///
    /// [`ReleaseError::Config`] — файл не читается/не парсится или невалиден.
    pub fn load(root: &Path, explicit: Option<&Path>) -> Result<Self> {
        let candidate = explicit.map(Path::to_path_buf).or_else(|| {
            [".releaserc.toml", "release.toml"]
                .iter()
                .map(|name| root.join(name))
                .find(|path| path.exists())
        });
        let mut config = if let Some(path) = candidate {
            let content = std::fs::read_to_string(&path)
                .map_err(|error| ReleaseError::Config(format!("{}: {error}", path.display())))?;
            toml_edit::de::from_str::<Self>(&content)
                .map_err(|error| ReleaseError::Config(format!("{}: {error}", path.display())))?
        } else {
            Self::default()
        };
        let json = root.join(".releaserc.json");
        if json.exists() {
            config.import_json(&json)?;
        }
        config.validate()?;
        Ok(config)
    }

    /// Импортирует subset semantic-release-конфига: `branches`, `tagFormat`.
    ///
    /// # Errors
    ///
    /// [`ReleaseError::Config`] — JSON не читается или невалиден.
    pub fn import_json(&mut self, path: &Path) -> Result<()> {
        #[derive(Deserialize)]
        struct JsonConfig {
            #[serde(default)]
            branches: Vec<String>,
            #[serde(default, rename = "tagFormat")]
            tag_format: Option<String>,
        }
        let content = std::fs::read_to_string(path)
            .map_err(|error| ReleaseError::Config(format!("{}: {error}", path.display())))?;
        let parsed: JsonConfig = serde_json::from_str(&content)
            .map_err(|error| ReleaseError::Config(format!("{}: {error}", path.display())))?;
        if !parsed.branches.is_empty() {
            self.branches = parsed.branches;
        }
        if let Some(format) = parsed.tag_format {
            self.tag_format = format;
        }
        Ok(())
    }

    /// Проверяет конфиг.
    ///
    /// # Errors
    ///
    /// [`ReleaseError::Config`] — пустые ветки, нет `{version}` в теге,
    /// невалидная стартовая версия или пустой independent-режим.
    pub fn validate(&self) -> Result<()> {
        if self.branches.is_empty() {
            return Err(ReleaseError::Config("branches must not be empty".into()));
        }
        if !self.tag_format.contains("{version}") {
            return Err(ReleaseError::Config(
                "tag_format must contain {version}".into(),
            ));
        }
        Version::parse(&self.initial_version)
            .map_err(|error| ReleaseError::Config(format!("initial_version: {error}")))?;
        if self.mode == PackageMode::Independent && self.packages.is_empty() {
            return Err(ReleaseError::Config(
                "independent mode requires at least one package".into(),
            ));
        }
        for package in &self.packages {
            if package.manifest.as_os_str().is_empty() {
                return Err(ReleaseError::Config(
                    "package manifest path must not be empty".into(),
                ));
            }
        }
        Ok(())
    }

    /// Тег для версии.
    #[must_use]
    pub fn tag_for(&self, version: &Version) -> String {
        self.tag_format.replace("{version}", &version.to_string())
    }

    /// Префикс тегов (часть `tag_format` до `{version}`).
    #[must_use]
    pub fn tag_prefix(&self) -> &str {
        self.tag_format
            .split_once("{version}")
            .map_or("", |(prefix, _)| prefix)
    }

    /// Стартовая версия.
    ///
    /// # Errors
    ///
    /// [`ReleaseError::Version`] — `initial_version` не парсится.
    pub fn initial_version(&self) -> Result<Version> {
        Version::parse(&self.initial_version)
            .map_err(|error| ReleaseError::Version(format!("initial_version: {error}")))
    }

    /// Настройка типа коммита.
    #[must_use]
    pub fn commit_type(&self, kind: &str) -> Option<&CommitTypeConfig> {
        self.commit_types.get(kind)
    }

    /// Содержит ли заголовок маркер пропуска релиза.
    #[must_use]
    pub fn is_skipped(&self, subject: &str) -> bool {
        self.skip_markers
            .iter()
            .any(|marker| subject.contains(marker.as_str()))
    }

    /// Путь к changelog.
    #[must_use]
    pub fn changelog_path(&self, root: &Path) -> PathBuf {
        root.join(&self.changelog)
    }

    /// Манифесты, которые нужно обновить.
    #[must_use]
    pub fn manifests(&self, root: &Path) -> Vec<PathBuf> {
        if self.packages.is_empty() {
            vec![root.join("Cargo.toml")]
        } else {
            self.packages
                .iter()
                .map(|package| root.join(&package.manifest))
                .collect()
        }
    }
}
