# releaser-rs

Semantic-release для Rust-workspace: Conventional Commits → semver
bump → версия в `Cargo.toml` → `CHANGELOG.md` → commit + annotated tag.

Без Node: только `git`, `cargo` и опциональные `gh`/`cargo-semver-checks`.

## Запуск

```bash
releaser-rs --help
cargo install releaser-rs
```

## Команды

| Команда | Что делает |
|---|---|
| `release` | bump, changelog, commit, tag; `--package`/`--publish`/`--push`/`--github-release` — opt-in |
| `version` | печатает следующую версию (read-only) |
| `changelog` | печатает запись changelog (read-only) |
| `check` | проверяет Conventional Commits; `--commits from..to`, `--strict` → exit 1 |
| `init` | создаёт `.releaserc.toml` и `CHANGELOG.md` |
| `doctor` | git/config/версия/наличие `cargo`, `gh`, `cargo-semver-checks` |

Общие флаги: `--root`, `--config`, `--json`, `--dry-run`.
Диапазон: `--from`, `--to`, `--commits from..to`, `--tag-prefix`,
`--template <path>`, `--prerelease alpha|beta|rc`, `--first-release`.
`--ci` отключает git-хуки; `--package`, `--push`, `--publish` и
`--github-release` остаются opt-in.

```bash
releaser-rs release --dry-run
releaser-rs release
releaser-rs release --prerelease beta
releaser-rs check --commits "origin/main..HEAD" --strict
releaser-rs init --locale ru
```

## Формат release notes

Секции — по типу коммита, внутри — по модулю (`scope`), затем пункты:

```markdown
### Features

#### ecs
- add sparse-set iteration
- speed up archetype migration

#### graphics
- add descriptor-set cache

### Fixes

#### ecs
- fix generation reuse
```

Заголовки: `Features`, `Fixes`, `Performance`, `Refactoring`, `Reverts`,
`Documentation`, `Internal`, `⚠ BREAKING CHANGES`. Модуль без `scope`
попадает в `General`. Порядок модулей — алфавитный, порядок пунктов —
порядок коммитов. Локализация `ru` переводит заголовки секций и `General` →
`Общее`; имена модулей остаются как в `scope`.

Сообщения коммитов — `type(module): описание`; пробел перед `(` парсер
допускает, но каноническая форма — `feat(ecs): ...`.

## Правила версий

- `feat` → minor, `fix`/`perf`/`revert` → patch, `!` или `BREAKING CHANGE:` → major.
- Для `0.x` breaking по умолчанию даёт minor
  (`bump_major_on_breaking_below_1_0 = true` включает major).
- `chore(release)`, merge-коммиты и заголовки с `[skip release]` исключаются.
- Pre-release: `1.2.0-alpha.1` → `alpha.2` при повторе, смена канала → `.1`.
- Идемпотентность: если тег уже существует, `release` не двигает версию и
  changelog; `--github-release` при этом дозаполняет отсутствующий релиз
  (перед созданием проверяется `gh release view` / `GET /releases/tags/<tag>`).

## Конфиг `.releaserc.toml`

Внешний шаблон (`template`) получает `{version}`, `{date}`, `{repo_url}` и
`{sections}`; без `{sections}` секции дописываются в конец.

```toml
branches = ["dev", "main", "master"]
tag_format = "v{version}"
changelog = "CHANGELOG.md"
initial_version = "0.1.0"
mode = "fixed"          # или "independent"
locale = "en"           # или "ru"
# template = "release/changelog.hbs"  # внешний шаблон (резерв)
# bump_major_on_breaking_below_1_0 = true
# semver_checks = true

# [[packages]]           # для монорепо
# name = "demo"
# manifest = "crates/demo/Cargo.toml"

# [commit_types]         # переопределение типов
# feat = { section = "Added", bump = "minor" }
```

Если `.releaserc.toml` нет, читается subset `.releaserc.json`
(`branches`, `tagFormat`), остальное — умолчания.

## Монорепо

- `mode = "fixed"` — одна версия (`workspace.package.version`), один тег.
- `mode = "independent"` — у каждого пакета своя текущая версия; bump общий,
  path-зависимости с `version` обновляются. Фильтрация коммитов по путям
  пакета — следующий этап (ADR-0040, «Заделы»).

## Ограничения

- `Cargo.lock` обновляется `cargo update --workspace` после записи версий и
  входит в release-коммит; это держит checkout чистым для следующего запуска.
- `--package` и `--publish` вызывают `cargo package`/`cargo publish` для каждого
  манифеста в порядке path-зависимостей; виртуальный workspace-манифест нужно
  заменить списком `[[packages]]`.
- `--publish --dry-run` выполняет `cargo publish --dry-run` без записи версии.
- `--github-release` использует `gh`, а без него — GitHub API через `curl` при
  наличии `GITHUB_TOKEN`/`GH_TOKEN`.
- `--semver-checks` требует `cargo-semver-checks`; без флага инструмент не
  вызывается и релиз не ломается.

## CI

- PR: `check --commits "origin/<base>..HEAD" --strict` — сообщения вне
  Conventional Commits красят PR.
- Push в `dev` или `workflow_dispatch`: после зелёных test/build job запускает
  `release --ci --push --github-release`; отдельный job `sync-master` делает
  fast-forward-мерж `dev` → `master` и выполняется в том числе на release-коммите
  (`chore(release): vX.Y.Z` исключён только из версионирования).
- Отмена прогонов (`cancel-in-progress`) включена только для PR: иначе push
  тега из release-job отменяет собственный прогон.
- Красный test/build/release на `dev` создаёт Issue (или комментарий к
  существующему Issue того же коммита).
- Push в защищённый `master` идёт через `RELEASE_TOKEN` (PAT администратора)
  с fallback на `GITHUB_TOKEN`; в protection выключен
  «Do not allow bypassing the above settings».

## Проверки

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```
