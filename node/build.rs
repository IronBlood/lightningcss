#[cfg(not(target_arch = "wasm32"))]
extern crate napi_build;

fn main() {
  let target = std::env::var("TARGET").unwrap_or_default();
  if target.ends_with("-pc-windows-gnu") {
    return;
  }

  #[cfg(not(target_arch = "wasm32"))]
  napi_build::setup();
}
