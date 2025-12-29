use std::{
  collections::HashMap,
  marker::PhantomData,
  ops::{Index, IndexMut},
};

use lightningcss::traits::IntoOwned;
use lightningcss::{
  media_query::MediaFeatureValue,
  properties::{
    custom::{Token, TokenList, TokenOrValue},
    Property,
  },
  rules::{CssRule, CssRuleList},
  stylesheet::ParserOptions,
  traits::ParseWithOptions,
  values::{ident::Ident, length::Length, string::CowArcStr},
  visitor::{Visit, VisitTypes, Visitor},
};
use napi::{
  bindgen_prelude::{Function, FunctionRef},
  Either, Env, Unknown,
};
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

use crate::{at_rule_parser::AtRule, transform::Visitor as VisitorOpt};

pub fn get_visitor(env: Env, opts: &Option<VisitorOpt>) -> napi::Result<Option<JsVisitor>> {
  let res = match opts {
    Some(visitor) => Some(JsVisitor::new(env, visitor)?),
    None => None,
  };
  Ok(res)
}

pub type JsVisitorCallback = Function<'static, Unknown<'static>, Option<Unknown<'static>>>;
pub type JsVisitorCallbackMap = HashMap<String, JsVisitorCallback>;
type JsVisitorCallbackRef = FunctionRef<Unknown<'static>, Option<Unknown<'static>>>;

pub struct JsVisitor {
  env: Env,
  visit_stylesheet: VisitorsRef,
  visit_rule: Visitors<JsVisitorCallbackRef>,
  rule_map: Visitors<HashMap<String, JsVisitorCallbackRef>>,
  property_map: Visitors<HashMap<String, JsVisitorCallbackRef>>,
  visit_declaration: Visitors<JsVisitorCallbackRef>,
  visit_length: Option<JsVisitorCallbackRef>,
  visit_angle: Option<JsVisitorCallbackRef>,
  visit_ratio: Option<JsVisitorCallbackRef>,
  visit_resolution: Option<JsVisitorCallbackRef>,
  visit_time: Option<JsVisitorCallbackRef>,
  visit_color: Option<JsVisitorCallbackRef>,
  visit_image: VisitorsRef,
  visit_url: Option<JsVisitorCallbackRef>,
  visit_media_query: VisitorsRef,
  visit_supports_condition: VisitorsRef,
  visit_custom_ident: Option<JsVisitorCallbackRef>,
  visit_dashed_ident: Option<JsVisitorCallbackRef>,
  visit_selector: Option<JsVisitorCallbackRef>,
  visit_token: Visitors<JsVisitorCallbackRef>,
  token_map: Visitors<HashMap<String, JsVisitorCallbackRef>>,
  visit_function: Visitors<JsVisitorCallbackRef>,
  function_map: Visitors<HashMap<String, JsVisitorCallbackRef>>,
  visit_variable: VisitorsRef,
  visit_env: Visitors<JsVisitorCallbackRef>,
  env_map: Visitors<HashMap<String, JsVisitorCallbackRef>>,
  types: VisitTypes,
}

// This is so that the visitor can work with bundleAsync.
// We ensure that we only call JsVisitor from the main JS thread.
unsafe impl Send for JsVisitor {}

#[derive(PartialEq, Eq, Clone, Copy)]
enum VisitStage {
  Enter,
  Exit,
}

type VisitorsRef = Visitors<JsVisitorCallbackRef>;

struct Visitors<T> {
  enter: Option<T>,
  exit: Option<T>,
}

impl<T> Visitors<T> {
  fn new(enter: Option<T>, exit: Option<T>) -> Self {
    Self { enter, exit }
  }

  fn for_stage(&self, stage: VisitStage) -> Option<&T> {
    match stage {
      VisitStage::Enter => self.enter.as_ref(),
      VisitStage::Exit => self.exit.as_ref(),
    }
  }
}

// NOTE Visitors<Ref<()>>
impl Visitors<JsVisitorCallbackRef> {
  pub fn get<'env>(
    &self,
    env: &'env Env,
  ) -> napi::Result<Visitors<Function<'env, Unknown<'env>, Option<Unknown<'env>>>>> {
    Ok(Visitors {
      enter: match &self.enter {
        Some(r) => Some(r.borrow_back(env)?),
        None => None,
      },
      exit: match &self.exit {
        Some(r) => Some(r.borrow_back(env)?),
        None => None,
      },
    })
  }
}

// NOTE Visitors<JsObject>
impl Visitors<HashMap<String, JsVisitorCallbackRef>> {
  fn named(&self, stage: VisitStage, name: &str) -> Option<&JsVisitorCallbackRef> {
    self.for_stage(stage)?.get(name)
  }

  // FIXME this `custom` is only applied to `rule_map`, the values binded to
  // `"unknown"` and `"custom"` might be either functions or map-liked objects,
  // but at this phase, they're treated as functions just for development.
  fn custom(&self, stage: VisitStage, obj: &str, _name: &str) -> Option<&JsVisitorCallbackRef> {
    self.for_stage(stage)?.get(obj)
  }
}

// TODO rename
fn cb_ref(
  types: &mut VisitTypes,
  flags: VisitTypes,
  f: &Option<JsVisitorCallback>,
) -> napi::Result<Option<JsVisitorCallbackRef>> {
  match f {
    Some(f) => {
      *types |= flags;
      Ok(Some(f.create_ref()?))
    }
    None => Ok(None),
  }
}

// the get! macro in the original JsVisitor::new()
fn cb_or_map_ref(
  types: &mut VisitTypes,
  flags: VisitTypes,
  cb: &Option<Either<JsVisitorCallback, HashMap<String, JsVisitorCallback>>>,
) -> napi::Result<(
  Option<JsVisitorCallbackRef>,
  Option<HashMap<String, JsVisitorCallbackRef>>,
)> {
  match cb {
    Some(Either::A(f)) => {
      *types |= flags;
      Ok((Some(f.create_ref()?), None))
    }
    Some(Either::B(map)) => {
      *types |= flags;
      let mut out = HashMap::with_capacity(map.len());
      for (k, f) in map {
        out.insert(k.clone(), f.create_ref()?);
      }
      Ok((None, Some(out)))
    }
    None => Ok((None, None)),
  }
}

fn cb_pair(
  types: &mut VisitTypes,
  flags: VisitTypes,
  enter: &Option<JsVisitorCallback>,
  exit: &Option<JsVisitorCallback>,
) -> napi::Result<VisitorsRef> {
  Ok(VisitorsRef {
    enter: cb_ref(types, flags, enter)?,
    exit: cb_ref(types, flags, exit)?,
  })
}

impl JsVisitor {
  // NOTE macros are replaced with functions, for better LSP supportd
  fn new(env: Env, visitor: &VisitorOpt) -> napi::Result<Self> {
    let mut types = VisitTypes::empty();

    let (visit_rule, rule_map) = cb_or_map_ref(&mut types, VisitTypes::RULES, &visitor.rule)?;
    let (visit_rule_exit, rule_map_exit) = cb_or_map_ref(&mut types, VisitTypes::RULES, &visitor.rule_exit)?;

    let (visit_declaration, property_map) =
      cb_or_map_ref(&mut types, VisitTypes::PROPERTIES, &visitor.declaration)?;
    let (visit_declaration_exit, property_map_exit) =
      cb_or_map_ref(&mut types, VisitTypes::PROPERTIES, &visitor.declaration_exit)?;

    let (visit_token, token_map) = cb_or_map_ref(&mut types, VisitTypes::TOKENS, &visitor.token)?;

    let (visit_function, function_map) = cb_or_map_ref(&mut types, VisitTypes::TOKENS, &visitor.function)?;
    let (visit_function_exit, function_map_exit) =
      cb_or_map_ref(&mut types, VisitTypes::TOKENS, &visitor.function_exit)?;

    let (visit_env, env_map) = cb_or_map_ref(
      &mut types,
      VisitTypes::TOKENS | VisitTypes::MEDIA_QUERIES | VisitTypes::ENVIRONMENT_VARIABLES,
      &visitor.environment_variable,
    )?;
    let (visit_env_exit, env_map_exit) = cb_or_map_ref(
      &mut types,
      VisitTypes::TOKENS | VisitTypes::MEDIA_QUERIES | VisitTypes::ENVIRONMENT_VARIABLES,
      &visitor.environment_variable_exit,
    )?;

    let visit_stylesheet = cb_pair(
      &mut types,
      VisitTypes::RULES,
      &visitor.stylesheet,
      &visitor.stylesheet_exit,
    )?;
    let visit_rule = Visitors::new(visit_rule, visit_rule_exit);
    let rule_map = Visitors::new(rule_map, rule_map_exit);
    let visit_declaration = Visitors::new(visit_declaration, visit_declaration_exit);
    let property_map = Visitors::new(property_map, property_map_exit);
    let visit_length = cb_ref(&mut types, VisitTypes::LENGTHS, &visitor.length)?;
    let visit_angle = cb_ref(&mut types, VisitTypes::ANGLES, &visitor.angle)?;
    let visit_ratio = cb_ref(&mut types, VisitTypes::RATIOS, &visitor.ratio)?;
    let visit_resolution = cb_ref(&mut types, VisitTypes::RESOLUTIONS, &visitor.resolution)?;
    let visit_time = cb_ref(&mut types, VisitTypes::TIMES, &visitor.time)?;
    let visit_color = cb_ref(&mut types, VisitTypes::COLORS, &visitor.color)?;
    let visit_image = cb_pair(&mut types, VisitTypes::IMAGES, &visitor.image, &visitor.image_exit)?;
    let visit_url = cb_ref(&mut types, VisitTypes::URLS, &visitor.url)?;
    let visit_media_query = cb_pair(
      &mut types,
      VisitTypes::MEDIA_QUERIES,
      &visitor.media_query,
      &visitor.media_query_exit,
    )?;
    let visit_supports_condition = cb_pair(
      &mut types,
      VisitTypes::SUPPORTS_CONDITIONS,
      &visitor.supports_condition,
      &visitor.supports_condition_exit,
    )?;
    let visit_variable = cb_pair(
      &mut types,
      VisitTypes::TOKENS,
      &visitor.variable,
      &visitor.variable_exit,
    )?;
    let visit_env = Visitors::new(visit_env, visit_env_exit);
    let env_map = Visitors::new(env_map, env_map_exit);
    let visit_custom_ident = cb_ref(&mut types, VisitTypes::CUSTOM_IDENTS, &visitor.custom_ident)?;
    let visit_dashed_ident = cb_ref(&mut types, VisitTypes::DASHED_IDENTS, &visitor.dashed_ident)?;
    let visit_function = Visitors::new(visit_function, visit_function_exit);
    let function_map = Visitors::new(function_map, function_map_exit);
    let visit_selector = cb_ref(&mut types, VisitTypes::SELECTORS, &visitor.selector)?;
    let visit_token = Visitors::new(visit_token, None);
    let token_map = Visitors::new(token_map, None);

    Ok(Self {
      env,
      visit_stylesheet,
      visit_rule,
      rule_map,
      visit_declaration,
      property_map,
      visit_length,
      visit_angle,
      visit_ratio,
      visit_resolution,
      visit_time,
      visit_color,
      visit_image,
      visit_url,
      visit_media_query,
      visit_supports_condition,
      visit_variable,
      visit_env,
      env_map,
      visit_custom_ident,
      visit_dashed_ident,
      visit_function,
      function_map,
      visit_selector,
      visit_token,
      token_map,
      types,
    })
  }
}

impl<'i> Visitor<'i, AtRule<'i>> for JsVisitor {
  type Error = napi::Error;

  fn visit_types(&self) -> lightningcss::visitor::VisitTypes {
    self.types
  }

  fn visit_stylesheet<'o>(
    &mut self,
    stylesheet: &mut lightningcss::stylesheet::StyleSheet<'i, 'o, AtRule<'i>>,
  ) -> Result<(), Self::Error> {
    if self.types.contains(VisitTypes::RULES) {
      let env = self.env;
      let visit_stylesheet = self.visit_stylesheet.get(&env)?;

      if let Some(visit) = visit_stylesheet.for_stage(VisitStage::Enter) {
        call_visitor(&env, stylesheet, visit)?;
      }

      stylesheet.visit_children(self)?;

      if let Some(visit) = visit_stylesheet.for_stage(VisitStage::Exit) {
        call_visitor(&env, stylesheet, visit)?;
      }

      Ok(())
    } else {
      stylesheet.visit_children(self)
    }
  }

  fn visit_rule_list(
    &mut self,
    rules: &mut lightningcss::rules::CssRuleList<'i, AtRule<'i>>,
  ) -> Result<(), Self::Error> {
    if !self.types.contains(VisitTypes::RULES) {
      return rules.visit_children(self);
    }

    let env = self.env;

    visit_list(
      self,
      rules,
      |this, value, stage| {
        let name = match value {
          CssRule::Media(..) => "media",
          CssRule::Import(..) => "import",
          CssRule::Style(..) => "style",
          CssRule::Keyframes(..) => "keyframes",
          CssRule::FontFace(..) => "font-face",
          CssRule::FontPaletteValues(..) => "font-palette-values",
          CssRule::FontFeatureValues(..) => "font-feature-values",
          CssRule::Page(..) => "page",
          CssRule::Supports(..) => "supports",
          CssRule::CounterStyle(..) => "counter-style",
          CssRule::Namespace(..) => "namespace",
          CssRule::CustomMedia(..) => "custom-media",
          CssRule::LayerBlock(..) => "layer-block",
          CssRule::LayerStatement(..) => "layer-statement",
          CssRule::Property(..) => "property",
          CssRule::Container(..) => "container",
          CssRule::Scope(..) => "scope",
          CssRule::MozDocument(..) => "moz-document",
          CssRule::Nesting(..) => "nesting",
          CssRule::NestedDeclarations(..) => "nested-declarations",
          CssRule::Viewport(..) => "viewport",
          CssRule::StartingStyle(..) => "starting-style",
          CssRule::ViewTransition(..) => "view-transition",
          CssRule::Unknown(v) => {
            let name = v.name.as_ref();
            // FIXME see the method `.custom`
            if let Some(visit) = this.rule_map.custom(stage, "unknown", name) {
              let visit = visit.borrow_back(&env)?;
              let js_value: Unknown = env.to_js_value(v)?;
              let res = visit.call(js_value)?;
              if let Some(res) = res {
                env.from_js_value(res).map(serde_detach::detach)?
              } else {
                "unknown"
              }
            } else {
              "unknown"
            }
          }
          CssRule::Custom(c) => {
            let name = c.name.as_ref();
            // FIXME see the method `.custom`
            if let Some(visit) = this.rule_map.custom(stage, "custom", name) {
              let visit = visit.borrow_back(&env)?;
              let js_value: Unknown = env.to_js_value(c)?;
              let res = visit.call(js_value)?;
              if let Some(res) = res {
                env.from_js_value(res).map(serde_detach::detach)?
              } else {
                "custom"
              }
            } else {
              "custom"
            }
          }
          CssRule::Ignored => return Ok(None),
        };

        if let Some(visit) = this.rule_map.named(stage, name).or(this.visit_rule.for_stage(stage)) {
          let js_value = env.to_js_value(value)?;
          let res = visit.borrow_back(&env)?.call(js_value)?;
          if let Some(res) = res {
            env.from_js_value(res).map(serde_detach::detach)
          } else {
            Ok(None)
          }
        } else {
          Ok(None)
        }
      },
      |this, rule| rule.visit_children(this),
    )?;

    Ok(())
  }

  fn visit_declaration_block(
    &mut self,
    decls: &mut lightningcss::declaration::DeclarationBlock<'i>,
  ) -> Result<(), Self::Error> {
    if self.types.contains(VisitTypes::PROPERTIES) {
      visit_declaration_list(self, &mut decls.important_declarations, |this, property| {
        property.visit_children(this)
      })?;
      visit_declaration_list(self, &mut decls.declarations, |this, property| {
        property.visit_children(this)
      })?;
      Ok(())
    } else {
      decls.visit_children(self)
    }
  }

  fn visit_length(&mut self, length: &mut lightningcss::values::length::LengthValue) -> Result<(), Self::Error> {
    visit(&self.env, length, &self.visit_length)
  }

  fn visit_angle(&mut self, angle: &mut lightningcss::values::angle::Angle) -> Result<(), Self::Error> {
    visit(&self.env, angle, &self.visit_angle)
  }

  fn visit_ratio(&mut self, ratio: &mut lightningcss::values::ratio::Ratio) -> Result<(), Self::Error> {
    visit(&self.env, ratio, &self.visit_ratio)
  }

  fn visit_resolution(
    &mut self,
    resolution: &mut lightningcss::values::resolution::Resolution,
  ) -> Result<(), Self::Error> {
    visit(&self.env, resolution, &self.visit_resolution)
  }

  fn visit_time(&mut self, time: &mut lightningcss::values::time::Time) -> Result<(), Self::Error> {
    visit(&self.env, time, &self.visit_time)
  }

  fn visit_color(&mut self, color: &mut lightningcss::values::color::CssColor) -> Result<(), Self::Error> {
    visit(&self.env, color, &self.visit_color)
  }

  fn visit_image(&mut self, image: &mut lightningcss::values::image::Image<'i>) -> Result<(), Self::Error> {
    visit(&self.env, image, &self.visit_image.enter)?;
    image.visit_children(self)?;
    visit(&self.env, image, &self.visit_image.exit)
  }

  fn visit_url(&mut self, url: &mut lightningcss::values::url::Url<'i>) -> Result<(), Self::Error> {
    visit(&self.env, url, &self.visit_url)
  }

  fn visit_media_list(&mut self, media: &mut lightningcss::media_query::MediaList<'i>) -> Result<(), Self::Error> {
    if self.types.contains(VisitTypes::MEDIA_QUERIES) {
      visit_list(
        self,
        &mut media.media_queries,
        |this, value, stage| {
          if let Some(visit) = this.visit_media_query.for_stage(stage) {
            let js_value = this.env.to_js_value(value)?;
            let visit = visit.borrow_back(&this.env)?;
            let res = visit.call(js_value)?;
            if let Some(res) = res {
              this.env.from_js_value(res).map(serde_detach::detach)
            } else {
              Ok(None)
            }
          } else {
            Ok(None)
          }
        },
        |this, q| q.visit_children(this),
      )?;
      Ok(())
    } else {
      media.visit_children(self)
    }
  }

  fn visit_media_feature_value(
    &mut self,
    value: &mut lightningcss::media_query::MediaFeatureValue<'i>,
  ) -> Result<(), Self::Error> {
    if self.types.contains(VisitTypes::ENVIRONMENT_VARIABLES) && matches!(value, MediaFeatureValue::Env(_)) {
      let call = |stage: VisitStage, value: &mut MediaFeatureValue, this: &JsVisitor| -> napi::Result<()> {
        let env_var = if let MediaFeatureValue::Env(env) = value {
          env
        } else {
          return Ok(());
        };
        let visit_type = this.env_map.named(stage, env_var.name.name());
        let visit = this.visit_env.for_stage(stage);
        let new_value: Option<TokenOrValue> = if let Some(visit) = visit_type.or(visit) {
          let js_value = this.env.to_js_value(env_var)?;
          let visit = visit.borrow_back(&this.env)?;
          let res = visit.call(js_value)?;
          if let Some(res) = res {
            this.env.from_js_value(res).map(serde_detach::detach)?
          } else {
            None
          }
        } else {
          None
        };

        match new_value {
          None => return Ok(()),
          Some(TokenOrValue::Length(l)) => *value = MediaFeatureValue::Length(Length::Value(l)),
          Some(TokenOrValue::Resolution(r)) => *value = MediaFeatureValue::Resolution(r),
          Some(TokenOrValue::Token(Token::Number { value: n, .. })) => *value = MediaFeatureValue::Number(n),
          Some(TokenOrValue::Token(Token::Ident(ident))) => *value = MediaFeatureValue::Ident(Ident(ident)),
          _ => {
            return Err(napi::Error::new(
              napi::Status::InvalidArg,
              format!("invalid environment value in media query: {:?}", new_value),
            ))
          }
        }

        Ok(())
      };

      call(VisitStage::Enter, value, &self)?;
      value.visit_children(self)?;
      call(VisitStage::Exit, value, &self)?;
      return Ok(());
    }

    value.visit_children(self)
  }

  fn visit_supports_condition(
    &mut self,
    condition: &mut lightningcss::rules::supports::SupportsCondition<'i>,
  ) -> Result<(), Self::Error> {
    visit(&self.env, condition, &self.visit_supports_condition.enter)?;
    condition.visit_children(self)?;
    visit(&self.env, condition, &self.visit_supports_condition.exit)
  }

  fn visit_custom_ident(
    &mut self,
    ident: &mut lightningcss::values::ident::CustomIdent,
  ) -> Result<(), Self::Error> {
    visit(&self.env, ident, &self.visit_custom_ident)
  }

  fn visit_dashed_ident(
    &mut self,
    ident: &mut lightningcss::values::ident::DashedIdent,
  ) -> Result<(), Self::Error> {
    visit(&self.env, ident, &self.visit_dashed_ident)
  }

  fn visit_selector_list(
    &mut self,
    selectors: &mut lightningcss::selector::SelectorList<'i>,
  ) -> Result<(), Self::Error> {
    let env = self.env;
    if let Some(visit) = &self.visit_selector {
      let visit = visit.borrow_back(&env)?;
      map::<_, _, _, true>(&mut selectors.0, |value| {
        let js_value = env.to_js_value(value)?;
        let res = visit.call(js_value)?;
        if let Some(res) = res {
          let new_value: ValueOrVec<_, true> = env.from_js_value(res).map(serde_detach::detach)?;
          Ok(Some(new_value))
        } else {
          Ok(None)
        }
      })?;
    }
    Ok(())
  }

  fn visit_token_list(
    &mut self,
    tokens: &mut lightningcss::properties::custom::TokenList<'i>,
  ) -> Result<(), Self::Error> {
    if self.types.contains(VisitTypes::TOKENS) {
      visit_list(
        self,
        &mut tokens.0,
        |this, value, stage| {
          let (visit_type, visit) = match value {
            TokenOrValue::Function(f) => (
              this.function_map.named(stage, f.name.0.as_ref()),
              this.visit_function.for_stage(stage),
            ),
            TokenOrValue::Var(_) => (None, this.visit_variable.for_stage(stage)),
            TokenOrValue::Env(e) => (
              this.env_map.named(stage, e.name.name()),
              this.visit_env.for_stage(stage),
            ),
            TokenOrValue::Token(t) => {
              let name = match t {
                Token::Ident(_) => Some("ident"),
                Token::AtKeyword(_) => Some("at-keyword"),
                Token::Hash(_) => Some("hash"),
                Token::IDHash(_) => Some("id-hash"),
                Token::String(_) => Some("string"),
                Token::Number { .. } => Some("number"),
                Token::Percentage { .. } => Some("percentage"),
                Token::Dimension { .. } => Some("dimension"),
                _ => None,
              };
              let visit = if let Some(name) = name {
                this.token_map.named(stage, name)
              } else {
                None
              };
              (visit, this.visit_token.for_stage(stage))
            }
            _ => return Ok(None),
          };

          if let Some(visit) = visit_type.or(visit) {
            let js_value = match value {
              TokenOrValue::Function(f) => this.env.to_js_value(f)?,
              TokenOrValue::Var(v) => this.env.to_js_value(v)?,
              TokenOrValue::Env(v) => this.env.to_js_value(v)?,
              TokenOrValue::Token(t) => this.env.to_js_value(t)?,
              _ => unreachable!(),
            };

            let visit = visit.borrow_back(&this.env)?;
            let res = visit.call(js_value)?;
            if let Some(res) = res {
              let res: Option<TokensOrRaw> = this.env.from_js_value(res).map(serde_detach::detach)?;
              Ok(res.map(|r| r.0))
            } else {
              Ok(None)
            }
          } else {
            Ok(None)
          }
        },
        |this, value| value.visit_children(this),
      )?;

      Ok(())
    } else {
      tokens.visit_children(self)
    }
  }
}

fn visit<V: Serialize + Deserialize<'static>>(
  env: &Env,
  value: &mut V,
  visit: &Option<FunctionRef<Unknown<'_>, Option<Unknown<'_>>>>,
) -> napi::Result<()> {
  if let Some(visit_ref) = visit {
    let visit = visit_ref.borrow_back(env)?;
    call_visitor(env, value, &visit)?;
  }
  Ok(())
}

fn call_visitor<V: Serialize + Deserialize<'static>>(
  env: &Env,
  value: &mut V,
  visit: &Function<Unknown<'_>, Option<Unknown<'_>>>,
) -> napi::Result<()> {
  let js_value: Unknown = env.to_js_value(value)?;
  let res = visit.call(js_value)?;
  if let Some(res) = res {
    let new_value: V = env.from_js_value(res).map(serde_detach::detach)?;
    *value = new_value;
  }
  Ok(())
}

fn visit_declaration_list<'i, C: FnMut(&mut JsVisitor, &mut Property<'i>) -> napi::Result<()>>(
  visitor: &mut JsVisitor,
  list: &mut Vec<Property<'i>>,
  visit_children: C,
) -> napi::Result<()> {
  visit_list(
    visitor,
    list,
    |this, value, stage| {
      let env = this.env;
      let visit = match value {
        Property::Custom(v) => {
          // FIXME see .custom
          if let Some(visit) = this.property_map.custom(stage, "custom", v.name.as_ref()) {
            let js_value = env.to_js_value(v)?;
            let res = visit.borrow_back(&this.env)?.call(js_value)?;
            if let Some(res) = res {
              return env.from_js_value(res).map(serde_detach::detach);
            } else {
              None
            }
          } else {
            None
          }
        }
        _ => this.property_map.named(stage, value.property_id().name()),
      };

      if let Some(visit) = visit.or(this.visit_declaration.for_stage(stage)) {
        let js_value = env.to_js_value(value)?;
        let visit = visit.borrow_back(&this.env)?;
        let res = visit.call(js_value)?;
        if let Some(res) = res {
          env.from_js_value(res).map(serde_detach::detach)
        } else {
          Ok(None)
        }
      } else {
        Ok(None)
      }
    },
    visit_children,
  )
}

// NOTE `env` is replaced by `visitor` because it's different to get the
// callbacks, passing `visitor` inside the function can fix the moved issue
// when invoking callbacks inside the closures.
fn visit_list<
  V,
  L: List<V>,
  F: FnMut(&mut JsVisitor, &mut V, VisitStage) -> napi::Result<Option<ValueOrVec<V>>>,
  C: FnMut(&mut JsVisitor, &mut V) -> napi::Result<()>,
>(
  visitor: &mut JsVisitor,
  list: &mut L,
  mut visit: F,
  mut visit_children: C,
) -> napi::Result<()> {
  map(list, |value| {
    let mut new_value: Option<ValueOrVec<V>> = visit(visitor, value, VisitStage::Enter)?;
    match &mut new_value {
      Some(ValueOrVec::Value(v)) => {
        visit_children(visitor, v)?;

        if let Some(val) = visit(visitor, v, VisitStage::Exit)? {
          new_value = Some(val);
        }
      }
      Some(ValueOrVec::Vec(v)) => {
        map(v, |value| {
          visit_children(visitor, value)?;
          visit(visitor, value, VisitStage::Exit)
        })?;
      }
      None => {
        visit_children(visitor, value)?;
        if let Some(val) = visit(visitor, value, VisitStage::Exit)? {
          new_value = Some(val);
        }
      }
    }

    Ok(new_value)
  })
}

fn map<V, L: List<V>, F: FnMut(&mut V) -> napi::Result<Option<ValueOrVec<V, IS_VEC>>>, const IS_VEC: bool>(
  list: &mut L,
  mut f: F,
) -> napi::Result<()> {
  let mut i = 0;
  while i < list.len() {
    let value = &mut list[i];
    let new_value = f(value)?;
    match new_value {
      Some(ValueOrVec::Value(v)) => {
        list[i] = v;
        i += 1;
      }
      Some(ValueOrVec::Vec(vec)) => {
        if vec.is_empty() {
          list.remove(i);
        } else {
          let len = vec.len();
          list.replace(i, vec);
          i += len;
        }
      }
      None => {
        i += 1;
      }
    }
  }
  Ok(())
}

#[derive(serde::Serialize)]
#[serde(untagged)]
enum ValueOrVec<V, const IS_VEC: bool = false> {
  Value(V),
  Vec(Vec<V>),
}

// Manually implemented deserialize for better error messages.
// https://github.com/serde-rs/serde/issues/773
impl<'de, V: serde::Deserialize<'de>, const IS_VEC: bool> serde::Deserialize<'de> for ValueOrVec<V, IS_VEC> {
  fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
  where
    D: serde::Deserializer<'de>,
  {
    use serde::Deserializer;
    let content = serde_content::Value::deserialize(deserializer)?;
    let de = serde_content::Deserializer::new(content.clone()).coerce_numbers();

    // Try to deserialize as a sequence first.
    let mut was_seq = false;
    let res = de.deserialize_seq(SeqVisitor {
      was_seq: &mut was_seq,
      phantom: PhantomData,
    });

    if was_seq {
      // Allow fallback if we know the value is also a list (e.g. selector).
      if res.is_ok() || !IS_VEC {
        return res.map_err(|e| serde::de::Error::custom(e.to_string())).map(ValueOrVec::Vec);
      }
    }

    // If it wasn't a sequence, try a value.
    let de = serde_content::Deserializer::new(content).coerce_numbers();
    return V::deserialize(de)
      .map_err(|e| serde::de::Error::custom(e.to_string()))
      .map(ValueOrVec::Value);

    struct SeqVisitor<'a, V> {
      was_seq: &'a mut bool,
      phantom: PhantomData<V>,
    }

    impl<'a, 'de, V: serde::Deserialize<'de>> serde::de::Visitor<'de> for SeqVisitor<'a, V> {
      type Value = Vec<V>;

      fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a sequence")
      }

      fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
      where
        A: serde::de::SeqAccess<'de>,
      {
        *self.was_seq = true;
        let mut vec = Vec::with_capacity(seq.size_hint().unwrap_or(1));
        while let Some(v) = seq.next_element()? {
          vec.push(v);
        }
        Ok(vec)
      }
    }
  }
}

struct TokensOrRaw<'i>(ValueOrVec<TokenOrValue<'i>>);

impl<'i, 'de: 'i> serde::Deserialize<'de> for TokensOrRaw<'i> {
  fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
  where
    D: serde::Deserializer<'de>,
  {
    #[derive(serde::Deserialize)]
    struct Raw<'i> {
      #[serde(borrow)]
      raw: CowArcStr<'i>,
    }

    let content = serde_content::Value::deserialize(deserializer)?;
    let de = serde_content::Deserializer::new(content.clone()).coerce_numbers();

    if let Ok(res) = Raw::deserialize(de) {
      let res = TokenList::parse_string_with_options(res.raw.as_ref(), ParserOptions::default())
        .map_err(|_| serde::de::Error::custom("Could not parse value"))?;
      return Ok(TokensOrRaw(ValueOrVec::Vec(res.into_owned().0)));
    }

    let de = serde_content::Deserializer::new(content).coerce_numbers();
    Ok(TokensOrRaw(
      ValueOrVec::deserialize(de).map_err(|e| serde::de::Error::custom(e.to_string()))?,
    ))
  }
}

trait List<V>: Index<usize, Output = V> + IndexMut<usize, Output = V> {
  fn len(&self) -> usize;
  fn remove(&mut self, i: usize);
  fn replace(&mut self, i: usize, items: Vec<V>);
}

impl<V> List<V> for Vec<V> {
  fn len(&self) -> usize {
    Vec::len(self)
  }

  fn remove(&mut self, i: usize) {
    Vec::remove(self, i);
  }

  fn replace(&mut self, i: usize, items: Vec<V>) {
    self.splice(i..i + 1, items);
  }
}

impl<V, T: smallvec::Array<Item = V>> List<V> for SmallVec<T> {
  fn len(&self) -> usize {
    SmallVec::len(self)
  }

  fn remove(&mut self, i: usize) {
    SmallVec::remove(self, i);
  }

  fn replace(&mut self, i: usize, items: Vec<V>) {
    let len = items.len();
    let mut iter = items.into_iter();
    self[i] = iter.next().unwrap();
    if len > 1 {
      self.insert_many(i + 1, iter);
    }
  }
}

impl<'i, R> List<CssRule<'i, R>> for CssRuleList<'i, R> {
  fn len(&self) -> usize {
    self.0.len()
  }

  fn remove(&mut self, i: usize) {
    self[i] = CssRule::Ignored;
  }

  fn replace(&mut self, i: usize, items: Vec<CssRule<'i, R>>) {
    self.0.replace(i, items)
  }
}
