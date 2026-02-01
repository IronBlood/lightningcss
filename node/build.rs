#[cfg(not(target_arch = "wasm32"))]
extern crate napi_build;

fn main() {
  #[cfg(all(not(target_arch = "wasm32"), not(all(target_os = "windows", target_env = "gnu"))))]
  napi_build::setup();
}
