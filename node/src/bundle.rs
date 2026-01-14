use std::sync::Mutex;

use lightningcss::{stylesheet::StyleSheet, visitor::Visit};
use napi::{
  bindgen_prelude::{FnArgs, Function},
  Either, Env,
};
use napi_derive::napi;

use crate::{
  at_rule_parser::AtRule,
  bundle_common::{compile_bundle, BundleConfig},
  compile_error::CompileErrorOwned,
  custom_at_rules::CustomAtRules,
  js_source_provider::JsSourceProvider,
  transform::{
    Browsers, CSSModulesConfig, DependencyOptions, Drafts, NonStandard, PseudoClasses, TransformResult, Visitor,
  },
  transformer::get_visitor,
};

#[napi(object)]
pub struct BundleOptions {
  /** The filename being transformed. Used for error messages and source maps. */
  pub filename: String,
  /** Whether to enable minification. */
  pub minify: Option<bool>,
  /** Whether to output a source map. */
  pub source_map: Option<bool>,
  /** An input source map to extend. */
  pub input_source_map: Option<String>,
  /**
   * An optional project root path, used as the source root in the output source map.
   * Also used to generate relative paths for sources used in CSS module hashes.
   */
  pub project_root: Option<String>,
  /** The browser targets for the generated code. */
  pub targets: Option<Browsers>,
  /** Features that should always be compiled, even when supported by targets. */
  pub include: Option<u32>, // TODO u32 from Config.include serde(default)
  /** Features that should never be compiled, even when unsupported by targets. */
  pub exclude: Option<u32>, // TODO u32 from Config.exclude serde(default)
  /** Whether to enable parsing various draft syntax. */
  pub drafts: Option<Drafts>,
  /** Whether to enable various non-standard syntax. */
  pub non_standard: Option<NonStandard>,
  /** Whether to compile this file as a CSS module. */
  pub css_modules: Option<Either<bool, CSSModulesConfig>>,
  /**
   * Whether to analyze dependencies (e.g. `@import` and `url()`).
   * When enabled, `@import` rules are removed, and `url()` dependencies
   * are replaced with hashed placeholders that can be replaced with the final
   * urls later (after bundling). Dependencies are returned as part of the result.
   */
  pub analyze_dependencies: Option<Either<bool, DependencyOptions>>,
  /**
   * Replaces user action pseudo classes with class names that can be applied from JavaScript.
   * This is useful for polyfills, for example.
   */
  pub pseudo_classes: Option<PseudoClasses>,
  /**
   * A list of class names, ids, and custom identifiers (e.g. @keyframes) that are known
   * to be unused. These will be removed during minification. Note that these are not
   * selectors but individual names (without any . or # prefixes).
   */
  pub unused_symbols: Option<Vec<String>>,
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
  pub visitor: Option<Visitor>, // TODO generic?
  /**
   * Defines how to parse custom CSS at-rules. Each at-rule can have a prelude, defined using a CSS
   * [syntax string](https://drafts.css-houdini.org/css-properties-values-api/#syntax-strings), and
   * a block body. The body can be a declaration list, rule list, or style block as defined in the
   * [css spec](https://drafts.csswg.org/css-syntax/#declaration-rule-list).
   */
  pub custom_at_rules: Option<CustomAtRules>, // TODO generic?
}

#[napi(object)]
pub struct Resolver {
  /** Read the given file and return its contents as a string. */
  #[napi(ts_type = "(file: string) => string")]
  pub read: Option<Function<'static, String, String>>,
  /** Read the given file and return its contents as a string. */
  #[napi(ts_type = "(specifier: string, originatingFile: string) => string ")]
  pub resolve: Option<Function<'static, FnArgs<(String, String)>, String>>,
}

#[napi]
pub fn bundle(env: Env, options: BundleOptions) -> napi::bindgen_prelude::Result<TransformResult> {
  let BundleOptions {
    filename,
    minify,
    source_map,
    input_source_map,
    project_root,
    targets,
    include,
    exclude,
    drafts,
    non_standard,
    css_modules,
    analyze_dependencies,
    pseudo_classes,
    unused_symbols,
    error_recovery,
    visitor,
    custom_at_rules,
  } = options;

  let mut visitor = get_visitor(env, &visitor)?;

  let config = BundleConfig {
    filename,
    minify,
    source_map,
    input_source_map,
    project_root,
    targets,
    include,
    exclude,
    drafts,
    non_standard,
    css_modules,
    analyze_dependencies,
    pseudo_classes,
    unused_symbols,
    error_recovery,
    custom_at_rules,
  };

  let provider = JsSourceProvider {
    resolve: None,
    read: None,
    inputs: Mutex::new(Vec::new()),
  };

  // This is pretty silly, but works around a rust limitation that you cannot
  // explicitly annotate lifetime bounds on closures.
  fn annotate<'i, 'o, F>(f: F) -> F
  where
    F: FnOnce(&mut StyleSheet<'i, 'o, AtRule<'i>>) -> napi::Result<()>,
  {
    f
  }

  let result = compile_bundle(
    &provider,
    &config,
    visitor.as_mut().map(|visitor| annotate(|stylesheet| stylesheet.visit(visitor))),
  );
  match result {
    Ok(v) => Ok(v),
    Err(err) => {
      let owned: CompileErrorOwned = err.into();
      Err(owned.into_js_error(env, None)?)
    }
  }
}
