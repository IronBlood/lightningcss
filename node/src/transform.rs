use crate::at_rule_parser::CustomAtRuleParser;
use crate::compile_error::CompileErrorOwned;
use crate::css_module_reference::convert_css_module_ref;
use crate::transformer::{get_visitor, JsVisitor, JsVisitorCallback, JsVisitorCallbackMap};
use lightningcss::stylesheet::{MinifyOptions, ParserFlags, ParserOptions, PrinterOptions, StyleSheet};
use lightningcss::targets::{
  // Browsers,
  Features,
  Targets,
};
use napi::bindgen_prelude::{Either, Function, Uint8Array};
use napi::{Env, Unknown};
use parcel_sourcemap::SourceMap;
use std::sync::{Arc, RwLock};

use napi_derive::napi;
use std::collections::HashMap;

use crate::{
  compile_error::CompileError,
  css_module_reference::{CSSModuleReference, DependencyCSSModuleReference},
  custom_at_rules::CustomAtRules,
};

// TODO Duplicate from `lightningcss::targets::Browsers`
#[napi(object)]
#[derive(Clone)]
pub struct Browsers {
  pub android: Option<u32>,
  pub chrome: Option<u32>,
  pub edge: Option<u32>,
  pub firefox: Option<u32>,
  pub ie: Option<u32>,
  pub ios_saf: Option<u32>,
  pub opera: Option<u32>,
  pub safari: Option<u32>,
  pub samsung: Option<u32>,
}

impl From<&Browsers> for lightningcss::targets::Browsers {
  fn from(b: &Browsers) -> Self {
    Self {
      android: b.android,
      chrome: b.chrome,
      edge: b.edge,
      firefox: b.firefox,
      ie: b.ie,
      ios_saf: b.ios_saf,
      opera: b.opera,
      safari: b.safari,
      samsung: b.samsung,
    }
  }
}

#[napi(object)]
pub struct Drafts {
  /** Whether to enable @custom-media rules. */
  pub custom_media: Option<bool>,
}

#[napi(object)]
pub struct NonStandard {
  pub deep_selector_combinator: Option<bool>,
}

#[napi(object)]
pub struct PseudoClasses {
  pub hover: Option<String>,
  pub active: Option<String>,
  pub focus: Option<String>,
  pub focus_visible: Option<String>,
  pub focus_within: Option<String>,
}

// NOTE copied and updated the struct names
impl<'a> Into<lightningcss::stylesheet::PseudoClasses<'a>> for &'a PseudoClasses {
  fn into(self) -> lightningcss::stylesheet::PseudoClasses<'a> {
    lightningcss::stylesheet::PseudoClasses {
      hover: self.hover.as_deref(),
      active: self.active.as_deref(),
      focus: self.focus.as_deref(),
      focus_visible: self.focus_visible.as_deref(),
      focus_within: self.focus_within.as_deref(),
    }
  }
}

#[napi(object)]
// See `CssModulesConfig` in `napi/src/lib.rs`
pub struct CSSModulesConfig {
  /** The pattern to use when renaming class names and other identifiers. Default is `[hash]_[local]`. */
  pub pattern: Option<String>,
  /** Whether to rename dashed identifiers, e.g. custom properties. */
  pub dashed_idents: Option<bool>,
  /** Whether to enable hashing for `@keyframes`. */
  pub animation: Option<bool>,
  /** Whether to enable hashing for CSS grid identifiers. */
  pub grid: Option<bool>,
  /** Whether to enable hashing for `@container` names. */
  pub container: Option<bool>,
  /** Whether to enable hashing for custom identifiers. */
  pub custom_idents: Option<bool>,
  /** Whether to require at least one class or id selector in each rule. */
  pub pure: Option<bool>,
}

#[napi(object)]
pub struct DependencyOptions {
  /** Whether to preserve `@import` rules rather than removing them. */
  pub preserve_imports: Option<bool>,
}

// TODO update ts_type
// TODO use `JsCb` (maybe rename)
#[napi(object)]
pub struct Visitor {
  #[napi(
    js_name = "StyleSheet",
    ts_type = "((stylesheet: StyleSheet) => StyleSheet<ReturnedDeclaration, ReturnedMediaQuery> | void) | void"
  )]
  pub stylesheet: Option<JsVisitorCallback>,
  #[napi(
    js_name = "StyleSheetExit",
    ts_type = "((stylesheet: StyleSheet) => StyleSheet<ReturnedDeclaration, ReturnedMediaQuery> | void) | void"
  )]
  pub stylesheet_exit: Option<JsVisitorCallback>,
  #[napi(
    js_name = "Rule",
    ts_type = "((rule: RequiredValue<Rule>) => ReturnedRule | ReturnedRule[] | void) | void"
  )]
  pub rule: Option<Either<JsVisitorCallback, HashMap<String, JsVisitorCallback>>>,
  #[napi(
    js_name = "RuleExit",
    ts_type = "((rule: RequiredValue<Rule>) => ReturnedRule | ReturnedRule[] | void) | void"
  )]
  pub rule_exit: Option<Either<JsVisitorCallback, HashMap<String, JsVisitorCallback>>>,
  #[napi(
    js_name = "Declaration",
    ts_type = "((property: Declaration) => ReturnedDeclaration | ReturnedDeclaration[] | void) | void"
  )]
  pub declaration: Option<Either<JsVisitorCallback, HashMap<String, JsVisitorCallback>>>,
  #[napi(
    js_name = "DeclarationExit",
    ts_type = "((property: Declaration) => ReturnedDeclaration | ReturnedDeclaration[] | void) | void"
  )]
  pub declaration_exit: Option<Either<JsVisitorCallback, HashMap<String, JsVisitorCallback>>>,
  #[napi(js_name = "Url", ts_type = "((url: Url) => Url | void) | void")]
  pub url: Option<Function<'static, Unknown<'static>, Option<Unknown<'static>>>>,
  #[napi(js_name = "Color", ts_type = "((color: CssColor) => CssColor | void) | void")]
  pub color: Option<Function<'static, Unknown<'static>, Option<Unknown<'static>>>>,
  #[napi(js_name = "Image", ts_type = "((image: Image) => Image | void) | void")]
  pub image: Option<Function<'static, Unknown<'static>, Option<Unknown<'static>>>>,
  #[napi(js_name = "ImageExit", ts_type = "((image: Image) => Image | void) | void")]
  pub image_exit: Option<Function<'static, Unknown<'static>, Option<Unknown<'static>>>>,
  #[napi(js_name = "Length", ts_type = "((length: LengthValue) => LengthValue | void) | void")]
  pub length: Option<Function<'static, Unknown<'static>, Option<Unknown<'static>>>>,
  #[napi(js_name = "Angle", ts_type = "((angle: Angle) => Angle | void) | void")]
  pub angle: Option<Function<'static, Unknown<'static>, Option<Unknown<'static>>>>,
  #[napi(js_name = "Ratio", ts_type = "((ratio: Ratio) => Ratio | void) | void")]
  pub ratio: Option<Function<'static, Unknown<'static>, Option<Unknown<'static>>>>,
  #[napi(
    js_name = "Resolution",
    ts_type = "((resolution: Resolution) => Resolution | void) | void"
  )]
  pub resolution: Option<Function<'static, Unknown<'static>, Option<Unknown<'static>>>>,
  #[napi(js_name = "Time", ts_type = "((time: Time) => Time | void) | void")]
  pub time: Option<Function<'static, Unknown<'static>, Option<Unknown<'static>>>>,
  #[napi(js_name = "CustomIdent", ts_type = "((ident: string) => string | void) | void")]
  pub custom_ident: Option<Function<'static, Unknown<'static>, Option<Unknown<'static>>>>,
  #[napi(js_name = "DashedIdent", ts_type = "((ident: string) => string | void) | void")]
  pub dashed_ident: Option<Function<'static, Unknown<'static>, Option<Unknown<'static>>>>,
  #[napi(
    js_name = "MediaQuery",
    ts_type = "((query: MediaQuery) => ReturnedMediaQuery | ReturnedMediaQuery[] | void) | void"
  )]
  pub media_query: Option<Function<'static, Unknown<'static>, Option<Unknown<'static>>>>,
  #[napi(
    js_name = "MediaQueryExit",
    ts_type = "((query: MediaQuery) => ReturnedMediaQuery | ReturnedMediaQuery[] | void) | void"
  )]
  pub media_query_exit: Option<Function<'static, Unknown<'static>, Option<Unknown<'static>>>>,
  #[napi(
    js_name = "SupportsCondition",
    ts_type = "((condition: SupportsCondition) => SupportsCondition) | void"
  )]
  pub supports_condition: Option<Function<'static, Unknown<'static>, Option<Unknown<'static>>>>,
  #[napi(
    js_name = "SupportsCondition",
    ts_type = "((condition: SupportsCondition) => SupportsCondition) | void"
  )]
  pub supports_condition_exit: Option<Function<'static, Unknown<'static>, Option<Unknown<'static>>>>,
  #[napi(
    js_name = "Selector",
    ts_type = "((selector: Selector) => Selector | Selector[] | void) | void"
  )]
  pub selector: Option<Function<'static, Unknown<'static>, Option<Unknown<'static>>>>,
  #[napi(
    js_name = "Token",
    ts_type = "(token: Token) => TokenReturnValue | Record<VisitableTokenTypes, (token: FindByType<Token, Name>) => TokenReturnValue> | void"
  )]
  pub token: Option<Either<JsVisitorCallback, HashMap<String, JsVisitorCallback>>>,
  #[napi(
    js_name = "Function",
    ts_type = "FunctionVisitor | Record<string, FunctionVisitor> | void "
  )]
  pub function: Option<Either<JsVisitorCallback, HashMap<String, JsVisitorCallback>>>,
  #[napi(
    js_name = "FunctionExit",
    ts_type = "FunctionVisitor | Record<string, FunctionVisitor> | void "
  )]
  pub function_exit: Option<Either<JsVisitorCallback, HashMap<String, JsVisitorCallback>>>,
  #[napi(js_name = "Variable", ts_type = "((variable: Variable) => TokenReturnValue) | void")]
  pub variable: Option<Function<'static, Unknown<'static>, Option<Unknown<'static>>>>,
  #[napi(
    js_name = "VariableExit",
    ts_type = "((variable: Variable) => TokenReturnValue) | void"
  )]
  pub variable_exit: Option<Function<'static, Unknown<'static>, Option<Unknown<'static>>>>,
  #[napi(
    js_name = "EnvironmentVariable",
    ts_type = "EnvironmentVariableVisitor | Record<string, EnvironmentVariableVisitor> | void"
  )]
  pub environment_variable: Option<Either<JsVisitorCallback, JsVisitorCallbackMap>>,
  #[napi(
    js_name = "EnvironmentVariable",
    ts_type = "EnvironmentVariableVisitor | Record<string, EnvironmentVariableVisitor> | void"
  )]
  pub environment_variable_exit: Option<Either<JsVisitorCallback, JsVisitorCallbackMap>>,
}

// NOTE see `Config` from `napi/src/lib.rs`
#[napi(object)]
pub struct TransformOptions {
  /** The filename being transformed. Used for error messages and source maps. */
  pub filename: String,
  /** The source code to transform. */
  pub code: Uint8Array,
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
pub struct CSSModuleExport {
  /** The local (compiled) name for this export. */
  pub name: String,
  /** Whether the export is referenced in this file. */
  pub is_referenced: bool,
  /** Other names that are composed by this export. */
  pub composes: Vec<CSSModuleReference>,
}

#[napi(object)]
pub struct ErrorLocation {
  // TODO extends
  pub filename: String,
  /** The line number (1-based). */
  pub line: u32, // TODO number NOTE in error.rs 0 based
  /** The column number (0-based). */
  pub column: u32, // TODO number NOTE in error.rs 1 based
}

#[napi(object)]
// NOTE aligned to `Warning` in napi/src/lib.rs not in index.d.ts
pub struct Warning {
  pub message: String,
  #[napi(js_name = "type")]
  pub _type: String,
  // pub value: Option<Object<'i>>, // TODO any
  pub data: String, // TODO
  //pub data: lightningcss::error::ParserError<'i>,
  pub loc: Option<ErrorLocation>, // TODO maybe optional
}

impl<'i> From<lightningcss::error::Error<lightningcss::error::ParserError<'i>>> for Warning {
  fn from(mut e: lightningcss::error::Error<lightningcss::error::ParserError<'i>>) -> Self {
    // Convert to 1-based line numbers.
    if let Some(loc) = &mut e.loc {
      loc.line += 1;
    }
    Warning {
      _type: "TODO".into(), // TODO
      message: e.kind.to_string(),
      data: "".into(), // TODO e.kind,
      loc: convert_error_location(e.loc),
    }
  }
}

// NOTE see `CssModuleExports` in napi/src/lib.rs
#[napi]
pub type CSSModuleExports = HashMap<String, CSSModuleExport>;
// NOTE see `CssModuleReferences` in napi/src/lib.rs
#[napi]
pub type CSSModuleReferences = HashMap<String, DependencyCSSModuleReference>;

#[napi(object)]
pub struct Location {
  /** The line number (1-based). */
  pub line: u32, // TODO number?
  /** The column number (0-based). */
  pub column: u32, // TODO number?
}

#[napi(object)]
pub struct SourceLocation {
  /** The file path in which the dependency exists. */
  pub file_path: String,
  /** The start location of the dependency. */
  pub start: Location,
  /** The end location (inclusive) of the dependency. */
  pub end: Location,
}

#[napi(object)]
pub struct ImportDependency {
  pub _type: String,
  /** The url of the `@import` dependency. */
  pub url: String,
  /** The media query for the `@import` rule. */
  pub media: Option<String>,
  /** The `supports()` query for the `@import` rule. */
  pub supports: Option<String>,
  /** The source location where the `@import` rule was found. */
  pub loc: SourceLocation,
  /** The placeholder that the import was replaced with. */
  pub placeholder: String,
}

#[napi(object)]
pub struct UrlDependency {
  pub _type: String,
  /** The url of the dependency. */
  pub url: String,
  /** The source location where the `url()` was found. */
  pub loc: SourceLocation,
  /** The placeholder that the url was replaced with. */
  pub placeholder: String,
}

#[napi]
pub type Dependency = Either<ImportDependency, UrlDependency>;

#[napi(object)]
pub struct TransformResult {
  /** The transformed code. */
  pub code: Uint8Array,
  /** The generated source map, if enabled. */
  pub map: Option<Uint8Array>,
  /** CSS module exports, if enabled. */
  pub exports: Option<CSSModuleExports>,
  /** CSS module references, if `dashedIdents` is enabled. */
  pub references: Option<CSSModuleReferences>,
  /** `@import` and `url()` dependencies, if enabled. */
  pub dependencies: Option<Vec<Dependency>>,
  /** Warnings that occurred during compilation. */
  pub warnings: Vec<Warning>,
}

fn compile<'i>(
  code: &'i str,
  config: &TransformOptions,
  visitor: &mut Option<JsVisitor>, // TODO feature gated
) -> Result<TransformResult, CompileError<'i, napi::Error>> {
  let drafts = config.drafts.as_ref();
  let non_standard = config.non_standard.as_ref();
  let warnings = Some(Arc::new(RwLock::new(Vec::new())));

  let filename = config.filename.clone(); //.unwrap_or_default();
  let project_root = config.project_root.as_ref().map(|p| p.as_ref());
  let mut source_map = if config.source_map.unwrap_or_default() {
    let mut sm = SourceMap::new(project_root.unwrap_or("/"));
    sm.add_source(&filename);
    sm.set_source_content(0, code)?;
    Some(sm)
  } else {
    None
  };

  let res = {
    let mut flags = ParserFlags::empty();
    flags.set(
      ParserFlags::CUSTOM_MEDIA,
      matches!(drafts, Some(d) if d.custom_media.unwrap_or(false)), // NOTE unwrap_or, because
                                                                    // `.custom_media` is defined as `Option<bool>` based on the definition from TypeScript
    );
    flags.set(
      ParserFlags::DEEP_SELECTOR_COMBINATOR,
      matches!(non_standard, Some(v) if v.deep_selector_combinator.unwrap_or(false)), // NOTE same unwrap
    );

    let mut stylesheet = StyleSheet::parse_with(
      &code,
      ParserOptions {
        filename: filename.clone(),
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
        source_index: 0,
        error_recovery: config.error_recovery.unwrap_or_default(),
        warnings: warnings.clone(),
      },
      &mut CustomAtRuleParser {
        configs: config
          .custom_at_rules
          .clone()
          .unwrap_or_default()
          .into_iter()
          .map(|(k, v)| Ok((k, v.try_into()?)))
          .collect::<napi::Result<_>>()?,
      },
    )?;

    // TODO
    // #[cfg(feature = "visitor")]
    if let Some(visitor) = visitor.as_mut() {
      use lightningcss::visitor::Visit;

      stylesheet.visit(visitor).map_err(CompileError::JsError)?;
    }

    let targets = Targets {
      browsers: config.targets.as_ref().map(Into::into),
      include: Features::from_bits_truncate(config.include.unwrap_or(u32::default())),
      exclude: Features::from_bits_truncate(config.exclude.unwrap_or(u32::default())),
    };

    stylesheet.minify(MinifyOptions {
      targets,
      // NOTE from `Option<Vec<String>>`
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

  let map = if let Some(mut source_map) = source_map {
    if let Some(input_source_map) = &config.input_source_map {
      if let Ok(mut sm) = SourceMap::from_json("/", input_source_map) {
        let _ = source_map.extends(&mut sm);
      }
    }

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
pub fn transform(env: Env, options: TransformOptions) -> napi::bindgen_prelude::Result<TransformResult> {
  let mut visitor = get_visitor(env, &options.visitor)?;
  let code =
    std::str::from_utf8(&options.code).map_err(|e| napi::Error::new(napi::Status::InvalidArg, e.to_string()))?;
  let result = compile(code, &options, &mut visitor);
  match result {
    Ok(v) => Ok(v),
    Err(err) => {
      let owned: CompileErrorOwned = err.into();
      Err(owned.into_js_error(env, None)?)
    }
  }
}

pub fn convert_exports(exports: Option<lightningcss::css_modules::CssModuleExports>) -> Option<CSSModuleExports> {
  let exports = exports?;

  let out: CSSModuleExports = exports.into_iter().map(|(k, v)| (k, v.into())).collect();

  Some(out)
}

impl From<lightningcss::css_modules::CssModuleExport> for CSSModuleExport {
  fn from(v: lightningcss::css_modules::CssModuleExport) -> Self {
    Self {
      name: v.name,
      is_referenced: v.is_referenced,
      composes: v.composes.into_iter().map(convert_css_module_ref).collect(),
    }
  }
}

pub fn convert_references(
  references: Option<lightningcss::css_modules::CssModuleReferences>,
) -> Option<CSSModuleReferences> {
  references.map(|m| {
    m.into_iter()
      .map(|(k, v)| {
        let _type: String = "dependency".into();
        let r = match v {
          lightningcss::css_modules::CssModuleReference::Local { name } => DependencyCSSModuleReference {
            _type,
            name,
            // TODO I'm not sure FIXME
            specifier: "".into(),
          },
          lightningcss::css_modules::CssModuleReference::Global { name } => DependencyCSSModuleReference {
            _type,
            name,
            // TODO I'm not sure FIXME
            specifier: "".into(),
          },
          lightningcss::css_modules::CssModuleReference::Dependency { name, specifier } => {
            DependencyCSSModuleReference { _type, name, specifier }
          }
        };
        (k, r)
      })
      .collect()
  })
}

pub fn convert_dependencies(
  dependencies: Option<Vec<lightningcss::dependencies::Dependency>>,
) -> Option<Vec<Either<ImportDependency, UrlDependency>>> {
  let dependencies = dependencies?;
  Some(
    dependencies
      .into_iter()
      .map(|dep| match dep {
        lightningcss::dependencies::Dependency::Import(d) => Either::A(ImportDependency {
          _type: "import".into(),
          url: d.url,
          media: d.media,
          supports: d.supports,
          loc: convert_source_location(d.loc),
          placeholder: d.placeholder,
        }),
        lightningcss::dependencies::Dependency::Url(d) => Either::B(UrlDependency {
          _type: "url".into(),
          url: d.url,
          loc: convert_source_location(d.loc),
          placeholder: d.placeholder,
        }),
      })
      .collect(),
  )
}

fn convert_source_location(loc: lightningcss::dependencies::SourceRange) -> SourceLocation {
  SourceLocation {
    file_path: loc.file_path,
    // TODO 0 or 1 based
    start: Location {
      line: loc.start.line,
      column: loc.start.column,
    },
    // TODO 0 or 1 based
    end: Location {
      line: loc.end.line,
      column: loc.end.column,
    },
  }
}

fn convert_error_location(loc: Option<lightningcss::error::ErrorLocation>) -> Option<ErrorLocation> {
  match loc {
    Some(loc) => Some(ErrorLocation {
      filename: loc.filename,
      line: loc.line,
      column: loc.column,
    }),
    None => None,
  }
}
