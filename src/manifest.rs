//! Чтение и запись версии в `Cargo.toml` без потери форматирования.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use semver::Version;
use toml_edit::{value, DocumentMut, Item, TableLike};

use crate::error::{ReleaseError, Result};

fn read_document(path: &Path) -> Result<(String, DocumentMut)> {
    let content = std::fs::read_to_string(path).map_err(|error| ReleaseError::Manifest {
        path: path.to_path_buf(),
        message: error.to_string(),
    })?;
    let document = content
        .parse::<DocumentMut>()
        .map_err(|error| ReleaseError::Manifest {
            path: path.to_path_buf(),
            message: format!("parse: {error}"),
        })?;
    Ok((content, document))
}

/// Читает версию: `[workspace.package].version`, иначе `[package].version`.
///
/// # Errors
///
/// [`ReleaseError::Manifest`] — файл не читается/не парсится или поле
/// `version` отсутствует/невалидно.
pub fn read_version(path: &Path) -> Result<Version> {
    let (_, document) = read_document(path)?;
    let raw = document
        .get("workspace")
        .and_then(|workspace| workspace.get("package"))
        .and_then(|package| package.get("version"))
        .and_then(Item::as_str)
        .or_else(|| {
            document
                .get("package")
                .and_then(|package| package.get("version"))
                .and_then(Item::as_str)
        });
    let Some(raw) = raw else {
        return Err(ReleaseError::Manifest {
            path: path.to_path_buf(),
            message: "version field not found".into(),
        });
    };
    Version::parse(raw).map_err(|error| ReleaseError::Manifest {
        path: path.to_path_buf(),
        message: format!("version: {error}"),
    })
}

/// Имя пакета (`[package].name`).
///
/// # Errors
///
/// [`ReleaseError::Manifest`] — файл не читается/не парсится.
pub fn read_package_name(path: &Path) -> Result<Option<String>> {
    let (_, document) = read_document(path)?;
    Ok(document
        .get("package")
        .and_then(|package| package.get("name"))
        .and_then(Item::as_str)
        .map(str::to_string))
}

fn set_version(document: &mut DocumentMut, version: &Version) -> bool {
    if let Some(package) = document
        .get_mut("workspace")
        .and_then(|workspace| workspace.get_mut("package"))
        .and_then(Item::as_table_like_mut)
    {
        package.insert("version", value(version.to_string()));
        return true;
    }
    if let Some(package) = document
        .get_mut("package")
        .and_then(Item::as_table_like_mut)
    {
        package.insert("version", value(version.to_string()));
        return true;
    }
    false
}

/// Записывает версию, сохраняя форматирование.
///
/// # Errors
///
/// [`ReleaseError::Manifest`] — нет поля версии или файл не пишется.
pub fn write_version(path: &Path, version: &Version) -> Result<()> {
    let (_, mut document) = read_document(path)?;
    if !set_version(&mut document, version) {
        return Err(ReleaseError::Manifest {
            path: path.to_path_buf(),
            message: "version field not found".into(),
        });
    }
    std::fs::write(path, document.to_string()).map_err(|error| ReleaseError::Manifest {
        path: path.to_path_buf(),
        message: error.to_string(),
    })
}

fn table_mut<'a>(document: &'a mut DocumentMut, path: &[&str]) -> Option<&'a mut dyn TableLike> {
    let (first, rest) = path.split_first()?;
    let mut item = document.get_mut(first)?;
    for key in rest {
        item = item.get_mut(key)?;
    }
    item.as_table_like_mut()
}

fn update_table(
    document: &mut DocumentMut,
    path: &[&str],
    versions: &BTreeMap<String, Version>,
) -> bool {
    let Some(table) = table_mut(document, path) else {
        return false;
    };
    let mut changed = false;
    for (name, item) in table.iter_mut() {
        let name = name.get();
        let Some(dependency) = item.as_table_like_mut() else {
            continue;
        };
        if dependency.get("path").is_none() {
            continue;
        }
        let Some(version) = versions.get(name) else {
            continue;
        };
        dependency.insert("version", value(version.to_string()));
        changed = true;
    }
    changed
}

/// Обновляет `version` у path-зависимостей с указанными именами.
///
/// # Errors
///
/// [`ReleaseError::Manifest`] — файл не читается/не пишется.
pub fn update_path_dependencies(path: &Path, versions: &BTreeMap<String, Version>) -> Result<bool> {
    let (_, mut document) = read_document(path)?;
    let mut changed = false;
    for table in [
        ["dependencies"].as_slice(),
        ["dev-dependencies"].as_slice(),
        ["build-dependencies"].as_slice(),
        ["workspace", "dependencies"].as_slice(),
    ] {
        changed |= update_table(&mut document, table, versions);
    }
    if changed {
        std::fs::write(path, document.to_string()).map_err(|error| ReleaseError::Manifest {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    }
    Ok(changed)
}

fn dependency_tables(document: &DocumentMut) -> Vec<&dyn TableLike> {
    let mut tables: Vec<&dyn TableLike> = Vec::new();
    for path in [
        ["dependencies"].as_slice(),
        ["dev-dependencies"].as_slice(),
        ["build-dependencies"].as_slice(),
        ["workspace", "dependencies"].as_slice(),
    ] {
        let Some((first, rest)) = path.split_first() else {
            continue;
        };
        let mut item = match document.get(first) {
            Some(item) => item,
            None => continue,
        };
        let mut complete = true;
        for key in rest {
            match item.get(key) {
                Some(next) => item = next,
                None => {
                    complete = false;
                    break;
                }
            }
        }
        if complete {
            if let Some(table) = item.as_table_like() {
                tables.push(table);
            }
        }
    }
    tables
}

/// Порядок публикации: path-зависимости раньше зависимых пакетов.
///
/// # Errors
///
/// [`ReleaseError::Manifest`] — манифест не читается/не парсится или граф
/// path-зависимостей содержит цикл.
pub fn publish_order(manifests: &[PathBuf]) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::with_capacity(manifests.len());
    let mut by_canonical = BTreeMap::new();
    for manifest in manifests {
        let canonical = std::fs::canonicalize(manifest).unwrap_or_else(|_| manifest.clone());
        by_canonical.insert(canonical, paths.len());
        paths.push(manifest.clone());
    }

    let mut dependencies: Vec<Vec<usize>> = Vec::with_capacity(paths.len());
    for (position, manifest) in paths.iter().enumerate() {
        let (_, document) = read_document(manifest)?;
        let base = manifest.parent().unwrap_or_else(|| Path::new("."));
        let mut edges = Vec::new();
        for table in dependency_tables(&document) {
            for (_, dependency) in table.iter() {
                let Some(dependency) = dependency.as_table_like() else {
                    continue;
                };
                let Some(relative) = dependency.get("path").and_then(Item::as_str) else {
                    continue;
                };
                let candidate = base.join(relative).join("Cargo.toml");
                let canonical = std::fs::canonicalize(&candidate).unwrap_or(candidate);
                if let Some(&index) = by_canonical.get(&canonical) {
                    if index != position {
                        edges.push(index);
                    }
                }
            }
        }
        edges.sort_unstable();
        edges.dedup();
        dependencies.push(edges);
    }

    let mut emitted = vec![false; paths.len()];
    let mut order = Vec::with_capacity(paths.len());
    while order.len() < paths.len() {
        let mut progressed = false;
        for (index, manifest) in paths.iter().enumerate() {
            if emitted[index] || !dependencies[index].iter().all(|dep| emitted[*dep]) {
                continue;
            }
            emitted[index] = true;
            order.push(manifest.clone());
            progressed = true;
        }
        if !progressed {
            return Err(ReleaseError::Manifest {
                path: paths.first().cloned().unwrap_or_default(),
                message: "path dependency cycle in publish order".into(),
            });
        }
    }
    Ok(order)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn write(path: &Path, content: &str) {
        std::fs::write(path, content).unwrap();
    }

    #[test]
    fn reads_and_writes_workspace_version() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join("Cargo.toml");
        write(
            &manifest,
            "[workspace]\nmembers = []\n\n[workspace.package]\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        );
        assert_eq!(read_version(&manifest).unwrap().to_string(), "0.1.0");
        write_version(&manifest, &Version::parse("1.2.3").unwrap()).unwrap();
        let updated = std::fs::read_to_string(&manifest).unwrap();
        assert!(updated.contains("version = \"1.2.3\""));
        assert!(updated.contains("edition = \"2021\""));
    }

    #[test]
    fn reads_and_writes_package_version() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join("Cargo.toml");
        write(
            &manifest,
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[dependencies]\n",
        );
        assert_eq!(
            read_package_name(&manifest).unwrap().as_deref(),
            Some("demo")
        );
        write_version(&manifest, &Version::parse("0.2.0").unwrap()).unwrap();
        assert_eq!(read_version(&manifest).unwrap().to_string(), "0.2.0");
    }

    #[test]
    fn publish_order_puts_dependencies_first() {
        let dir = tempfile::tempdir().unwrap();
        let core = dir.path().join("core");
        let app = dir.path().join("app");
        std::fs::create_dir_all(&core).unwrap();
        std::fs::create_dir_all(&app).unwrap();
        let core_manifest = core.join("Cargo.toml");
        let app_manifest = app.join("Cargo.toml");
        write(
            &core_manifest,
            "[package]\nname = \"core\"\nversion = \"0.1.0\"\n",
        );
        write(
            &app_manifest,
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\ncore = { path = \"../core\", version = \"0.1.0\" }\n",
        );
        let order = publish_order(&[app_manifest.clone(), core_manifest.clone()]).unwrap();
        assert_eq!(order, vec![core_manifest, app_manifest]);
    }

    #[test]
    fn updates_path_dependencies_only() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join("Cargo.toml");
        write(
            &manifest,
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[dependencies]\nflux-core = { path = \"../flux-core\", version = \"0.1.0\" }\nserde = \"1\"\n",
        );
        let versions =
            BTreeMap::from([("flux-core".to_string(), Version::parse("0.2.0").unwrap())]);
        assert!(update_path_dependencies(&manifest, &versions).unwrap());
        let updated = std::fs::read_to_string(&manifest).unwrap();
        assert!(updated.contains("version = \"0.2.0\""));
        assert!(updated.contains("serde = \"1\""));
    }

    #[test]
    fn missing_version_is_typed_error() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join("Cargo.toml");
        write(&manifest, "[package]\nname = \"demo\"\n");
        assert!(matches!(
            read_version(&manifest),
            Err(ReleaseError::Manifest { .. })
        ));
    }
}
