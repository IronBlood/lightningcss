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
      other => napi::Error::new(napi::Status::GenericFailure, other.to_string()),
    }
  }
}
