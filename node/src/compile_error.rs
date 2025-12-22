use lightningcss::bundler::BundleErrorKind;
use lightningcss::{
  css_modules::PatternParseError,
  error::{
    Error,
    //ErrorLocation,
    MinifyErrorKind,
    ParserError,
    PrinterErrorKind,
  },
};
use napi::bindgen_prelude::{Function, JsObjectValue, Object, ToNapiValue};
use napi::Env;

pub enum CompileError<'i, E: std::error::Error> {
  ParseError(Error<ParserError<'i>>),
  MinifyError(Error<MinifyErrorKind>),
  PrinterError(Error<PrinterErrorKind>),
  SourceMapError(parcel_sourcemap::SourceMapError),
  BundleError(Error<BundleErrorKind<'i, E>>),
  PatternError(PatternParseError),
  // TODO #[cfg(feature = "visitor")]
  JsError(napi::Error),
}

impl<'i, E: std::error::Error> std::fmt::Display for CompileError<'i, E> {
  fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
    match self {
      CompileError::ParseError(err) => err.kind.fmt(f),
      CompileError::MinifyError(err) => err.kind.fmt(f),
      CompileError::PrinterError(err) => err.kind.fmt(f),
      CompileError::BundleError(err) => err.kind.fmt(f),
      CompileError::PatternError(err) => err.fmt(f),
      CompileError::SourceMapError(err) => write!(f, "{}", err.to_string()), // TODO: switch to `fmt::Display` once parcel_sourcemap supports this
      // TODO #[cfg(feature = "visitor")]
      CompileError::JsError(err) => std::fmt::Debug::fmt(&err, f),
    }
  }
}

impl<'i, E: std::error::Error> From<Error<ParserError<'i>>> for CompileError<'i, E> {
  fn from(e: Error<ParserError<'i>>) -> CompileError<'i, E> {
    CompileError::ParseError(e)
  }
}

impl<'i, E: std::error::Error> From<Error<MinifyErrorKind>> for CompileError<'i, E> {
  fn from(err: Error<MinifyErrorKind>) -> CompileError<'i, E> {
    CompileError::MinifyError(err)
  }
}

impl<'i, E: std::error::Error> From<Error<PrinterErrorKind>> for CompileError<'i, E> {
  fn from(err: Error<PrinterErrorKind>) -> CompileError<'i, E> {
    CompileError::PrinterError(err)
  }
}

impl<'i, E: std::error::Error> From<parcel_sourcemap::SourceMapError> for CompileError<'i, E> {
  fn from(e: parcel_sourcemap::SourceMapError) -> CompileError<'i, E> {
    CompileError::SourceMapError(e)
  }
}

impl<'i, E: std::error::Error> From<Error<BundleErrorKind<'i, E>>> for CompileError<'i, E> {
  fn from(e: Error<BundleErrorKind<'i, E>>) -> CompileError<'i, E> {
    CompileError::BundleError(e)
  }
}

impl<'i, E: std::error::Error> From<napi::Error> for CompileError<'i, E> {
  fn from(e: napi::Error) -> Self {
    CompileError::JsError(e)
  }
}

impl<'i> From<CompileError<'i, napi::Error>> for napi::Error {
  fn from(e: CompileError<'i, napi::Error>) -> Self {
    match e {
      CompileError::JsError(err) => err,
      // TODO FIXME
      other => napi::Error::new(napi::Status::GenericFailure, other.to_string()),
    }
  }
}

pub enum CompileErrorOwned {
  JsError {
    err: napi::Error,
    loc: Option<lightningcss::error::ErrorLocation>,
  },
  Lightning {
    message: String,
    kind: String,
    loc: Option<lightningcss::error::ErrorLocation>,
    stage: &'static str,
  },
  Other(String),
}

impl<'i> From<CompileError<'i, napi::Error>> for CompileErrorOwned {
  fn from(e: CompileError<'i, napi::Error>) -> Self {
    use lightningcss::error::Error as LcError;

    let message = e.to_string(); // TODO not sure
    match e {
      CompileError::JsError(err) => Self::JsError { err, loc: None },
      CompileError::BundleError(LcError { kind, loc, .. }) => {
        if let BundleErrorKind::ResolverError(js_err) = kind {
          return Self::JsError { err: js_err, loc: loc };
        }

        Self::Lightning {
          message,
          kind: format!("{kind}"), // TODO not sure
          loc,
          stage: "BundleError",
        }
      }
      CompileError::ParseError(LcError { kind, loc, .. }) => Self::Lightning {
        message,
        kind: format!("{kind}"),
        loc,
        stage: "ParserError",
      },
      CompileError::PrinterError(LcError { kind, loc, .. }) => Self::Lightning {
        message,
        kind: format!("{kind}"),
        loc,
        stage: "PrinterError",
      },
      CompileError::MinifyError(LcError { kind, loc, .. }) => Self::Lightning {
        message,
        kind: format!("{kind}"),
        loc,
        stage: "MinifyError",
      },
      other => CompileErrorOwned::Other(other.to_string()),
    }
  }
}

fn attach_loc(
  env: &Env,
  obj: &mut Object,
  loc: &lightningcss::error::ErrorLocation,
  code: Option<&str>,
) -> napi::Result<()> {
  obj.set("fileName", loc.filename.clone())?;
  if let Some(code) = code {
    obj.set("source", code)?;
  }

  let mut loc_obj = Object::new(env)?;
  loc_obj.set("line", loc.line + 1)?;
  loc_obj.set("column", loc.column)?;
  obj.set("loc", loc_obj)?;

  Ok(())
}

fn syntax_error_object<'i>(env: &'i Env, message: &str) -> napi::Result<Object<'i>> {
  let syntax_error = env.get_global()?.get_named_property::<Function<String>>("SyntaxError")?;
  let obj = syntax_error.new_instance(message.to_string())?;
  let obj = unsafe { obj.cast::<Object>()? };
  Ok(obj)
}

impl CompileErrorOwned {
  pub fn into_js_error(self, env: Env, code: Option<&str>) -> napi::Result<napi::Error> {
    match self {
      CompileErrorOwned::JsError { err, loc } => {
        let js_err = err.into_unknown(&env)?;
        if js_err.get_type()? == napi::ValueType::Object {
          // let mut obj: Object = Object::from(js_err);
          let mut obj: Object = unsafe { js_err.cast::<Object>()? };
          if let Some(loc) = loc {
            attach_loc(&env, &mut obj, &loc, code)?;
          }
          let obj = obj.into_unknown(&env)?;
          Ok(obj.into())
        } else {
          Ok(js_err.into())
        }
      }
      CompileErrorOwned::Lightning {
        message,
        kind,
        loc,
        stage,
      } => {
        let mut obj = syntax_error_object(&env, &message)?;
        if let Some(loc) = loc {
          attach_loc(&env, &mut obj, &loc, code)?;
        }
        obj.set("stage", stage)?;
        obj.set("kind", kind)?;
        let unk = obj.into_unknown(&env)?;
        Ok(unk.into())
      }
      CompileErrorOwned::Other(message) => Ok(napi::Error::new(napi::Status::GenericFailure, message)),
    }
  }
}
