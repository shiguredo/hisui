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
    parse(json, Path::new("")).or_fail()
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

    // 長い行を省略する
    let (display_line, display_column) = truncate_line_for_display(line, column_num.get());
    let prev_display_line = prev_line.map(|prev| {
        let (truncated, _) = truncate_line_for_display(prev, column_num.get());
        truncated
    });

    orfail::Failure::new(format!(
        r#"{e}

INPUT:{}{}{}
{:4} |{display_line}
     |{:>column$} error

BACKTRACE:"#,
        if path.display().to_string().is_empty() {
            ""
        } else {
            " "
        },
        path.display(),
        if let Some(prev) = prev_display_line {
            format!("\n     |{prev}")
        } else {
            "".to_owned()
        },
        line_num,
        "^",
        column = display_column
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

    // 長い行を省略する
    let (display_line, display_column) = truncate_line_for_display(line, column_num.get());
    let prev_display_line = prev_line.map(|prev| {
        let (truncated, _) = truncate_line_for_display(prev, column_num.get());
        truncated
    });

    // エラー箇所のハイライト長も調整
    let highlight_length = std::cmp::min(
        value.as_raw_str().chars().count() - 1,
        display_line.chars().count() - display_column,
    );

    orfail::Failure::new(format!(
        r#"{e}

INPUT:{}{}{}
{:4} |{display_line}
     |{:>column$}{} {}

BACKTRACE:"#,
        if path.display().to_string().is_empty() {
            ""
        } else {
            " "
        },
        path.display(),
        if let Some(prev) = prev_display_line {
            format!("\n     |{prev}")
        } else {
            "".to_owned()
        },
        line_num,
        "^",
        std::iter::repeat_n('^', highlight_length).collect::<String>(),
        if let Some(reason) = e.source() {
            format!("{reason}")
        } else {
            "error".to_owned()
        },
        column = display_column
    ))
}

// エラー開始地点を起点にして、前後 50 文字まで含めるように切り詰める
fn truncate_line_for_display(line: &str, column_pos: usize) -> (String, usize) {
    let chars: Vec<char> = line.chars().collect();
    let max_context = 40;

    // column_pos は 1-based なので、0-based に変換
    let error_pos = column_pos.saturating_sub(1);

    // エラー位置が文字数を超えている場合は調整
    let error_pos = std::cmp::min(error_pos, chars.len());

    // 前後 40 文字の範囲を計算
    let start_pos = error_pos.saturating_sub(max_context);
    let end_pos = std::cmp::min(error_pos + max_context + 1, chars.len());

    let mut result = String::new();
    let mut new_column_pos = error_pos - start_pos + 1; // 1-based に戻す

    // 前方に省略がある場合
    if start_pos > 0 {
        result.push_str("...");
        new_column_pos += 3;
    }

    // 実際の文字列部分を追加
    result.push_str(&chars[start_pos..end_pos].iter().collect::<String>());

    // 後方に省略がある場合
    if end_pos < chars.len() {
        result.push_str("...");
    }

    (result, new_column_pos)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_single_line_malformed_json() {
        let malformed_json = r#"{"key": "value", "another": 123"#; // 閉じカッコがない

        let mut error = parse_str::<()>(malformed_json).err().expect("bug");
        error.backtrace.clear(); // 行番号を含めると壊れやすくなるので削除する
        eprintln!("{}", error.to_string());

        let expected = r#"unexpected EOS while parsing Object at byte position 31

INPUT:
   1 |{"key": "value", "another": 123
     |                               ^ error

BACKTRACE:
"#;
        assert_eq!(error.to_string(), expected);
    }

    #[test]
    fn test_parse_multiline_malformed_json() {
        // "another" の値の後ろにカンマがない
        let malformed_json = r#"{
        "key": "value",
        "another": 123
        "missing_comma": true
    }"#;

        let mut error = parse_str::<()>(malformed_json).err().expect("bug");
        error.backtrace.clear(); // 行番号を含めると壊れやすくなるので削除する
        eprintln!("{}", error.to_string());

        let expected = r#"unexpected char while parsing Object at byte position 57

INPUT:
     |        "another": 123
   4 |        "missing_comma": true
     |        ^ error

BACKTRACE:
"#;
        assert_eq!(error.to_string(), expected);
    }

    #[test]
    fn test_parse_long_single_line_malfomed_json() {
        // 200 文字を超える長い行で JSON が不正なケース
        let long_value = "a".repeat(150);
        let invalid_json = format!(
            r#"{{"key": "value", "foo": "bar", "very_long_key" "{}", "number": "not_a_number"}}"#,
            long_value
        );

        let mut error = parse_str::<()>(&invalid_json).err().expect("bug");
        error.backtrace.clear(); // 行番号を含めると壊れやすくなるので削除する
        eprintln!("{}", error.to_string());

        // エラーメッセージの行が 80 文字に収まるように切りつめられる
        let expected = r#"unexpected char while parsing Object at byte position 47

INPUT:
   1 |... "value", "foo": "bar", "very_long_key" "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa...
     |                                           ^ error

BACKTRACE:
"#;
        assert_eq!(error.to_string(), expected);
    }

    #[test]
    fn test_parse_long_multiline_malformed_json() {
        // 複数行で長い行を含む JSON が不正なケース
        let long_value = "a".repeat(100);
        let invalid_json = format!(
            r#"{{
    "very_long_key_with_long_value": "{}",
    "key": "value", "foo": "bar", "another_key" "missing_colon_value"
}}"#,
            long_value
        );

        let mut error = parse_str::<()>(&invalid_json).err().expect("bug");
        error.backtrace.clear(); // 行番号を含めると壊れやすくなるので削除する
        eprintln!("{}", error.to_string());

        // エラーメッセージの行が 80 文字に収まるように切りつめられる
        let expected = r#"unexpected char while parsing Object at byte position 191

INPUT:
     |...y_long_key_with_long_value": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa...
   3 |...": "value", "foo": "bar", "another_key" "missing_colon_value"
     |                                           ^ error

BACKTRACE:
"#;
        assert_eq!(error.to_string(), expected);
    }

    #[test]
    fn test_parse_invalid_json() {
        // 文法的には正しいけど値が不正な JSON
        let invalid_json = r#""not_a_number""#;

        let mut error = parse_str::<i32>(invalid_json).err().expect("bug");
        error.backtrace.clear(); // 行番号を含めると壊れやすくなるので削除する
        eprintln!("{}", error.to_string());

        let expected = r#"JSON String at byte position 0 is invalid: expected Integer, but found String

INPUT:
   1 |"not_a_number"
     |^^^^^^^^^^^^^^ expected Integer, but found String

BACKTRACE:
"#;
        assert_eq!(error.to_string(), expected);
    }
}
