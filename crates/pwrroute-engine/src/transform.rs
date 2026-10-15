//! TypeScript and modern JavaScript to something QuickJS runs, with oxc.

use std::path::Path;

use anyhow::{Result, anyhow, bail};
use oxc::{
    allocator::Allocator,
    codegen::Codegen,
    parser::Parser,
    semantic::SemanticBuilder,
    span::SourceType,
    transformer::{TransformOptions, Transformer},
};

/// The output of [`transform`].
pub(crate) struct Transformed {
    pub code: String,
    /// Whether the source uses `import` or `export`. A config without either
    /// is a legacy CommonJS (`module.exports = …`) config.
    pub is_module: bool,
}

/// Strips types and lowers syntax QuickJS lacks. The source type comes from
/// the extension, so `.ts` gets TypeScript parsing and `.js` does not.
pub(crate) fn transform(path: &Path, source: &str) -> Result<Transformed> {
    let allocator = Allocator::default();
    let source_type =
        SourceType::from_path(path).map_err(|e| anyhow!("{}: {e}", path.display()))?;
    let parsed = Parser::new(&allocator, source, source_type).parse();
    if let Some(first) = parsed.diagnostics.errors().next() {
        bail!("{}: {first}", path.display());
    }
    let is_module = parsed.module_record.has_module_syntax;
    let mut program = parsed.program;

    let scoping = SemanticBuilder::new()
        // The enum transform reads member values computed here; without it,
        // string enums get wrong reverse mappings (oxc#21667).
        .with_enum_eval(true)
        .build(&program)
        .semantic
        .into_scoping();
    let transformed = Transformer::new(&allocator, path, &TransformOptions::default())
        .build_with_scoping(scoping, &mut program);
    if let Some(first) = transformed.diagnostics.errors().next() {
        bail!("{}: {first}", path.display());
    }
    Ok(Transformed {
        code: Codegen::new().build(&program).code,
        is_module,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_typescript() {
        let out = transform(
            Path::new("config.ts"),
            "enum P { Work = 'w' }\nconst x: number = 1;\nexport default { p: P.Work, x } satisfies object;",
        )
        .unwrap();
        assert!(out.is_module);
        assert!(!out.code.contains(": number"));
        assert!(!out.code.contains("satisfies"));
    }

    #[test]
    fn detects_commonjs() {
        let out = transform(Path::new("config.js"), "module.exports = {};").unwrap();
        assert!(!out.is_module);
    }

    #[test]
    fn reports_syntax_errors_with_the_path() {
        let err = transform(Path::new("broken.ts"), "export default {")
            .err()
            .unwrap()
            .to_string();
        assert!(err.starts_with("broken.ts:"), "{err}");
    }
}
