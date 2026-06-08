use std::collections::BTreeMap;

// パラメータチューニング用の JSON 値型群
//
// nojson には所有権を持つ値型がないため、レイアウト JSON のクローン・パス指定での変更・
// 再シリアライズを行うために独自の型を定義している

/// JSON の数値 (整数または浮動小数)
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

/// 所有権を持つ JSON 値
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

/// JSON オブジェクトのメンバーへのパス (ドット結合で表現する)
///
/// 例: `"libvpx_vp9_encode_params.cpu_used"` は
/// `libvpx_vp9_encode_params` オブジェクトの `cpu_used` メンバーを指す。
///
/// NOTE: パス区切りに `.` を使うため、メンバー名自体に `.` を含むレイアウトには対応しない。
/// これは optuna ベース実装からの既存制約をそのまま踏襲している。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct JsonObjectMemberPath(Vec<String>);

impl JsonObjectMemberPath {
    /// パスが指す値への参照を返す (存在しなければ `None`)
    pub fn get<'a>(&self, mut value: &'a JsonValue) -> Option<&'a JsonValue> {
        for name in &self.0 {
            let JsonValue::Object(object) = value else {
                return None;
            };
            value = object.get(name)?;
        }
        Some(value)
    }

    /// パスが指す値への可変参照を返す (存在しなければ `None`)
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
