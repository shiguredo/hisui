//! JSON 関連のユーティリティモジュール
use std::{borrow::Cow, collections::BTreeMap};

#[derive(Debug)]
pub struct JsonObject<'a, 'text> {
    object: nojson::RawJsonValue<'text, 'a>,
    members: BTreeMap<Cow<'text, str>, nojson::RawJsonValue<'text, 'a>>,
}

impl<'a, 'text> JsonObject<'a, 'text> {
    pub fn new(value: nojson::RawJsonValue<'text, 'a>) -> Result<Self, nojson::JsonParseError> {
        Ok(Self {
            object: value,
            members: value
                .to_object()?
                .map(|(k, v)| Ok((k.to_unquoted_string_str()?, v)))
                .collect::<Result<_, nojson::JsonParseError>>()?,
        })
    }

    pub fn get<T>(&self, key: &str) -> Result<Option<T>, nojson::JsonParseError>
    where
        T: TryFrom<nojson::RawJsonValue<'text, 'a>, Error = nojson::JsonParseError>,
    {
        let Some(value) = self.members.get(key).copied() else {
            return Ok(None);
        };
        Ok(Some(value.try_into()?))
    }

    pub fn get_with<T, F>(&self, key: &str, f: F) -> Result<Option<T>, nojson::JsonParseError>
    where
        F: FnOnce(nojson::RawJsonValue<'text, 'a>) -> Result<T, nojson::JsonParseError>,
    {
        let Some(value) = self.members.get(key).copied() else {
            return Ok(None);
        };
        Ok(Some(f(value)?))
    }

    pub fn get_required<T>(&self, key: &str) -> Result<T, nojson::JsonParseError>
    where
        T: TryFrom<nojson::RawJsonValue<'text, 'a>, Error = nojson::JsonParseError>,
    {
        let Some(value) = self.members.get(key).copied() else {
            return Err(self
                .object
                .invalid(format!("missing required member {key:?}")));
        };
        value.try_into()
    }

    pub fn get_required_with<T, F>(&self, key: &str, f: F) -> Result<T, nojson::JsonParseError>
    where
        F: FnOnce(nojson::RawJsonValue<'text, 'a>) -> Result<T, nojson::JsonParseError>,
    {
        let Some(value) = self.members.get(key).copied() else {
            return Err(self
                .object
                .invalid(format!("missing required member {key:?}")));
        };
        f(value)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum JsonNumber {
    Integer(i64),
    Float(f64),
}

impl<'text> nojson::FromRawJsonValue<'text> for JsonNumber {
    fn from_raw_json_value(
        value: nojson::RawJsonValue<'text, '_>,
    ) -> Result<Self, nojson::JsonParseError> {
        match value.kind() {
            nojson::JsonValueKind::Integer => {
                let int_value = value
                    .as_integer_str()?
                    .parse::<i64>()
                    .map_err(|e| value.invalid(e))?;
                Ok(JsonNumber::Integer(int_value))
            }
            nojson::JsonValueKind::Float => {
                let float_value = value
                    .as_float_str()?
                    .parse::<f64>()
                    .map_err(|e| value.invalid(e))?;
                Ok(JsonNumber::Float(float_value))
            }
            _ => Err(value.invalid("expected a number (integer or float)")),
        }
    }
}
