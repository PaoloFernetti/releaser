//! CLI релиз-инструмента .

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};
use releaser_rs::config::Locale;
use releaser_rs::release::{self, Options};
use releaser_rs::ReleaseError;

#[derive(Parser)]
#[command(
    name = "releaser-rs",
    version,
    about = "Semantic-release для Rust: Conventional Commits → semver, CHANGELOG, теги"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
    /// Корень проекта.
    #[arg(long, global = true, default_value = ".")]
    root: PathBuf,
    /// Явный путь к конфигу.
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    /// Машинный вывод.
    #[arg(long, global = true)]
    json: bool,
    /// Ничего не писать.
    #[arg(long, global = true)]
    dry_run: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Выполнить релиз.
    Release(ReleaseArgs),
    /// Показать следующую версию.
    Version(RangeArgs),
    /// Показать запись changelog.
    Changelog(RangeArgs),
    /// Проверить Conventional Commits.
    Check(CheckArgs),
    /// Создать конфиг и CHANGELOG.
    Init(InitArgs),
    /// Диагностика окружения.
    Doctor,
}

#[derive(Args)]
struct RangeArgs {
    /// Начало диапазона коммитов.
    #[arg(long)]
    from: Option<String>,
    /// Конец диапазона коммитов.
    #[arg(long)]
    to: Option<String>,
    /// Pre-release-канал (alpha/beta/rc).
    #[arg(long)]
    prerelease: Option<String>,
    /// Внешний шаблон changelog.
    #[arg(long)]
    template: Option<PathBuf>,
    /// Первый релиз: взять initial_version.
    #[arg(long)]
    first_release: bool,
}

#[derive(Args)]
struct ReleaseArgs {
    #[command(flatten)]
    range: RangeArgs,
    /// Переопределение префикса тегов.
    #[arg(long)]
    tag_prefix: Option<String>,
    /// Выполнить cargo package после тега.
    #[arg(long)]
    package: bool,
    /// Не запускать git-хуки.
    #[arg(long)]
    no_verify: bool,
    /// Не проверять git-состояние.
    #[arg(long)]
    no_git_checks: bool,
    /// Выполнить cargo publish после тега.
    #[arg(long)]
    publish: bool,
    /// Выполнить git push после тега.
    #[arg(long)]
    push: bool,
    /// Создать GitHub Release.
    #[arg(long)]
    github_release: bool,
    /// Запустить cargo-semver-checks.
    #[arg(long)]
    semver_checks: bool,
    /// CI-режим.
    #[arg(long)]
    ci: bool,
}

#[derive(Args)]
struct CheckArgs {
    #[command(flatten)]
    range: RangeArgs,
    /// Диапазон `from..to` (например, `origin/main..HEAD`).
    #[arg(long)]
    commits: Option<String>,
    /// Ненулевой exit code при нарушениях.
    #[arg(long)]
    strict: bool,
}

#[derive(Args)]
struct InitArgs {
    /// Перезаписать существующие файлы.
    #[arg(long)]
    force: bool,
    /// Язык changelog.
    #[arg(long, value_enum, default_value = "en")]
    locale: LocaleArg,
}

#[derive(Clone, Copy, ValueEnum)]
enum LocaleArg {
    En,
    Ru,
}

impl From<LocaleArg> for Locale {
    fn from(value: LocaleArg) -> Self {
        match value {
            LocaleArg::En => Locale::En,
            LocaleArg::Ru => Locale::Ru,
        }
    }
}

fn options(cli: &Cli, range: &RangeArgs) -> Options {
    Options {
        root: cli.root.clone(),
        config_path: cli.config.clone(),
        dry_run: cli.dry_run,
        json: cli.json,
        from: range.from.clone(),
        to: range.to.clone(),
        prerelease: range.prerelease.clone(),
        template: range.template.clone(),
        first_release: range.first_release,
        ..Options::default()
    }
}

fn release_options(cli: &Cli, args: &ReleaseArgs) -> Options {
    Options {
        tag_prefix: args.tag_prefix.clone(),
        package: args.package,
        no_verify: args.no_verify,
        no_git_checks: args.no_git_checks,
        publish: args.publish,
        push: args.push,
        github_release: args.github_release,
        semver_checks: args.semver_checks,
        ci: args.ci,
        ..options(cli, &args.range)
    }
}

fn print_json<T: serde::Serialize>(value: &T) -> Result<(), ReleaseError> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("releaser-rs: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<(), ReleaseError> {
    match &cli.command {
        Command::Release(args) => {
            let options = release_options(cli, args);
            let outcome = release::apply(&options)?;
            if options.json {
                print_json(&outcome)?;
            } else if outcome.skipped {
                println!("releaser-rs: no release ({})", outcome.previous);
            } else {
                let next = outcome.next.clone().unwrap_or_default();
                let tag = outcome.tag.clone().unwrap_or_default();
                println!(
                    "releaser-rs: {next} ({:?}), commits: {}, changelog: {}, tag: {tag}",
                    outcome.bump, outcome.commits, outcome.changelog_written
                );
                if outcome.dry_run {
                    println!("releaser-rs: dry-run, файлы не изменены");
                }
            }
        }
        Command::Version(range) => {
            let options = options(cli, range);
            let preview = release::preview(&options)?;
            if options.json {
                print_json(&preview)?;
            } else {
                println!(
                    "{}",
                    preview
                        .next
                        .clone()
                        .unwrap_or_else(|| preview.current.clone())
                );
            }
        }
        Command::Changelog(range) => {
            let options = options(cli, range);
            let preview = release::preview(&options)?;
            if options.json {
                print_json(&preview)?;
            } else if let Some(entry) = preview.changelog {
                println!("{entry}");
            }
        }
        Command::Check(args) => {
            let mut options = options(cli, &args.range);
            options.commits = args.commits.clone();
            let report = release::check_commits(&options)?;
            if options.json {
                print_json(&report)?;
            } else {
                println!(
                    "commits: {}, conventional: {}, invalid: {}",
                    report.total,
                    report.conventional,
                    report.invalid.len()
                );
                for line in &report.invalid {
                    println!("  {line}");
                }
            }
            if args.strict && !report.invalid.is_empty() {
                return Err(ReleaseError::Config(format!(
                    "{} commit(s) are not Conventional Commits",
                    report.invalid.len()
                )));
            }
        }
        Command::Init(args) => {
            let paths = release::init(&cli.root, args.force, args.locale.into())?;
            for path in paths {
                println!("releaser-rs: created {}", path.display());
            }
        }
        Command::Doctor => {
            let report = release::doctor(&cli.root, cli.config.as_deref());
            if cli.json {
                print_json(&report)?;
            } else {
                println!("git repo:       {}", report.git_repo);
                println!("clean:          {}", report.clean);
                println!("branch:         {}", report.branch.unwrap_or_default());
                println!("config ok:      {}", report.config_ok);
                println!("version:        {}", report.version.unwrap_or_default());
                println!("tags:           {}", report.tags);
                println!("cargo:          {}", report.cargo);
                println!("gh:             {}", report.gh);
                println!("semver-checks:  {}", report.semver_checks);
            }
        }
    }
    Ok(())
}
