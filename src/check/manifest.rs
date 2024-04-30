use std::{ops::Range, path::Path};

use codespan_reporting::diagnostic::{Diagnostic, Label};
use ecow::eco_format;
use globset::{Glob, GlobSet};
use tracing::debug;
use typst::syntax::{
    package::{PackageSpec, PackageVersion},
    FileId, VirtualPath,
};

use crate::{check::file_size, world::SystemWorld};

use super::Diagnostics;

pub struct Worlds {
    pub package: SystemWorld,
    pub template: Option<SystemWorld>,
}

pub fn check(
    package_dir: &Path,
    diags: &mut Diagnostics,
    package_spec: Option<&PackageSpec>,
) -> Worlds {
    let manifest_path = package_dir.join("typst.toml");
    debug!("Reading manifest at {}", &manifest_path.display());
    let manifest_contents = std::fs::read_to_string(manifest_path).unwrap();
    let manifest = toml_edit::ImDocument::parse(&manifest_contents).unwrap();

    let entrypoint = package_dir.join(manifest["package"]["entrypoint"].as_str().unwrap());
    let mut world = SystemWorld::new(entrypoint, package_dir.to_owned())
        .map_err(|err| eco_format!("{err}"))
        .unwrap();

    let manifest_file_id = FileId::new(None, VirtualPath::new("typst.toml"));

    if !manifest.contains_table("package") {
        diags.errors.push(
            Diagnostic::error()
                .with_labels(vec![Label::primary(manifest_file_id, 0..0)])
                .with_message(
                    "All `typst.toml` must contain a [package] section. See the README.md file of this repository for details about the manifest format."
                ),
        );
        return Worlds {
            package: world,
            template: None,
        };
    }

    let name = check_name(diags, manifest_file_id, &manifest, package_spec);
    let version = check_version(diags, manifest_file_id, &manifest, package_spec);
    exclude_large_files(diags, package_dir, &manifest);
    check_file_names(diags, package_dir);
    dont_over_exclude(diags, manifest_file_id, &manifest);

    let (exclude, _) = read_exclude(&manifest);
    world.exclude(exclude);

    let template_world = if let (Some(name), Some(version)) = (name, version) {
        let inferred_package_spec = PackageSpec {
            namespace: "preview".into(),
            name: name.into(),
            version,
        };
        world_for_template(
            &manifest,
            package_dir,
            package_spec.unwrap_or(&inferred_package_spec),
        )
    } else {
        None
    };

    Worlds {
        package: world,
        template: template_world,
    }
}

fn check_name(
    diags: &mut Diagnostics,
    manifest_file_id: FileId,
    manifest: &toml_edit::ImDocument<&String>,
    package_spec: Option<&PackageSpec>,
) -> Option<String> {
    let Some(name) = manifest["package"].get("name") else {
        diags.errors.push(
            Diagnostic::error()
                .with_labels(vec![Label::primary(manifest_file_id, 0..0)])
                .with_message(
                    "All `typst.toml` must contain a `name` field. See the README.md file of this repository for details about the manifest format."
                ),
        );
        return None;
    };

    let error = Diagnostic::error().with_labels(vec![Label::primary(
        manifest_file_id,
        name.span().unwrap_or_default(),
    )]);
    let warning = Diagnostic::warning().with_labels(vec![Label::primary(
        manifest_file_id,
        name.span().unwrap_or_default(),
    )]);

    let Some(name) = name.as_str() else {
        diags
            .errors
            .push(error.with_message("`name` must be a string."));
        return None;
    };

    if name != casbab::kebab(name) {
        diags.errors.push(
            error
                .clone()
                .with_message("Please use kebab-case for package names."),
        )
    }

    if name.contains("typst") {
        diags
            .warnings
            .push(warning.with_message("Package names should generally not include \"typst\"."));
    }

    if let Some(package_spec) = package_spec {
        if name != package_spec.name {
            diags.errors.push(
                error
                    .with_message(format!(
                        "Unexpected package name. `{name}` was expected. If you want to publish a new package, create a new directory in `packages/{namespace}/`.",
                        name = package_spec.name,
                        namespace = package_spec.namespace,
                    )),
            )
        }
    }

    Some(name.to_owned())
}

fn check_version(
    diags: &mut Diagnostics,
    manifest_file_id: FileId,
    manifest: &toml_edit::ImDocument<&String>,
    package_spec: Option<&PackageSpec>,
) -> Option<PackageVersion> {
    let Some(version) = manifest["package"].get("version") else {
        diags.errors.push(
            Diagnostic::error()
                .with_labels(vec![Label::primary(manifest_file_id, 0..0)])
                .with_message(
                    "All `typst.toml` must contain a `version` field. See the README.md file of this repository for details about the manifest format."
                ),
        );
        return None;
    };

    let error = Diagnostic::error().with_labels(vec![Label::primary(
        manifest_file_id,
        version.span().unwrap_or_default(),
    )]);

    let Some(version) = version.as_str() else {
        diags
            .errors
            .push(error.with_message("`version` must be a string."));
        return None;
    };

    let Ok(version) = version.parse::<PackageVersion>() else {
        diags
            .errors
            .push(error.with_message("`version` must be a valid semantic version (i.e follow the `MAJOR.MINOR.PATCH` format)."));
        return None;
    };

    if let Some(package_spec) = package_spec {
        if version != package_spec.version {
            diags.errors.push(
            error
                .with_message(format!(
                    "Unexpected version number. `{version}` was expected. If you want to publish a new version, create a new directory in `packages/{namespace}/{name}`.",
                    version = package_spec.version,
                    name = package_spec.name,
                    namespace = package_spec.namespace,
                )),
        )
        }
    }

    return Some(version);
}

fn exclude_large_files(
    diags: &mut Diagnostics,
    package_dir: &Path,
    manifest: &toml_edit::ImDocument<&String>,
) {
    let (exclude, _) = read_exclude(manifest);

    const REALLY_LARGE: u64 = 50 * 1024 * 1024;

    let large_files = file_size::find_large_files(package_dir);
    for (path, size) in large_files {
        let fid = FileId::new(None, VirtualPath::new(&path));

        let message = if size > REALLY_LARGE {
            format!(
                "This file is really large ({size}MB). If possible, do not include it in this repository at all.",
                size = size / 1024 / 1024
            )
        } else if !exclude.is_match(path) {
            format!(
                "This file is quite large ({size}MB). If it is not required to use the package (i.e. it is a documentation file, or part of an example), it should be added to `exclude` in your `typst.toml`.",
                size = size / 1024 / 1024
            )
        } else {
            continue;
        };

        diags.warnings.push(
            Diagnostic::warning()
                .with_labels(vec![Label::primary(fid, 0..0)])
                .with_message(message),
        )
    }

    // Also exclude examples
    for ch in walkdir::WalkDir::new(package_dir) {
        let Ok(ch) = ch else {
            continue;
        };

        let file_name = ch.file_name();
        if exclude.is_match(&file_name) {
            continue;
        }

        let file_name_str = file_name.to_string_lossy();
        let file_id = FileId::new(
            None,
            VirtualPath::new(&ch.path().strip_prefix(package_dir).unwrap()),
        );
        let warning = Diagnostic::warning().with_labels(vec![Label::primary(file_id, 0..0)]);
        if file_name_str.contains("example") {
            diags.warnings.push(
                warning.clone()
                    .with_message("This file seems to be an example, and should probably be added to `exclude` in your `typst.toml`.")
            );
        }

        if file_name_str.contains("test") {
            diags.warnings.push(
                warning.clone()
                    .with_message("This file seems to be a test, and should probably be added to `exclude` in your `typst.toml`.")
            );
        }

        if Path::new(&file_name).extension().and_then(|e| e.to_str()) == Some("pdf") {
            diags.warnings.push(
                warning
                    .with_message("This file seems to be for documentation or generated by Typst, and should probably be added to `exclude` in your `typst.toml`.")
            );
        }
    }
}

fn dont_over_exclude(
    diags: &mut Diagnostics,
    manifest_file_id: FileId,
    manifest: &toml_edit::ImDocument<&String>,
) {
    let (exclude, span) = read_exclude(manifest);

    let warning = Diagnostic::warning().with_labels(vec![Label::primary(manifest_file_id, span)]);

    if exclude.is_match("LICENSE") {
        diags.warnings.push(
            warning
                .clone()
                .with_message("Your LICENSE file should not be excluded."),
        );
    }

    if exclude.is_match("README.md") {
        diags
            .warnings
            .push(warning.with_message("Your README.md file should not be excluded."));
    }
}

fn check_file_names(diags: &mut Diagnostics, package_dir: &Path) {
    for ch in std::fs::read_dir(package_dir).unwrap() {
        let mut error_for_file = |path, message| {
            let file_id = FileId::new(None, VirtualPath::new(path));
            diags.errors.push(
                Diagnostic::error()
                    .with_labels(vec![Label::primary(file_id, 0..0)])
                    .with_message(message),
            )
        };

        let Ok(ch) = ch else {
            continue;
        };
        let Ok(meta) = ch.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }

        let file_name = ch.file_name();
        let file_path = Path::new(&file_name);
        let stem = file_path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned());
        let stem_uppercase = stem.as_ref().map(|s| s.to_uppercase());

        if stem_uppercase.as_deref() == Some("LICENCE") {
            error_for_file(file_path, "This file should be named LICENSE.");
        }

        if (stem_uppercase.as_deref() == Some("LICENSE")
            || stem_uppercase.as_deref() == Some("README"))
            && stem_uppercase != stem
        {
            let fixed = if let Some(ext) = file_path.extension() {
                format!("{}.{}", stem.unwrap().to_uppercase(), ext.to_string_lossy())
            } else {
                stem.unwrap().to_uppercase()
            };
            error_for_file(file_path, &format!("To keep consistency, please use ALL CAPS for the name of this file (i.e. {fixed})"))
        }
    }
}

fn read_exclude(manifest: &toml_edit::ImDocument<&String>) -> (GlobSet, Range<usize>) {
    let empty_array = toml_edit::Array::new();
    let exclude = manifest["package"]
        .get("exclude")
        .and_then(|item| item.as_array())
        .unwrap_or(&empty_array);

    let mut exclude_globs = GlobSet::builder();
    for glob in exclude {
        exclude_globs.add(Glob::new(glob.as_str().unwrap()).unwrap());
    }
    (exclude_globs.build().unwrap(), exclude.span().unwrap())
}

fn world_for_template(
    manifest: &toml_edit::ImDocument<&String>,
    package_dir: &Path,
    package_spec: &PackageSpec,
) -> Option<SystemWorld> {
    let template = manifest.get("template")?.as_table()?;
    let template_path = package_dir.join(template.get("path")?.as_str()?);
    let template_main = template_path.join(template.get("entrypoint")?.as_str()?);
    Some(
        SystemWorld::new(template_main, template_path)
            .ok()?
            .with_package_override(package_spec, package_dir),
    )
}
