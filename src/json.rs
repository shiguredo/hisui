//! JSON 関連のユーティリティモジュール
use std::{error::Error, io::Write, path::Path};

// エラーメッセージに、入力 JSON の問題となっている行を表示する際の文字数の最大値。
// これを超える場合には超過分の前後が ... で置換される。
const MAX_ERROR_LINE_CHARS: usize = 80;

pub fn parse_file<P: AsRef<Path>, T>(path: P) -> crate::Result<T>
where
    T: for<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>, Error = nojson::JsonParseError>,
{
    let enable_jsonc = path.as_ref().extension().is_some_and(|e| e == "jsonc");
    let json = std::fs::read_to_string(&path).map_err(|e| {
        crate::Error::new(format!(
            "failed to read file {}: {e}",
            path.as_ref().display()
        ))
    })?;
    parse(&json, path.as_ref(), enable_jsonc)
}

pub fn parse_str<T>(json: &str) -> crate::Result<T>
where
    T: for<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>, Error = nojson::JsonParseError>,
{
    parse(json, Path::new(""), true)
}

fn parse<T>(text: &str, path: &Path, enable_jsonc: bool) -> crate::Result<T>
where
    T: for<'text, 'raw> TryFrom<nojson::RawJsonValue<'text, 'raw>, Error = nojson::JsonParseError>,
{
    let json = if enable_jsonc {
        nojson::RawJson::parse_jsonc(text).map(|(json, _comments)| json)
    } else {
        nojson::RawJson::parse(text)
    }
    .map_err(|e| malformed_json_error(path, text, e))?;
    json.value()
        .try_into()
        .map_err(|e| invalid_json_error(path, &json, e))
}

pub fn to_pretty_string<T: nojson::DisplayJson>(value: T) -> String {
    nojson::json(|f| {
        f.set_indent_size(2);
        f.set_spacing(true);
        f.value(&value)
    })
    .to_string()
}

pub fn pretty_print<T: nojson::DisplayJson>(value: T) -> crate::Result<()> {
    let stdout = std::io::stdout();
    let result = writeln!(
        stdout.lock(),
        "{}",
        nojson::json(|f| {
            f.set_indent_size(2);
            f.set_spacing(true);
            f.value(&value)
        })
    );
    match result {
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {
            // 出力先のパイプが途中閉じられた場合はエラーにしない
            Ok(())
        }
        Err(e) => Err(e.into()),
        Ok(()) => Ok(()),
    }
}

fn malformed_json_error(path: &Path, text: &str, e: nojson::JsonParseError) -> crate::Error {
    let (line_num, column_num) = e.get_line_and_column_numbers(text).expect("infallible");
    let line = e.get_line(text).expect("infallible");
    let prev_line = if line_num.get() == 1 {
        None
    } else {
        text.lines().nth(line_num.get() - 2)
    };

    // 長い行を省略する
    let (display_line, display_column) = format_line_around_position(line, column_num.get());
    let prev_display_line = prev_line.map(|prev| {
        let (truncated, _) = format_line_around_position(prev, column_num.get());
        truncated
    });

    crate::Error::new(format!(
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
) -> crate::Error {
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
    let (display_line, display_column) = format_line_around_position(line, column_num.get());
    let prev_display_line = prev_line.map(|prev| {
        let (truncated, _) = format_line_around_position(prev, column_num.get());
        truncated
    });

    // エラー箇所のハイライト長も調整
    let highlight_length = std::cmp::min(
        value.as_raw_str().chars().count() - 1,
        display_line.chars().count() - display_column,
    );

    crate::Error::new(format!(
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

// エラー表示用に指定位置周辺の行をフォーマットします
//
// この関数は、テキストの行と列位置を受け取り、エラー位置を中心として
// MAX_ERROR_LINE_CHARS 文字以内に収まるように前後の内容を切り詰めてフォーマットします。
// エラー位置は可能な限りフォーマット後の出力の中央付近に配置されます。
fn format_line_around_position(line: &str, column_pos: usize) -> (String, usize) {
    let chars: Vec<char> = line.chars().collect();
    let max_context = MAX_ERROR_LINE_CHARS / 2;

    // column_pos は 1-based なので、0-based に変換
    let error_pos = column_pos.saturating_sub(1);

    // エラー位置が文字数を超えている場合は調整
    let error_pos = std::cmp::min(error_pos, chars.len());

    // 前後に含める文字の範囲を計算
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

        let error = parse_str::<()>(malformed_json).expect_err("bug");
        eprintln!("{}", error.display());

        let expected = r#"unexpected EOS while parsing Object at byte position 31

INPUT:
   1 |{"key": "value", "another": 123
     |                               ^ error

BACKTRACE:"#;
        assert_eq!(error.reason, expected);
    }

    #[test]
    fn test_parse_multiline_malformed_json() {
        // "another" の値の後ろにカンマがない
        let malformed_json = r#"{
        "key": "value",
        "another": 123
        "missing_comma": true
    }"#;

        let error = parse_str::<()>(malformed_json).expect_err("bug");
        eprintln!("{}", error.display());

        let expected = r#"unexpected char while parsing Object at byte position 57

INPUT:
     |        "another": 123
   4 |        "missing_comma": true
     |        ^ error

BACKTRACE:"#;
        assert_eq!(error.reason, expected);
    }

    #[test]
    fn test_parse_long_single_line_malfomed_json() {
        // 200 文字を超える長い行で JSON が不正なケース
        let long_value = "a".repeat(150);
        let invalid_json = format!(
            r#"{{"key": "value", "foo": "bar", "very_long_key" "{}", "number": "not_a_number"}}"#,
            long_value
        );

        let error = parse_str::<()>(&invalid_json).expect_err("bug");
        eprintln!("{}", error.display());

        // エラーメッセージの行が MAX_ERROR_LINE_CHARS 文字に収まるように切りつめられる
        let expected = r#"unexpected char while parsing Object at byte position 47

INPUT:
   1 |... "value", "foo": "bar", "very_long_key" "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa...
     |                                           ^ error

BACKTRACE:"#;
        assert_eq!(error.reason, expected);
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

        let error = parse_str::<()>(&invalid_json).expect_err("bug");
        eprintln!("{}", error.display());

        // エラーメッセージの行が MAX_ERROR_LINE_CHARS 文字に収まるように切りつめられる
        let expected = r#"unexpected char while parsing Object at byte position 191

INPUT:
     |...y_long_key_with_long_value": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa...
   3 |...": "value", "foo": "bar", "another_key" "missing_colon_value"
     |                                           ^ error

BACKTRACE:"#;
        assert_eq!(error.reason, expected);
    }

    #[test]
    fn test_parse_invalid_json() {
        // 文法的には正しいけど値が不正な JSON
        let invalid_json = r#""not_a_number""#;

        let error = parse_str::<i32>(invalid_json).expect_err("bug");
        eprintln!("{}", error.display());

        let expected = r#"JSON String at byte position 0 is invalid: expected Integer, but found String

INPUT:
   1 |"not_a_number"
     |^^^^^^^^^^^^^^ expected Integer, but found String

BACKTRACE:"#;
        assert_eq!(error.reason, expected);
    }
}
