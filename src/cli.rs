use std::path::Path;

use codespan_reporting::{diagnostic::Diagnostic, term};
use ecow::eco_format;
use typst::syntax::{package::PackageSpec, FileId, Source};

use crate::{check::all_checks, world::SystemWorld};

pub fn main(package_spec: String) {
    let package_spec: Option<PackageSpec> = package_spec.parse().ok();
    let package_dir = if let Some(ref package_spec) = package_spec {
        Path::new(".")
            .join(package_spec.namespace.to_string())
            .join(package_spec.name.to_string())
            .join(package_spec.version.to_string())
    } else {
        Path::new(".").to_owned()
    };

    dbg!(&package_dir);

    let (world, diags) = all_checks(package_spec.as_ref(), package_dir);
    print_diagnostics(&world, &diags.errors, &diags.warnings)
        .map_err(|err| eco_format!("failed to print diagnostics ({err})"))
        .unwrap();
}

/// Print diagnostic messages to the terminal.
pub fn print_diagnostics(
    world: &SystemWorld,
    errors: &[Diagnostic<FileId>],
    warnings: &[Diagnostic<FileId>],
) -> Result<(), codespan_reporting::files::Error> {
    let config = term::Config {
        tab_width: 2,
        ..Default::default()
    };

    for diagnostic in warnings.iter().chain(errors) {
        term::emit(
            &mut term::termcolor::StandardStream::stdout(term::termcolor::ColorChoice::Always),
            &config,
            world,
            diagnostic,
        )?;
    }

    Ok(())
}

type CodespanResult<T> = Result<T, CodespanError>;
type CodespanError = codespan_reporting::files::Error;

impl<'a> codespan_reporting::files::Files<'a> for SystemWorld {
    type FileId = FileId;
    type Name = String;
    type Source = Source;

    fn name(&'a self, id: FileId) -> CodespanResult<Self::Name> {
        let vpath = id.vpath();
        Ok(if let Some(package) = id.package() {
            format!("{package}{}", vpath.as_rooted_path().display())
        } else {
            // Try to express the path relative to the working directory.
            vpath
                .resolve(self.root())
                .and_then(|abs| pathdiff::diff_paths(abs, self.workdir()))
                .as_deref()
                .unwrap_or_else(|| vpath.as_rootless_path())
                .to_string_lossy()
                .into()
        })
    }

    fn source(&'a self, id: FileId) -> CodespanResult<Self::Source> {
        match self.lookup(id) {
            Ok(x) => Ok(x),
            // Hack to be able to report errors on files that are not UTF-8. The
            // error range should always be 0..0 for this to work.
            Err(typst::diag::FileError::InvalidUtf8) => Ok(Source::new(id, String::new())),
            Err(e) => Err(CodespanError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                e,
            ))),
        }
    }

    fn line_index(&'a self, id: FileId, given: usize) -> CodespanResult<usize> {
        let source = self.source(id)?;
        source
            .byte_to_line(given)
            .ok_or_else(|| CodespanError::IndexTooLarge {
                given,
                max: source.len_bytes(),
            })
    }

    fn line_range(&'a self, id: FileId, given: usize) -> CodespanResult<std::ops::Range<usize>> {
        let source = self.source(id)?;
        source
            .line_to_range(given)
            .ok_or_else(|| CodespanError::LineTooLarge {
                given,
                max: source.len_lines(),
            })
    }

    fn column_number(&'a self, id: FileId, _: usize, given: usize) -> CodespanResult<usize> {
        let source = self.source(id)?;
        source.byte_to_column(given).ok_or_else(|| {
            let max = source.len_bytes();
            if given <= max {
                CodespanError::InvalidCharBoundary { given }
            } else {
                CodespanError::IndexTooLarge { given, max }
            }
        })
    }
}
