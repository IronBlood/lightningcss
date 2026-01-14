#[cfg(target_os = "macos")]
#[global_allocator]
static GLOBAL: jemallocator::Jemalloc = jemallocator::Jemalloc;

pub mod at_rule_parser;
#[cfg(not(target_arch = "wasm32"))]
pub mod bundle;
#[cfg(not(target_arch = "wasm32"))]
pub mod bundle_async;
pub mod bundle_common;
#[cfg(target_arch = "wasm32")]
pub mod bundle_wasm;
pub mod compile_error;
pub mod css_module_reference;
pub mod custom_at_rules;
#[cfg(not(target_arch = "wasm32"))]
pub mod js_source_provider;
pub mod transform;
pub mod transform_style_attribute;
pub mod transformer;
