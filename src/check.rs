use std::path::PathBuf;

use codespan_reporting::diagnostic::Label;
use typst::{
    syntax::{package::PackageSpec, FileId, Span},
    WorldExt,
};

use crate::world::SystemWorld;

mod authors;
mod compile;
mod diagnostics;
mod file_size;
mod imports;
mod kebab_case;
mod manifest;

pub use diagnostics::Diagnostics;

pub async fn all_checks(
    package_spec: Option<&PackageSpec>,
    package_dir: PathBuf,
) -> (SystemWorld, Diagnostics) {
    let mut diags = Diagnostics::default();

    let worlds = manifest::check(&package_dir, &mut diags, package_spec).await;
    compile::check(&mut diags, &worlds.package);
    if let Some(template_world) = worlds.template {
        let mut template_diags = Diagnostics::default();
        compile::check(&mut template_diags, &template_world);
        let template_dir = template_world
            .root()
            .strip_prefix(worlds.package.root())
            .expect("Template should be in a subfolder of the package");
        diags.extend(template_diags, template_dir);
    }
    kebab_case::check(&mut diags, &worlds.package);
    imports::check(&mut diags, package_spec, &package_dir, &worlds.package);
    if let Some(spec) = package_spec {
        authors::check(&mut diags, spec);
    }

    (worlds.package, diags)
}

/// Create a label for a span.
fn label(world: &SystemWorld, span: Span) -> Option<Label<FileId>> {
    Some(Label::primary(span.id()?, world.range(span)?))
}
