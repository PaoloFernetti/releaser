//! Генерация `CHANGELOG.md` в формате Keep a Changelog.

use std::collections::BTreeMap;

use semver::Version;

use crate::commit::ConventionalCommit;
use crate::config::{Locale, ReleaseConfig};

const HEADER_EN: &str = "# Changelog

All notable changes to this project are documented here.
The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).
";

const HEADER_RU: &str = "# Changelog

Все заметные изменения проекта документируются здесь.
Формат — [Keep a Changelog](https://keepachangelog.com/ru/1.1.0/),
версионирование — [Semantic Versioning](https://semver.org/lang/ru/).
";

const ORDER: [&str; 8] = [
    "Breaking",
    "Features",
    "Fixes",
    "Performance",
    "Refactoring",
    "Reverts",
    "Documentation",
    "Internal",
];

fn canonical_section(section: &str) -> &str {
    match section {
        "Added" => "Features",
        "Fixed" => "Fixes",
        "Changed" => "Refactoring",
        other => other,
    }
}

fn module_title(scope: Option<&str>, locale: Locale) -> String {
    match scope {
        Some(scope) => scope.to_string(),
        None => match locale {
            Locale::En => "General".to_string(),
            Locale::Ru => "Общее".to_string(),
        },
    }
}

/// Заголовок нового changelog.
#[must_use]
pub fn header(locale: Locale) -> &'static str {
    match locale {
        Locale::En => HEADER_EN,
        Locale::Ru => HEADER_RU,
    }
}

/// Локализованный заголовок секции.
#[must_use]
pub fn section_title(section: &str, locale: Locale) -> String {
    let localized = match (section, locale) {
        ("Breaking", Locale::En) => "⚠ BREAKING CHANGES",
        ("Breaking", Locale::Ru) => "⚠ Ломающие изменения",
        ("Features", Locale::En) | ("Added", Locale::En) => "Features",
        ("Features", Locale::Ru) | ("Added", Locale::Ru) => "Функции",
        ("Fixes", Locale::En) | ("Fixed", Locale::En) => "Fixes",
        ("Fixes", Locale::Ru) | ("Fixed", Locale::Ru) => "Исправления",
        ("Performance", Locale::En) => "Performance",
        ("Performance", Locale::Ru) => "Производительность",
        ("Refactoring", Locale::En) | ("Changed", Locale::En) => "Refactoring",
        ("Refactoring", Locale::Ru) | ("Changed", Locale::Ru) => "Рефакторинг",
        ("Reverts", Locale::En) => "Reverts",
        ("Reverts", Locale::Ru) => "Откаты",
        ("Documentation", Locale::En) => "Documentation",
        ("Documentation", Locale::Ru) => "Документация",
        ("Internal", Locale::En) => "Internal",
        ("Internal", Locale::Ru) => "Внутреннее",
        _ => section,
    };
    localized.to_string()
}

/// Рендерит только секции изменений (без заголовка версии).
#[must_use]
pub fn render_sections(
    commits: &[ConventionalCommit],
    config: &ReleaseConfig,
    repo_url: Option<&str>,
) -> String {
    // Секция → модуль → строки. BTreeMap даёт детерминированный порядок.
    let mut sections: BTreeMap<String, BTreeMap<String, Vec<String>>> = BTreeMap::new();
    for commit in commits {
        if commit.is_release_commit() || config.is_skipped(&commit.description) {
            continue;
        }
        let Some(kind) = config.commit_type(&commit.kind) else {
            continue;
        };
        let section = canonical_section(&kind.section).to_string();
        let module = module_title(commit.scope.as_deref(), config.locale);
        let mut line = format!("- {}", commit.description);
        if let Some(url) = repo_url {
            let link = format!("{}/commit/{}", url.trim_end_matches('/'), commit.hash);
            line.push_str(&format!(" ([{}]({link}))", commit.short_hash));
        }
        if commit.breaking {
            let notes = if commit.breaking_notes.is_empty() {
                vec![commit.description.clone()]
            } else {
                commit.breaking_notes.clone()
            };
            let mut block = format!("- {}", commit.description);
            for note in notes {
                block.push_str(&format!("\n  - {note}"));
            }
            sections
                .entry("Breaking".to_string())
                .or_default()
                .entry(module.clone())
                .or_default()
                .push(block);
        }
        sections
            .entry(section)
            .or_default()
            .entry(module)
            .or_default()
            .push(line);
    }

    let mut output = String::new();
    // Секции в фиксированном порядке, незнакомые — в конце по алфавиту.
    let mut keys: Vec<String> = ORDER
        .iter()
        .filter(|section| sections.contains_key(**section))
        .map(|section| (*section).to_string())
        .collect();
    for section in sections.keys() {
        if !keys.iter().any(|known| known == section) {
            keys.push(section.clone());
        }
    }
    for key in keys {
        let Some(modules) = sections.get(&key) else {
            continue;
        };
        output.push_str(&format!("\n### {}\n", section_title(&key, config.locale)));
        for (module, lines) in modules {
            if lines.is_empty() {
                continue;
            }
            output.push_str(&format!("\n#### {module}\n\n"));
            for line in lines {
                output.push_str(line);
                output.push('\n');
            }
        }
    }
    output
}

/// Рендерит запись для версии.
#[must_use]
pub fn render_entry(
    version: &Version,
    date: &str,
    commits: &[ConventionalCommit],
    config: &ReleaseConfig,
    repo_url: Option<&str>,
) -> String {
    let sections = render_sections(commits, config, repo_url);
    format!("## [{version}] - {date}\n{sections}")
}

/// Есть ли в changelog запись для версии.
#[must_use]
pub fn contains_version(existing: &str, version: &Version) -> bool {
    existing.contains(&format!("## [{version}]"))
}

/// Вставляет запись в существующий changelog; `None` — версия уже есть.
#[must_use]
pub fn insert_entry(
    existing: &str,
    entry: &str,
    version: &Version,
    locale: Locale,
) -> Option<String> {
    if contains_version(existing, version) {
        return None;
    }
    let entry = entry.trim_end();
    if existing.trim().is_empty() {
        return Some(format!("{}\n{entry}\n", header(locale)));
    }
    let marker = "\n## ";
    if let Some(index) = existing.find(marker) {
        let (before, after) = existing.split_at(index + 1);
        return Some(format!("{before}{entry}\n\n{after}"));
    }
    let mut output = existing.trim_end().to_string();
    output.push_str("\n\n");
    output.push_str(entry);
    output.push('\n');
    Some(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commit::ConventionalCommit;

    fn commit(kind: &str, scope: Option<&str>, breaking: bool) -> ConventionalCommit {
        ConventionalCommit {
            hash: "abcdef123456".into(),
            short_hash: "abcdef1".into(),
            date: "2026-11-14".into(),
            kind: kind.into(),
            scope: scope.map(str::to_string),
            description: "change".into(),
            breaking,
            breaking_notes: if breaking {
                vec!["api changed".into()]
            } else {
                Vec::new()
            },
        }
    }

    #[test]
    fn renders_sections_and_links() {
        let config = ReleaseConfig::default();
        let entry = render_entry(
            &Version::parse("1.2.0").unwrap(),
            "2026-11-14",
            &[
                commit("feat", Some("render"), false),
                commit("fix", None, false),
                commit("feat", None, true),
            ],
            &config,
            Some("https://github.com/o/r"),
        );
        assert!(entry.starts_with("## [1.2.0] - 2026-11-14"));
        assert!(entry.contains("### ⚠ BREAKING CHANGES"));
        assert!(entry.contains("### Features"));
        assert!(entry.contains("### Fixes"));
        assert!(entry.contains("#### render"));
        assert!(entry.contains("#### General"));
        assert!(entry.contains("- change"));
        assert!(entry.contains("[abcdef1](https://github.com/o/r/commit/abcdef123456)"));
    }

    #[test]
    fn localizes_sections() {
        let config = ReleaseConfig {
            locale: Locale::Ru,
            ..ReleaseConfig::default()
        };
        let entry = render_entry(
            &Version::parse("1.0.0").unwrap(),
            "2026-11-14",
            &[commit("feat", None, false)],
            &config,
            None,
        );
        assert!(entry.contains("### Функции"));
        assert!(entry.contains("#### Общее"));
    }

    #[test]
    fn legacy_sections_are_normalized() {
        let mut config = ReleaseConfig::default();
        config.commit_types.get_mut("feat").unwrap().section = "Added".into();
        let entry = render_entry(
            &Version::parse("1.0.0").unwrap(),
            "2026-11-14",
            &[commit("feat", Some("ecs"), false)],
            &config,
            None,
        );
        assert!(entry.contains("### Features"));
        assert!(entry.contains("#### ecs"));
        assert!(!entry.contains("### Added"));
    }

    #[test]
    fn insertion_is_idempotent() {
        let version = Version::parse("1.0.0").unwrap();
        let entry = "## [1.0.0] - 2026-11-14\n\n### Added\n\n- x\n";
        let first = insert_entry("", entry, &version, Locale::En).unwrap();
        assert!(first.starts_with("# Changelog"));
        assert!(first.contains("## [1.0.0]"));
        assert!(insert_entry(&first, entry, &version, Locale::En).is_none());

        let second = insert_entry(
            &first,
            "## [1.1.0] - 2026-11-15\n\n### Fixed\n\n- y\n",
            &Version::parse("1.1.0").unwrap(),
            Locale::En,
        )
        .unwrap();
        let first_pos = second.find("## [1.1.0]").unwrap();
        let old_pos = second.find("## [1.0.0]").unwrap();
        assert!(first_pos < old_pos);
    }
}
