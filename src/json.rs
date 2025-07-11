//! JSON 関連のユーティリティモジュール
use std::{borrow::Cow, collections::BTreeMap, path::Path};

use orfail::OrFail;

pub fn parse_file<P: AsRef<Path>, T>(path: P) -> orfail::Result<T>
where
    T: for<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>, Error = nojson::JsonParseError>,
{
    // TODO: エラーメッセージをわかりやすくする
    let json = std::fs::read_to_string(path).or_fail()?;
    parse_str(&json).or_fail()
}

pub fn parse_str<T>(json: &str) -> orfail::Result<T>
where
    T: for<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>, Error = nojson::JsonParseError>,
{
    // TODO: エラーメッセージをわかりやすくする
    Ok(json.parse::<nojson::Json<T>>().or_fail()?.0)
}

pub fn to_pretty_string<T: nojson::DisplayJson>(value: T) -> String {
    nojson::json(|f| {
        f.set_indent_size(2);
        f.set_spacing(true);
        f.value(&value)
    })
    .to_string()
}

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

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for JsonNumber {
    type Error = nojson::JsonParseError;

    fn try_from(value: nojson::RawJsonValue<'text, 'raw>) -> Result<Self, Self::Error> {
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

impl nojson::DisplayJson for JsonNumber {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            JsonNumber::Integer(v) => v.fmt(f),
            JsonNumber::Float(v) => v.fmt(f),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum JsonValue {
    Null,
    Boolean(bool),
    Integer(i64),
    Float(f64),
    String(String),
    Array(Vec<JsonValue>),
    Object(BTreeMap<String, JsonValue>),
}

impl<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>> for JsonValue {
    type Error = nojson::JsonParseError;

    fn try_from(value: nojson::RawJsonValue<'text, 'raw>) -> Result<Self, Self::Error> {
        match value.kind() {
            nojson::JsonValueKind::Null => Ok(JsonValue::Null),
            nojson::JsonValueKind::Boolean => Ok(JsonValue::Boolean(value.try_into()?)),
            nojson::JsonValueKind::Integer => Ok(JsonValue::Integer(value.try_into()?)),
            nojson::JsonValueKind::Float => Ok(JsonValue::Float(value.try_into()?)),
            nojson::JsonValueKind::String => Ok(JsonValue::String(value.try_into()?)),
            nojson::JsonValueKind::Array => Ok(JsonValue::Array(value.try_into()?)),
            nojson::JsonValueKind::Object => Ok(JsonValue::Object(value.try_into()?)),
        }
    }
}

impl nojson::DisplayJson for JsonValue {
    fn fmt(&self, f: &mut nojson::JsonFormatter<'_, '_>) -> std::fmt::Result {
        match self {
            JsonValue::Null => None::<()>.fmt(f),
            JsonValue::Boolean(v) => v.fmt(f),
            JsonValue::Integer(v) => v.fmt(f),
            JsonValue::Float(v) => v.fmt(f),
            JsonValue::String(v) => v.fmt(f),
            JsonValue::Array(v) => v.fmt(f),
            JsonValue::Object(v) => v.fmt(f),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct JsonObjectMemberPath(Vec<String>);

impl JsonObjectMemberPath {
    pub fn get<'a>(&self, mut value: &'a JsonValue) -> Option<&'a JsonValue> {
        for name in &self.0 {
            let JsonValue::Object(object) = value else {
                return None;
            };
            value = object.get(name)?;
        }
        Some(value)
    }

    pub fn get_mut<'a>(&self, mut value: &'a mut JsonValue) -> Option<&'a mut JsonValue> {
        for name in &self.0 {
            let JsonValue::Object(object) = value else {
                return None;
            };
            value = object.get_mut(name)?;
        }
        Some(value)
    }
}

impl std::str::FromStr for JsonObjectMemberPath {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(s.split('.').map(|s| s.to_owned()).collect()))
    }
}

impl std::fmt::Display for JsonObjectMemberPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.join("."))
    }
}
