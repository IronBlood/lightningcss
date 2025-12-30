use std::sync::{Arc, Mutex, RwLock};

use lightningcss::{
  bundler::{Bundler, SourceProvider},
  printer::PrinterOptions,
  stylesheet::{MinifyOptions, ParserFlags, ParserOptions, StyleSheet},
  targets::{Features, Targets},
  visitor::Visit,
};
use napi::{
  bindgen_prelude::{FnArgs, Function},
  Either, Env,
};
use napi_derive::napi;
use parcel_sourcemap::SourceMap;

use crate::{
  at_rule_parser::{AtRule, CustomAtRuleParser},
  compile_error::{CompileError, CompileErrorOwned},
  custom_at_rules::CustomAtRules,
  js_source_provider::JsSourceProvider,
  transform::{
    convert_dependencies, convert_exports, convert_references, Browsers, CSSModulesConfig, DependencyOptions,
    Drafts, NonStandard, PseudoClasses, TransformResult, Visitor,
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

// BundleOptions without visitors
pub struct BundleConfig {
  pub filename: String,
  pub minify: Option<bool>,
  pub source_map: Option<bool>,
  pub input_source_map: Option<String>,
  pub project_root: Option<String>,
  pub targets: Option<Browsers>,
  pub include: Option<u32>,
  pub exclude: Option<u32>,
  pub drafts: Option<Drafts>,
  pub non_standard: Option<NonStandard>,
  pub css_modules: Option<Either<bool, CSSModulesConfig>>,
  pub analyze_dependencies: Option<Either<bool, DependencyOptions>>,
  pub pseudo_classes: Option<PseudoClasses>,
  pub unused_symbols: Option<Vec<String>>,
  pub error_recovery: Option<bool>,
  pub custom_at_rules: Option<CustomAtRules>,
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

pub fn compile_bundle<
  'i,
  'o,
  P: SourceProvider,
  F: FnOnce(&mut StyleSheet<'i, 'o, AtRule<'i>>) -> napi::Result<()>,
>(
  fs: &'i P,
  config: &'o BundleConfig,
  visit: Option<F>,
) -> Result<TransformResult, CompileError<'i, P::Error>> {
  use std::path::Path;

  let project_root = config.project_root.as_ref().map(|p| p.as_ref());
  let mut source_map = if config.source_map.unwrap_or_default() {
    Some(SourceMap::new(project_root.unwrap_or("/")))
  } else {
    None
  };
  let warnings = Some(Arc::new(RwLock::new(Vec::new())));

  let res = {
    let drafts = config.drafts.as_ref();
    let non_standard = config.non_standard.as_ref();
    let mut flags = ParserFlags::empty();
    flags.set(
      ParserFlags::CUSTOM_MEDIA,
      matches!(drafts, Some(d) if d.custom_media.unwrap_or(false)),
    );
    flags.set(
      ParserFlags::DEEP_SELECTOR_COMBINATOR,
      matches!(non_standard, Some(v) if v.deep_selector_combinator.unwrap_or(false)),
    );

    let parser_options = ParserOptions {
      flags,
      css_modules: if let Some(css_modules) = &config.css_modules {
        match css_modules {
          Either::A(true) => Some(lightningcss::css_modules::Config::default()),
          Either::A(false) => None,
          Either::B(c) => Some(lightningcss::css_modules::Config {
            pattern: if let Some(pattern) = c.pattern.as_ref() {
              match lightningcss::css_modules::Pattern::parse(pattern) {
                Ok(p) => p,
                Err(e) => return Err(CompileError::PatternError(e)),
              }
            } else {
              Default::default()
            },
            dashed_idents: c.dashed_idents.unwrap_or_default(),
            animation: c.animation.unwrap_or(true),
            container: c.container.unwrap_or(true),
            grid: c.grid.unwrap_or(true),
            custom_idents: c.custom_idents.unwrap_or(true),
            pure: c.pure.unwrap_or_default(),
          }),
        }
      } else {
        None
      },
      error_recovery: config.error_recovery.unwrap_or_default(),
      warnings: warnings.clone(),
      filename: String::new(),
      source_index: 0,
    };

    let mut at_rule_parser = CustomAtRuleParser {
      configs: config
        .custom_at_rules
        .clone()
        .unwrap_or_default()
        .into_iter()
        .map(|(k, v)| Ok((k, v.try_into()?)))
        .collect::<napi::Result<_>>()?,
    };

    let mut bundler =
      Bundler::new_with_at_rule_parser(fs, source_map.as_mut(), parser_options, &mut at_rule_parser);
    let mut stylesheet = bundler.bundle(Path::new(&config.filename))?;

    if let Some(visit) = visit {
      visit(&mut stylesheet).map_err(CompileError::JsError)?;
    }

    let targets = Targets {
      browsers: config.targets.as_ref().map(Into::into),
      include: Features::from_bits_truncate(config.include.unwrap_or(u32::default())),
      exclude: Features::from_bits_truncate(config.exclude.unwrap_or(u32::default())),
    };

    stylesheet.minify(MinifyOptions {
      targets,
      unused_symbols: config.unused_symbols.clone().unwrap_or_default().into_iter().collect(),
    })?;

    stylesheet.to_css(PrinterOptions {
      minify: config.minify.unwrap_or_default(),
      source_map: source_map.as_mut(),
      project_root,
      targets,
      analyze_dependencies: if let Some(d) = &config.analyze_dependencies {
        match d {
          Either::A(b) if *b => Some(lightningcss::dependencies::DependencyOptions { remove_imports: true }),
          Either::B(c) => Some(lightningcss::dependencies::DependencyOptions {
            remove_imports: !c.preserve_imports.unwrap_or(false),
          }),
          _ => None,
        }
      } else {
        None
      },
      pseudo_classes: config.pseudo_classes.as_ref().map(|p| p.into()),
    })?
  };

  let map = if let Some(source_map) = &mut source_map {
    source_map.to_json(None).ok()
  } else {
    None
  };

  Ok(TransformResult {
    code: res.code.into(),
    map: map.map(|m| m.into()),
    exports: convert_exports(res.exports),
    references: convert_references(res.references),
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
