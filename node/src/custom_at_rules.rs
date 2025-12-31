use lightningcss::values::syntax::SyntaxString;
use napi::{bindgen_prelude::Null, Either, Status};
use napi_derive::napi;
use std::collections::HashMap;

use crate::at_rule_parser::{CustomAtRuleBodyType, CustomAtRuleConfig};

// TODO manually calculated
#[napi(string_enum)]
pub enum PreludeTypes {
  #[napi(value = "length")]
  Length,
  #[napi(value = "number")]
  Number,
  #[napi(value = "percentage")]
  Percentage,
  #[napi(value = "length-percentage")]
  LengthPercentage,
  #[napi(value = "color")]
  Color,
  #[napi(value = "image")]
  Image,
  #[napi(value = "url")]
  Url,
  #[napi(value = "integer")]
  Integer,
  #[napi(value = "angle")]
  Angle,
  #[napi(value = "time")]
  Time,
  #[napi(value = "resolution")]
  Resolution,
  #[napi(value = "transform-function")]
  TransformFunction,
  #[napi(value = "transform-list")]
  TransformList,
  #[napi(value = "custom-ident")]
  CustomIdent,
  #[napi(value = "token-list")]
  TokenList,
}

#[napi(object)]
#[derive(Debug, Clone)]
pub struct CustomAtRuleDefinition {
  /**
   * Defines the syntax for a custom at-rule prelude. The value should be a
   * CSS [syntax string](https://drafts.css-houdini.org/css-properties-values-api/#syntax-strings)
   * representing the types of values that are accepted. This property may be omitted or
   * set to null to indicate that no prelude is accepted.
   */
  #[napi(ts_type = "`<${PreludeTypes}>` | `<${PreludeTypes}>+` | `<${PreludeTypes}>#` | (string & {})")]
  pub prelude: Option<Either<String, Null>>,
  /**
   * Defines the type of body contained within the at-rule block.
   *   - declaration-list: A CSS declaration list, as in a style rule.
   *   - rule-list: A list of CSS rules, as supported within a non-nested
   *       at-rule such as `@media` or `@supports`.
   *   - style-block: Both a declaration list and rule list, as accepted within
   *       a nested at-rule within a style rule (e.g. `@media` inside a style rule
   *       with directly nested declarations).
   */
  #[napi(ts_type = "'declaration-list' | 'rule-list' | 'style-block'")]
  pub body: Option<String>,
}

#[napi]
pub type CustomAtRules = HashMap<String, CustomAtRuleDefinition>;

impl TryFrom<CustomAtRuleDefinition> for CustomAtRuleConfig {
  type Error = napi::Error;
  fn try_from(def: CustomAtRuleDefinition) -> Result<Self, Self::Error> {
    let body = match def.body.as_deref() {
      Some("declaration-list") => Some(CustomAtRuleBodyType::DeclarationList),
      Some("rule-list") => Some(CustomAtRuleBodyType::RuleList),
      Some("style-block") => Some(CustomAtRuleBodyType::StyleBlock),
      _ => None,
    };
    Ok(CustomAtRuleConfig {
      prelude: parse_prelude(match def.prelude {
        Some(Either::A(s)) => Some(s),
        _ => None,
      })?,
      body,
    })
  }
}

fn parse_prelude(s: Option<String>) -> napi::Result<Option<SyntaxString>> {
  s.map(|raw| {
    SyntaxString::parse_string(&raw)
      .map_err(|_| napi::Error::new(Status::InvalidArg, "Invalid custom at-rule prelude syntax"))
  })
  .transpose()
}
