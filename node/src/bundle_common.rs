use std::sync::{Arc, RwLock};

use lightningcss::{
  bundler::{Bundler, SourceProvider},
  printer::PrinterOptions,
  stylesheet::{MinifyOptions, ParserFlags, ParserOptions, StyleSheet},
  targets::{Features, Targets},
};
use napi::Either;
use parcel_sourcemap::SourceMap;

use crate::{
  at_rule_parser::{AtRule, CustomAtRuleParser},
  compile_error::CompileError,
  custom_at_rules::CustomAtRules,
  transform::{
    convert_dependencies, convert_exports, convert_references, Browsers, CSSModulesConfig, DependencyOptions,
    Drafts, NonStandard, PseudoClasses, TransformResult,
  },
};

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
