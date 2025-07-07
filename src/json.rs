//! JSON 関連のユーティリティモジュール
use std::{borrow::Cow, collections::BTreeMap};

#[derive(Debug)]
pub struct JsonObject<'a, 'text>(BTreeMap<Cow<'text, str>, nojson::RawJsonValue<'text, 'a>>);

impl<'a, 'text> JsonObject<'a, 'text> {
    pub fn new(value: nojson::RawJsonValue<'text, 'a>) -> Result<Self, nojson::JsonParseError> {
        Ok(Self(
            value
                .to_object()?
                .map(|(k, v)| Ok((k.to_unquoted_string_str()?, v)))
                .collect::<Result<_, _>>()?,
        ))
    }

    pub fn get<T>(&self, key: &str) -> Result<Option<T>, nojson::JsonParseError>
    where
        T: nojson::FromRawJsonValue<'text>,
    {
        let Some(value) = self.0.get(key) else {
            return Ok(None);
        };
        Ok(Some(value.try_to()?))
    }

    pub fn get_with<T, F>(&self, key: &str, f: F) -> Result<Option<T>, nojson::JsonParseError>
    where
        F: FnOnce(nojson::RawJsonValue<'text, 'a>) -> Result<T, nojson::JsonParseError>,
    {
        let Some(value) = self.0.get(key).copied() else {
            return Ok(None);
        };
        Ok(Some(f(value)?))
    }
}
