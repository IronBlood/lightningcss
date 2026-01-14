use std::{cell::UnsafeCell, path::PathBuf, str::FromStr};

use lightningcss::{bundler::SourceProvider, stylesheet::StyleSheet, visitor::Visit};
use napi::{
  bindgen_prelude::{FnArgs, FromNapiValue, Function, FunctionRef},
  Either, Env, JsValue, Unknown,
};
use napi_derive::napi;

use crate::{
  at_rule_parser::AtRule,
  bundle_common::{compile_bundle, BundleConfig},
  compile_error::CompileErrorOwned,
  custom_at_rules::CustomAtRules,
  transform::{
    Browsers, CSSModulesConfig, DependencyOptions, Drafts, NonStandard, PseudoClasses, TransformResult, Visitor,
  },
  transformer::get_visitor,
};

#[no_mangle]
pub extern "C" fn napi_wasm_malloc(size: usize) -> *mut u8 {
  use std::alloc::{alloc, Layout};
  use std::mem;

  let align = mem::align_of::<usize>();
  if let Ok(layout) = Layout::from_size_align(size, align) {
    unsafe {
      if layout.size() > 0 {
        let ptr = alloc(layout);
        if !ptr.is_null() {
          return ptr;
        }
      } else {
        return align as *mut u8;
      }
    }
  }

  std::process::abort();
}

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
  pub resolver: Resolver,
}

#[napi(object)]
pub struct Resolver {
  /** Read the given file and return its contents as a string. */
  #[napi(ts_type = "(file: string) => string | Promise<string>")]
  pub read: Function<'static, String, Unknown<'static>>,
  /** Read the given file and return its contents as a string. */
  #[napi(ts_type = "(specifier: string, originatingFile: string) => string | Promise<string> ")]
  pub resolve: Option<Function<'static, FnArgs<(String, String)>, Unknown<'static>>>,
}

// This relies on Binaryen's Asyncify transform to allow Rust to call async JS functions from sync code.
// See the comments in async.mjs for more details about how this works.
extern "C" {
  fn await_promise_sync(
    promise: napi::sys::napi_value,
    result: *mut napi::sys::napi_value,
    error: *mut napi::sys::napi_value,
  );
}

struct JsSourceProvider {
  env: Env,
  resolve: Option<FunctionRef<FnArgs<(String, String)>, Unknown<'static>>>,
  read: FunctionRef<String, Unknown<'static>>,
  inputs: UnsafeCell<Vec<*mut String>>,
}

unsafe impl Sync for JsSourceProvider {}
unsafe impl Send for JsSourceProvider {}

fn get_result(env: &Env, mut value: Unknown<'_>) -> napi::Result<String> {
  if value.is_promise()? {
    let mut result = std::ptr::null_mut();
    let mut error = std::ptr::null_mut();
    unsafe { await_promise_sync(value.raw(), &mut result, &mut error) };
    if !error.is_null() {
      let error = unsafe { Unknown::from_raw_unchecked(env.raw(), error) };
      return Err(napi::Error::from(error));
    }
    if result.is_null() {
      return Err(napi::Error::new(napi::Status::GenericFailure, "No result".to_string()));
    }

    value = unsafe { Unknown::from_raw_unchecked(env.raw(), result) };
  }

  String::from_unknown(value)
}

impl SourceProvider for JsSourceProvider {
  type Error = napi::Error;

  fn read<'a>(&'a self, file: &std::path::Path) -> Result<&'a str, Self::Error> {
    let read = self.read.borrow_back(&self.env)?;
    let source = read.call(file.to_str().unwrap().to_owned())?;
    let source = get_result(&self.env, source)?;

    // cache the result
    let ptr = Box::into_raw(Box::new(source));
    let inputs = unsafe { &mut *self.inputs.get() };
    inputs.push(ptr);
    // SAFETY: this is safe because the pointer is not dropped
    // until the JsSourceProvider is, and we never remove from the
    // list of pointers stored in the vector.
    Ok(unsafe { &*ptr })
  }

  fn resolve(
    &self,
    specifier: &str,
    originating_file: &std::path::Path,
  ) -> Result<std::path::PathBuf, Self::Error> {
    if let Some(resolve) = &self.resolve {
      let resolve = resolve.borrow_back(&self.env)?;
      let specifier = specifier.to_string();
      let originating_file = originating_file.to_str().unwrap().to_owned();
      let result = resolve.call((specifier, originating_file).into())?;
      let result = get_result(&self.env, result)?;
      Ok(PathBuf::from_str(result.as_str()).unwrap())
    } else {
      Ok(originating_file.with_file_name(specifier))
    }
  }
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
    resolver,
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
    env: env.clone(),
    read: resolver.read.create_ref()?,
    resolve: match resolver.resolve {
      Some(r) => Some(r.create_ref()?),
      None => None,
    },
    inputs: UnsafeCell::new(Vec::new()),
  };

  // This is pretty silly, but works around a rust limitation that you cannot
  // explicitly annotate lifetime bounds on closures.
  fn annotate<'i, 'o, F>(f: F) -> F
  where
    F: FnOnce(&mut StyleSheet<'i, 'o, AtRule<'i>>) -> napi::Result<()>,
  {
    f
  }

  let res = compile_bundle(
    &provider,
    &config,
    visitor.as_mut().map(|visitor| annotate(|stylesheet| stylesheet.visit(visitor))),
  );

  match res {
    Ok(res) => Ok(res),
    Err(err) => {
      let owned: CompileErrorOwned = err.into();
      Err(owned.into_js_error(env, None)?)
    }
  }
}
