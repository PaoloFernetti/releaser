//! Conventional Commits 1.0: парсинг заголовка, scope, `!` и breaking-футеров.

/// Сырой коммит из `git log`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawCommit {
    /// Полный хэш.
    pub hash: String,
    /// Короткий хэш.
    pub short_hash: String,
    /// Дата автора (`YYYY-MM-DD`).
    pub date: String,
    /// Полное сообщение.
    pub message: String,
}

/// Разобранный Conventional Commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConventionalCommit {
    /// Полный хэш.
    pub hash: String,
    /// Короткий хэш.
    pub short_hash: String,
    /// Дата автора.
    pub date: String,
    /// Тип (`feat`, `fix`, ...).
    pub kind: String,
    /// Scope, если указан.
    pub scope: Option<String>,
    /// Описание без типа/scope.
    pub description: String,
    /// Ломающее изменение (`!` или `BREAKING CHANGE`).
    pub breaking: bool,
    /// Тексты breaking-футеров.
    pub breaking_notes: Vec<String>,
}

impl ConventionalCommit {
    /// Строка для changelog: `scope: description` или просто описание.
    #[must_use]
    pub fn summary(&self) -> String {
        match &self.scope {
            Some(scope) => format!("**{scope}:** {}", self.description),
            None => self.description.clone(),
        }
    }

    /// Release-коммит (`chore(release)`), который не попадает в changelog.
    #[must_use]
    pub fn is_release_commit(&self) -> bool {
        self.kind == "chore" && self.scope.as_deref() == Some("release")
    }
}

/// Merge-коммит.
#[must_use]
pub fn is_merge_commit(message: &str) -> bool {
    message.starts_with("Merge ")
        || message.starts_with("Merge branch ")
        || message.starts_with("Merge pull request ")
}

/// Разбирает сырой коммит; `None` — не Conventional Commits или merge.
#[must_use]
pub fn parse_commit(raw: &RawCommit) -> Option<ConventionalCommit> {
    if is_merge_commit(&raw.message) {
        return None;
    }
    let mut lines = raw.message.lines();
    let header = lines.next()?.trim();
    let (left, description) = header.split_once(": ")?;
    let description = description.trim();
    if description.is_empty() {
        return None;
    }

    let mut left = left.trim();
    let mut breaking = false;
    if let Some(stripped) = left.strip_suffix('!') {
        left = stripped;
        breaking = true;
    }

    let (kind, scope) = match left.split_once('(') {
        Some((kind, rest)) if rest.ends_with(')') => {
            let scope = rest[..rest.len() - 1].trim();
            if scope.is_empty() {
                return None;
            }
            (kind.trim(), Some(scope.to_string()))
        }
        Some(_) => return None,
        None => (left, None),
    };
    if kind.is_empty()
        || !kind
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return None;
    }

    let mut breaking_notes = Vec::new();
    let mut collecting = false;
    for line in lines {
        let trimmed = line.trim();
        let footer = trimmed
            .strip_prefix("BREAKING CHANGE:")
            .or_else(|| trimmed.strip_prefix("BREAKING-CHANGE:"));
        if let Some(note) = footer {
            breaking = true;
            collecting = true;
            let note = note.trim();
            if !note.is_empty() {
                breaking_notes.push(note.to_string());
            }
            continue;
        }
        if collecting {
            if trimmed.is_empty() {
                collecting = false;
            } else {
                breaking_notes.push(trimmed.to_string());
            }
        }
    }

    Some(ConventionalCommit {
        hash: raw.hash.clone(),
        short_hash: raw.short_hash.clone(),
        date: raw.date.clone(),
        kind: kind.to_ascii_lowercase(),
        scope,
        description: description.to_string(),
        breaking,
        breaking_notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(message: &str) -> RawCommit {
        RawCommit {
            hash: "0123456789abcdef".into(),
            short_hash: "0123456".into(),
            date: "2026-11-14".into(),
            message: message.into(),
        }
    }

    #[test]
    fn parses_feat_with_scope() {
        let commit = parse_commit(&raw("feat(render): add bloom pass\n\nbody")).unwrap();
        assert_eq!(commit.kind, "feat");
        assert_eq!(commit.scope.as_deref(), Some("render"));
        assert_eq!(commit.description, "add bloom pass");
        assert!(!commit.breaking);
        assert_eq!(commit.summary(), "**render:** add bloom pass");
    }

    #[test]
    fn bang_marks_breaking() {
        let commit = parse_commit(&raw("fix(api)!: drop old signature")).unwrap();
        assert!(commit.breaking);
    }

    #[test]
    fn breaking_footer_is_collected() {
        let commit = parse_commit(&raw(
            "feat: new api\n\nBREAKING CHANGE: Renderer::new signature changed\nsecond line",
        ))
        .unwrap();
        assert!(commit.breaking);
        assert_eq!(
            commit.breaking_notes,
            vec![
                "Renderer::new signature changed".to_string(),
                "second line".to_string()
            ]
        );
    }

    #[test]
    fn rejects_merge_and_non_conventional() {
        assert!(parse_commit(&raw("Merge branch 'main'")).is_none());
        assert!(parse_commit(&raw("just a message")).is_none());
        assert!(parse_commit(&raw("feat: ")).is_none());
        assert!(parse_commit(&raw("feat(bad: no closing paren")).is_none());
    }

    #[test]
    fn release_commit_is_detected() {
        let commit = parse_commit(&raw("chore(release): v1.2.3")).unwrap();
        assert!(commit.is_release_commit());
    }
}
