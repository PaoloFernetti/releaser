//! Вычисление следующей версии по Conventional Commits.

use semver::{Prerelease, Version};

use crate::commit::ConventionalCommit;
use crate::config::{Bump, ReleaseConfig};
use crate::error::{ReleaseError, Result};

/// План версии: текущая, следующая (если есть изменения) и вклад коммитов.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BumpPlan {
    /// Текущая версия.
    pub current: Version,
    /// Следующая версия; `None` — изменений нет.
    pub next: Option<Version>,
    /// Вклад коммитов.
    pub bump: Bump,
}

/// Определяет максимальный bump по коммитам.
#[must_use]
pub fn decide_bump(
    commits: &[ConventionalCommit],
    config: &ReleaseConfig,
    current: &Version,
) -> Bump {
    let mut bump = Bump::None;
    for commit in commits {
        if commit.is_release_commit() || config.is_skipped(&commit.description) {
            continue;
        }
        if let Some(kind) = config.commit_type(&commit.kind) {
            bump = bump.max(kind.bump);
        }
        if commit.breaking {
            let breaking = if current.major == 0 && !config.bump_major_on_breaking_below_1_0 {
                Bump::Minor
            } else {
                Bump::Major
            };
            bump = bump.max(breaking);
        }
    }
    bump
}

/// Применяет bump и pre-release-канал к версии.
///
/// # Errors
///
/// [`ReleaseError::Version`] — невалидный канал или переполнение версии.
pub fn next_version(current: &Version, bump: Bump, prerelease: Option<&str>) -> Result<Version> {
    let mut next = match bump {
        Bump::None => current.clone(),
        Bump::Patch => Version::new(current.major, current.minor, current.patch + 1),
        Bump::Minor => Version::new(current.major, current.minor + 1, 0),
        Bump::Major => Version::new(current.major + 1, 0, 0),
    };
    if let Some(channel) = prerelease {
        if channel.is_empty()
            || !channel
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
        {
            return Err(ReleaseError::Version(format!(
                "invalid prerelease channel '{channel}'"
            )));
        }
        let prefix = format!("{channel}.");
        let number = if current.pre.as_str().starts_with(&prefix) {
            current
                .pre
                .as_str()
                .rsplit('.')
                .next()
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(0)
                .saturating_add(1)
        } else {
            1
        };
        next.pre = Prerelease::new(&format!("{channel}.{number}"))
            .map_err(|error| ReleaseError::Version(format!("prerelease: {error}")))?;
    }
    Ok(next)
}

/// Строит план: bump по коммитам и следующая версия.
///
/// # Errors
///
/// [`ReleaseError::Version`] — невалидная версия/канал.
pub fn plan(
    commits: &[ConventionalCommit],
    config: &ReleaseConfig,
    current: &Version,
    prerelease: Option<&str>,
    first_release: bool,
) -> Result<BumpPlan> {
    let bump = decide_bump(commits, config, current);
    if first_release {
        return Ok(BumpPlan {
            current: current.clone(),
            next: Some(config.initial_version()?),
            bump,
        });
    }
    if bump == Bump::None && prerelease.is_none() {
        return Ok(BumpPlan {
            current: current.clone(),
            next: None,
            bump,
        });
    }
    let effective = if bump == Bump::None {
        Bump::Patch
    } else {
        bump
    };
    Ok(BumpPlan {
        current: current.clone(),
        next: Some(next_version(current, effective, prerelease)?),
        bump,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commit::ConventionalCommit;

    fn commit(kind: &str, breaking: bool) -> ConventionalCommit {
        ConventionalCommit {
            hash: "0".into(),
            short_hash: "0".into(),
            date: "2026-11-14".into(),
            kind: kind.into(),
            scope: None,
            description: "change".into(),
            breaking,
            breaking_notes: Vec::new(),
        }
    }

    #[test]
    fn bump_mapping_follows_conventional_commits() {
        let config = ReleaseConfig::default();
        let current = Version::parse("1.2.3").unwrap();
        assert_eq!(
            decide_bump(&[commit("fix", false)], &config, &current),
            Bump::Patch
        );
        assert_eq!(
            decide_bump(&[commit("feat", false)], &config, &current),
            Bump::Minor
        );
        assert_eq!(
            decide_bump(&[commit("chore", false)], &config, &current),
            Bump::None
        );
        assert_eq!(
            decide_bump(&[commit("feat", true)], &config, &current),
            Bump::Major
        );
    }

    #[test]
    fn breaking_below_one_point_zero_is_minor_by_default() {
        let config = ReleaseConfig::default();
        let current = Version::parse("0.4.2").unwrap();
        assert_eq!(
            decide_bump(&[commit("fix", true)], &config, &current),
            Bump::Minor
        );
        let strict = ReleaseConfig {
            bump_major_on_breaking_below_1_0: true,
            ..ReleaseConfig::default()
        };
        assert_eq!(
            decide_bump(&[commit("fix", true)], &strict, &current),
            Bump::Major
        );
    }

    #[test]
    fn prerelease_increments_number() {
        let current = Version::parse("1.2.0-alpha.1").unwrap();
        let next = next_version(&current, Bump::None, Some("alpha")).unwrap();
        assert_eq!(next.to_string(), "1.2.0-alpha.2");
        let switched = next_version(&current, Bump::None, Some("beta")).unwrap();
        assert_eq!(switched.to_string(), "1.2.0-beta.1");
        assert!(next_version(&current, Bump::None, Some("bad channel")).is_err());
    }

    #[test]
    fn plan_is_empty_without_relevant_commits() {
        let config = ReleaseConfig::default();
        let current = Version::parse("1.2.3").unwrap();
        let empty = plan(&[commit("chore", false)], &config, &current, None, false).unwrap();
        assert_eq!(empty.next, None);
        let fixed = plan(&[commit("fix", false)], &config, &current, None, false).unwrap();
        assert_eq!(fixed.next.unwrap().to_string(), "1.2.4");
    }
}
