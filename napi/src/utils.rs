use napi::{Error, JsObject, Result, Unknown};

// Workaround for https://github.com/napi-rs/napi-rs/issues/1641
pub fn get_named_property<T: TryFrom<Unknown<'static>, Error = Error>>(
  obj: &JsObject,
  property: &str,
) -> Result<T> {
  let unknown = obj.get_named_property::<Unknown>(property)?;
  T::try_from(unknown)
}
