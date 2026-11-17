//! Типизированные ошибки релиз-инструмента.

use std::path::PathBuf;

/// Ошибка релиза.
#[derive(Debug, thiserror::Error)]
pub enum ReleaseError {
    /// Некорректная конфигурация.
    #[error("конфигурация: {0}")]
    Config(String),
    /// Ошибка git-операции.
    #[error("git: {0}")]
    Git(String),
    /// Ошибка чтения/записи манифеста.
    #[error("манифест {path}: {message}")]
    Manifest {
        /// Путь к манифесту.
        path: PathBuf,
        /// Причина.
        message: String,
    },
    /// Ошибка changelog.
    #[error("changelog: {0}")]
    Changelog(String),
    /// Ошибка версии.
    #[error("версия: {0}")]
    Version(String),
    /// Ошибка внешней команды (cargo publish, gh, cargo-semver-checks).
    #[error("команда: {0}")]
    Command(String),
    /// Ошибка ввода-вывода.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// Ошибка JSON.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

/// Результат релиз-операции.
pub type Result<T> = std::result::Result<T, ReleaseError>;
