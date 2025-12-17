use napi::bindgen_prelude::Either3;
use napi_derive::napi;

#[napi(object)]
pub struct LocalCSSModuleReference {
  #[napi(js_name = "type", ts_type = "'local'")]
  pub _type: String,
  /** The local (compiled) name for the reference. */
  pub name: String,
}

#[napi(object)]
pub struct GlobalCSSModuleReference {
  #[napi(js_name = "type", ts_type = "'global'")]
  pub _type: String,
  /** The referenced global name. */
  pub name: String,
}

#[napi(object)]
pub struct DependencyCSSModuleReference {
  #[napi(js_name = "type", ts_type = "'dependency'")]
  pub _type: String,
  /** The name to reference within the dependency. */
  pub name: String,
  /** The dependency specifier for the referenced file. */
  pub specifier: String,
}

#[napi]
pub type CSSModuleReference =
  Either3<LocalCSSModuleReference, GlobalCSSModuleReference, DependencyCSSModuleReference>;

pub fn convert_css_module_ref(r: lightningcss::css_modules::CssModuleReference) -> CSSModuleReference {
  match r {
    lightningcss::css_modules::CssModuleReference::Local { name } => Either3::A(LocalCSSModuleReference {
      _type: "local".into(),
      name,
    }),
    lightningcss::css_modules::CssModuleReference::Global { name } => Either3::B(GlobalCSSModuleReference {
      _type: "global".into(),
      name,
    }),
    lightningcss::css_modules::CssModuleReference::Dependency { name, specifier } => {
      Either3::C(DependencyCSSModuleReference {
        _type: "dependency".into(),
        name,
        specifier,
      })
    }
  }
}
