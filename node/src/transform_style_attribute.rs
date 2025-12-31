use std::sync::{Arc, RwLock};

use lightningcss::{
  dependencies::DependencyOptions,
  printer::PrinterOptions,
  stylesheet::{MinifyOptions, ParserOptions, StyleAttribute},
  targets::{Features, Targets},
  visitor::Visit,
};
use napi::{bindgen_prelude::Uint8Array, Env};
use napi_derive::napi;

use crate::{
  compile_error::{CompileError, CompileErrorOwned},
  transform::{convert_dependencies, Browsers, Dependency, Visitor, Warning},
  transformer::{get_visitor, JsVisitor},
};

#[napi(object)]
pub struct TransformAttributeOptions {
  /** The filename in which the style attribute appeared. Used for error messages and dependencies. */
  pub filename: Option<String>,
  /** The source code to transform. */
  pub code: Uint8Array,
  /** Whether to enable minification. */
  pub minify: Option<bool>,
  /** The browser targets for the generated code. */
  pub targets: Option<Browsers>,
  /**
   * Whether to analyze `url()` dependencies.
   * When enabled, `url()` dependencies are replaced with hashed placeholders
   * that can be replaced with the final urls later (after bundling).
   * Dependencies are returned as part of the result.
   */
  pub analyze_dependencies: Option<bool>,
  /**
   * Whether to ignore invalid rules and declarations rather than erroring.
   * When enabled, warnings are returned, and the invalid rule or declaration is
   * omitted from the output code.
   */
  pub error_recovery: Option<bool>,
  /**
   * An AST visitor object. This allows custom transforms or analysis to be implemented in JavaScript.
   * Multiple visitors can be composed into one using the `composeVisitors` function.
   * For optimal performance, visitors should be as specific as possible about what types of values
   * they care about so that JavaScript has to be called as little as possible.
   */
  pub visitor: Option<Visitor>,
  /** Features that should always be compiled, even when supported by targets. */
  pub include: Option<u32>, // TODO u32 from Config.include serde(default)
  /** Features that should never be compiled, even when unsupported by targets. */
  pub exclude: Option<u32>, // TODO u32 from Config.exclude serde(default)
}

#[napi(object)]
pub struct TransformAttributeResult {
  /** The transformed code. */
  pub code: Uint8Array,
  /** `@import` and `url()` dependencies, if enabled. */
  pub dependencies: Option<Vec<Dependency>>,
  /** Warnings that occurred during compilation. */
  pub warnings: Vec<Warning>,
}

fn compile_attr<'i>(
  code: &'i str,
  config: &TransformAttributeOptions,
  visitor: &mut Option<JsVisitor>, // TODO feature
) -> Result<TransformAttributeResult, CompileError<'i, napi::Error>> {
  let error_recovery = config.error_recovery.unwrap_or(false);

  let warnings = if error_recovery {
    Some(Arc::new(RwLock::new(Vec::new())))
  } else {
    None
  };

  let res = {
    let filename = config.filename.clone().unwrap_or_default();
    let mut attr = StyleAttribute::parse(
      &code,
      ParserOptions {
        filename,
        error_recovery,
        warnings: warnings.clone(),
        ..ParserOptions::default()
      },
    )?;

    //#[cfg(feature = "visitor")] TODO
    if let Some(visitor) = visitor.as_mut() {
      attr.visit(visitor).unwrap();
    }

    let targets = Targets {
      browsers: config.targets.as_ref().map(Into::into),
      include: Features::from_bits_truncate(config.include.unwrap_or(u32::default())),
      exclude: Features::from_bits_truncate(config.exclude.unwrap_or(u32::default())),
    };

    attr.minify(MinifyOptions {
      targets,
      ..MinifyOptions::default()
    });
    attr.to_css(PrinterOptions {
      minify: config.minify.unwrap_or(false),
      source_map: None,
      project_root: None,
      targets,
      analyze_dependencies: if config.analyze_dependencies.unwrap_or(false) {
        Some(DependencyOptions::default())
      } else {
        None
      },
      pseudo_classes: None,
    })?
  };
  Ok(TransformAttributeResult {
    code: res.code.into(),
    dependencies: convert_dependencies(res.dependencies),
    warnings: warnings.map_or(Vec::new(), |w| {
      Arc::try_unwrap(w)
        .unwrap()
        .into_inner()
        .unwrap()
        .into_iter()
        .map(|w| w.into())
        .collect()
    }),
  })
}

#[napi]
pub fn transform_style_attribute(
  env: Env,
  options: TransformAttributeOptions,
) -> napi::Result<TransformAttributeResult> {
  let mut visitor = get_visitor(env, &options.visitor)?;
  let code =
    std::str::from_utf8(&options.code).map_err(|e| napi::Error::new(napi::Status::InvalidArg, e.to_string()))?;
  let result = compile_attr(code, &options, &mut visitor);
  match result {
    Ok(v) => Ok(v),
    Err(err) => {
      let owned: CompileErrorOwned = err.into();
      Err(owned.into_js_error(env, None)?)
    }
  }
}
