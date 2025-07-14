//! JSON 関連のユーティリティモジュール
use std::{borrow::Cow, collections::BTreeMap, error::Error, num::NonZeroUsize, path::Path};

use orfail::OrFail;

pub fn parse_file<P: AsRef<Path>, T>(path: P) -> orfail::Result<T>
where
    T: for<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>, Error = nojson::JsonParseError>,
{
    let json = std::fs::read_to_string(&path)
        .or_fail_with(|e| format!("faild to read file {}: {e}", path.as_ref().display()))?;
    parse(&json, path.as_ref()).or_fail()
}

pub fn parse_str<T>(json: &str) -> orfail::Result<T>
where
    T: for<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>, Error = nojson::JsonParseError>,
{
    parse(json, Path::new("nofile")).or_fail()
}

fn parse<T>(text: &str, path: &Path) -> orfail::Result<T>
where
    T: for<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>, Error = nojson::JsonParseError>,
{
    let json = nojson::RawJson::parse(text)
        .map_err(|e| malformed_json_error(path, text, e))
        .or_fail()?;
    json.value()
        .try_into()
        .map_err(|e| invalid_json_error(path, &json, e))
        .or_fail()
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

fn malformed_json_error(path: &Path, text: &str, e: nojson::JsonParseError) -> orfail::Failure {
    let (line_num, column_num) = if e.position() == text.len() {
        // TODO(sile): nojson がこのケースを扱えるようになったら削除する
        (
            NonZeroUsize::new(text.lines().count()).unwrap_or(NonZeroUsize::MIN),
            text.lines()
                .last()
                .map(|line| line.chars().count() + 1)
                .and_then(NonZeroUsize::new)
                .unwrap_or(NonZeroUsize::MIN),
        )
    } else {
        e.get_line_and_column_numbers(text).expect("infallible")
    };
    let line = e.get_line(text).expect("infallible");
    let prev_line = if line_num.get() == 1 {
        None
    } else {
        text.lines().nth(line_num.get() - 2)
    };
    orfail::Failure::new(format!(
        r#"{e}

INPUT: {}{}
{:4} |{line}
     |{:>column$} error

BACKTRACE:"#,
        path.display(),
        if let Some(prev) = prev_line {
            format!("\n     |{prev}")
        } else {
            "".to_owned()
        },
        line_num,
        "^",
        column = column_num.get()
    ))
}

fn invalid_json_error(
    path: &Path,
    json: &nojson::RawJson,
    e: nojson::JsonParseError,
) -> orfail::Failure {
    let text = json.text();
    let (line_num, column_num) = e.get_line_and_column_numbers(text).expect("infallible");
    let line = e.get_line(text).expect("infallible");
    let prev_line = if line_num.get() == 1 {
        None
    } else {
        text.lines().nth(line_num.get() - 2)
    };
    let value = json
        .get_value_by_position(e.position())
        .expect("infallible");
    orfail::Failure::new(format!(
        r#"{e}

INPUT: {}{}
{:4} |{line}
     |{:>column$}{} {}

BACKTRACE:"#,
        path.display(),
        if let Some(prev) = prev_line {
            format!("\n     |{prev}")
        } else {
            "".to_owned()
        },
        line_num,
        "^",
        std::iter::repeat_n('^', value.as_raw_str().len() - 1).collect::<String>(),
        if let Some(reason) = e.source() {
            format!("{reason}")
        } else {
            "error".to_owned()
        },
        column = column_num.get()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_singlie_line_malformed_json() {
        let malformed_json = r#"{"key": "value", "another": 123"#; // 閉じかっこがない

        let mut error = parse_str::<()>(malformed_json).err().expect("bug");
        error.backtrace.clear(); // 行番号を含めると壊れやすくなるので削除する
        eprintln!("{}", error.to_string());

        let expected = r#"unexpected EOS while parsing Object at byte position 31

INPUT: nofile
   1 |{"key": "value", "another": 123
     |                               ^ error

BACKTRACE:
"#;
        assert_eq!(error.to_string(), expected);
    }

    //     #[test]
    //     fn test_malformed_json_error_with_multiline() {
    //         // Test malformed JSON with multiple lines to check prev_line display
    //         let malformed_json = r#"{
    //     "key": "value",
    //     "another": 123
    //     "missing_comma": true
    // }"#;

    //         let result: orfail::Result<JsonValue> = parse_str(malformed_json);
    //         assert!(result.is_err());

    //         let error_msg = format!("{}", result.unwrap_err());
    //         let expected = r#"unexpected string at line 4, column 5

    // INPUT: nofile
    //      |    "another": 123
    //    4 |    "missing_comma": true
    //      |     ^ error

    // BACKTRACE:"#;

    //         assert_eq!(error_msg, expected);
    //     }

    //     #[test]
    //     fn test_invalid_json_error_via_parse_str() {
    //         // Test with syntactically valid JSON but invalid for specific type conversion
    //         let valid_json_invalid_conversion = r#"{"key": "not_a_number"}"#;

    //         // Try to parse as a number which should fail during conversion
    //         let result: orfail::Result<i64> = parse_str(valid_json_invalid_conversion);
    //         assert!(result.is_err());

    //         let error_msg = format!("{}", result.unwrap_err());
    //         let expected = r#"expected integer at line 1, column 1

    // INPUT: nofile
    //    1 |{"key": "not_a_number"}
    //      |^ error

    // BACKTRACE:"#;

    //         assert_eq!(error_msg, expected);
    //     }

    //     #[test]
    //     fn test_invalid_json_error_with_multiline() {
    //         // Test invalid JSON error with multiple lines
    //         let json = r#"{
    //     "valid_key": "valid_value",
    //     "invalid_key": "this_should_be_number"
    // }"#;

    //         // This should parse as JSON but fail when trying to convert to a specific type
    //         let result: orfail::Result<i64> = parse_str(json);
    //         assert!(result.is_err());

    //         let error_msg = format!("{}", result.unwrap_err());
    //         let expected = r#"expected integer at line 1, column 1

    // INPUT: nofile
    //    1 |{
    //      |^ error

    // BACKTRACE:"#;

    //         assert_eq!(error_msg, expected);
    //     }

    //     #[test]
    //     fn test_error_formatting_with_first_line() {
    //         // Test error on first line (no previous line to show)
    //         let malformed_json = r#"{"key": "value""#; // Missing closing quote and brace

    //         let result: orfail::Result<JsonValue> = parse_str(malformed_json);
    //         assert!(result.is_err());

    //         let error_msg = format!("{}", result.unwrap_err());
    //         let expected = r#"unexpected end of input at line 1, column 16

    // INPUT: nofile
    //    1 |{"key": "value"
    //      |                ^ error

    // BACKTRACE:"#;

    //         assert_eq!(error_msg, expected);
    //     }

    //     #[test]
    //     fn test_malformed_json_error_second_line() {
    //         // Test error on second line to verify previous line display
    //         let malformed_json = r#"{
    //     "key": "value",
    //     "another": 123,
    //     "bad": invalid
    // }"#;

    //         let result: orfail::Result<JsonValue> = parse_str(malformed_json);
    //         assert!(result.is_err());

    //         let error_msg = format!("{}", result.unwrap_err());
    //         let expected = r#"unexpected keyword at line 4, column 12

    // INPUT: nofile
    //      |    "another": 123,
    //    4 |    "bad": invalid
    //      |            ^ error

    // BACKTRACE:"#;

    //         assert_eq!(error_msg, expected);
    //     }

    //     #[test]
    //     fn test_invalid_json_error_with_specific_value() {
    //         // Test invalid JSON error that targets a specific value
    //         let json = r#"[1, 2, "not_a_number"]"#;

    //         // Try to parse as Vec<i64> which should fail on the string element
    //         let result: orfail::Result<Vec<i64>> = parse_str(json);
    //         assert!(result.is_err());

    //         let error_msg = format!("{}", result.unwrap_err());
    //         let expected = r#"expected integer at line 1, column 8

    // INPUT: nofile
    //    1 |[1, 2, "not_a_number"]
    //      |        ^^^^^^^^^^^^^^ error

    // BACKTRACE:"#;

    //         assert_eq!(error_msg, expected);
    //     }
}
