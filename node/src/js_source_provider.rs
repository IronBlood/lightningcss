use std::{path::PathBuf, sync::Mutex};

use crossbeam_channel::{self, Receiver, Sender};
use lightningcss::{bundler::SourceProvider, stylesheet::StyleSheet, visitor::Visit};
use napi::{
  bindgen_prelude::{FnArgs, FromNapiValue, Function, Object, Promise},
  threadsafe_function::{ThreadsafeFunction, ThreadsafeFunctionCallMode},
  Env, JsValue, Status, Unknown,
};

use crate::{
  at_rule_parser::AtRule,
  bundle::{compile_bundle, BundleConfig},
  compile_error::CompileErrorOwned,
  transform::TransformResult,
  transformer::JsVisitor,
};

thread_local! {
  static CHANNEL: (Sender<napi::Result<String>>, Receiver<napi::Result<String>>) = crossbeam_channel::unbounded();
}
pub struct JsSourceProvider {
  pub resolve: Option<
    ThreadsafeFunction<FnArgs<(String, String)>, Unknown<'static>, FnArgs<(String, String)>, Status, false>,
  >,
  pub read: Option<ThreadsafeFunction<String, Unknown<'static>, String, Status, false>>,
  pub inputs: Mutex<Vec<*mut String>>,
}

// NOTE: there are two `JsSourceProvider` in `napi/src/lib.rs`,
//       one is for wasm, the other is for non-wasm. Only the
//       wasm drops `env` held by `read`/`resolve`. The following
//       code is suggested by ChatGPT.
impl Drop for JsSourceProvider {
  fn drop(&mut self) {
    if let Ok(mut v) = self.inputs.lock() {
      for ptr in v.drain(..) {
        unsafe {
          drop(Box::from_raw(ptr));
        }
      }
    }
  }
}

unsafe impl Sync for JsSourceProvider {}
unsafe impl Send for JsSourceProvider {}

impl SourceProvider for JsSourceProvider {
  type Error = napi::Error;
  fn read<'a>(&'a self, file: &std::path::Path) -> Result<&'a str, Self::Error> {
    let source = if let Some(read) = &self.read {
      CHANNEL.with(|channel| {
        let tx = channel.0.clone();
        let tx_cb = tx.clone();
        let file = file.to_str().unwrap().to_owned();
        let status =
          read.call_with_return_value(file, ThreadsafeFunctionCallMode::Blocking, move |js_result, env| {
            let ret = match js_result {
              Ok(v) => v,
              Err(e) => {
                let _ = tx_cb.send(Err(e));
                return Ok(());
              }
            };
            if ret.is_promise()? {
              let p = Promise::<String>::from_unknown(ret)?;
              env.spawn_future(async move {
                let s = p.await;
                let _ = tx_cb.send(s);
                Ok::<(), napi::Error>(())
              })?;
            } else {
              let s = String::from_unknown(ret);
              let _ = tx_cb.send(s);
            }
            Ok(())
          });
        if status != Status::Ok {
          let _ = tx.send(Err(napi::Error::new(status, "failed to call resolve()")));
        }
        channel.1.recv().unwrap()
      })
    } else {
      Ok(std::fs::read_to_string(file)?)
    };

    match source {
      Ok(source) => {
        let ptr = Box::into_raw(Box::new(source));
        self.inputs.lock().unwrap().push(ptr);
        Ok(unsafe { &*ptr })
      }
      Err(e) => Err(e),
    }
  }

  fn resolve(
    &self,
    specifier: &str,
    originating_file: &std::path::Path,
  ) -> Result<std::path::PathBuf, Self::Error> {
    if let Some(resolve) = &self.resolve {
      return CHANNEL.with(|channel| {
        let arg: FnArgs<(String, String)> =
          (specifier.to_owned(), originating_file.to_str().unwrap().to_owned()).into();
        let tx = channel.0.clone();
        let tx_cb = tx.clone();
        let status =
          resolve.call_with_return_value(arg, ThreadsafeFunctionCallMode::Blocking, move |js_result, env| {
            let ret = match js_result {
              Ok(v) => v,
              Err(e) => {
                let _ = tx_cb.send(Err(e));
                return Ok(());
              }
            };
            if ret.is_promise()? {
              let p = Promise::<String>::from_unknown(ret)?;
              env.spawn_future(async move {
                let s = p.await;
                let _ = tx_cb.send(s);
                Ok::<(), napi::Error>(())
              })?;
            } else {
              let s = String::from_unknown(ret);
              let _ = tx_cb.send(s);
            }
            Ok(())
          });
        if status != Status::Ok {
          let _ = tx.send(Err(napi::Error::new(status, "failed to call resolve()")));
        }
        let result = channel.1.recv().unwrap();
        match result {
          Ok(result) => Ok(PathBuf::from(result)),
          Err(e) => Err(e),
        }
      });
    }
    Ok(originating_file.with_file_name(specifier))
  }
}

struct VisitMessage {
  stylesheet: &'static mut StyleSheet<'static, 'static, AtRule<'static>>,
  tx: Sender<napi::Result<String>>,
}

// Runs bundling on a background thread managed by rayon. This is similar to AsyncTask from napi-rs, however,
// because we call back into the JS thread, which might call other tasks in the node threadpool (e.g. fs.readFile),
// we may end up deadlocking if the number of rayon threads exceeds node's threadpool size. Therefore, we must
// run bundling from a thread not managed by Node.
pub fn run_bundle_task<P>(
  provider: P,
  config: BundleConfig,
  visitor: Option<JsVisitor>,
  env: Env,
) -> napi::Result<Object<'static>>
where
  P: 'static + SourceProvider<Error = napi::Error>,
{
  let (deferred, promise) = env.create_deferred::<TransformResult, _>()?;

  // NOTE WARN Detach the lifetime from `&env` borrow. (By ChatGPT)
  // SAFETY: this is an N-API value; the `'env` lifetime is only a Rust typing constraint.
  let promise: Object<'static> = unsafe { std::mem::transmute::<Object<'_>, Object<'static>>(promise) };
  let noop: Function<(), ()> = env.create_function_from_closure("lightningcssVisitNoop", |_ctx| Ok(()))?;
  let tsfn = if let Some(mut visitor) = visitor {
    Some(
      noop.build_threadsafe_function::<VisitMessage>().build_callback(move |ctx| {
        if let Err(err) = ctx.value.stylesheet.visit(&mut visitor) {
          ctx.value.tx.send(Err(err)).expect("send error");
          return Ok(());
        }
        ctx.value.tx.send(Ok(Default::default())).expect("send error");
        Ok(())
      })?,
    )
  } else {
    None
  };

  rayon::spawn(move || {
    let res = compile_bundle(
      unsafe { std::mem::transmute::<&'_ P, &'static P>(&provider) },
      &config,
      tsfn.map(move |tsfn| {
        move |stylesheet: &mut StyleSheet<AtRule>| {
          CHANNEL.with(|channel| {
            let message = VisitMessage {
              // SAFETY: we immediately lock the thread until we get a response,
              // so stylesheet cannot be dropped in that time.
              stylesheet: unsafe {
                std::mem::transmute::<
                  &'_ mut StyleSheet<'_, '_, AtRule>,
                  &'static mut StyleSheet<'static, 'static, AtRule>,
                >(stylesheet)
              },
              tx: channel.0.clone(),
            };

            tsfn.call(message, ThreadsafeFunctionCallMode::Blocking);
            channel.1.recv().expect("recv error").map(|_| ())
          })
        }
      }),
    );

    deferred.resolve(move |env| match res {
      Ok(v) => Ok(v),
      Err(err) => {
        let owned: CompileErrorOwned = err.into();
        Err(owned.into_js_error(env, None)?)
      }
    });
  });

  Ok(promise)
}
