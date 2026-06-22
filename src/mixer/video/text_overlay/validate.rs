//! テキストオーバーレイの入力検証ロジック。
//!
//! `text` のバイト数 / 行数チェック、 `fontSize` の範囲チェック、
//! `fontName` の path traversal 対策チェック (`canonicalize` + ルート配下確認)、
//! `TextOverlayPatch` の適用ロジックを集約する。
//!
//! ファイル I/O を伴うのは `validate_font_name_and_resolve_path` のみで、
//! それ以外は純粋関数なので PBT で広く境界を検証できる。

use std::path::PathBuf;

use super::{
    TEXT_MAX_BYTES, TEXT_MAX_LINES, TextOverlayConfig, TextOverlayError, TextOverlayPatch,
    TextOverlaySpec,
};

/// `text` と `fontSize` をまとめて検証するヘルパー。
///
/// `text` のバイト数 / 行数と `fontSize` の範囲のみを確認する。
/// `fontColor` は u32 に詰めた時点で必ず妥当 (`0xAARRGGBB` 範囲内) のため別経路で確認する。
/// `fontName` の検証は `TextOverlayLayer::resolve_font_face` 側に集約する。
pub(super) fn validate_text_and_font_size(
    text: &str,
    font_size: u32,
    canvas_height: usize,
) -> Result<(), TextOverlayError> {
    validate_text(text)?;
    validate_font_size(font_size, canvas_height)?;
    Ok(())
}

/// `text` のバイト数と行数を検証する。 pbt クレートから境界値テストするため `pub`。
pub fn validate_text(text: &str) -> Result<(), TextOverlayError> {
    if text.len() > TEXT_MAX_BYTES {
        return Err(TextOverlayError::InvalidText(format!(
            "text exceeds maximum {} bytes (got {})",
            TEXT_MAX_BYTES,
            text.len()
        )));
    }
    // 行数 = 改行数 + 1 (空文字も 1 行扱い、 末尾の `\n` も 1 行を区切る)。
    let lines = text.matches('\n').count() + 1;
    if lines > TEXT_MAX_LINES {
        return Err(TextOverlayError::InvalidText(format!(
            "text exceeds maximum {} lines (got {})",
            TEXT_MAX_LINES, lines
        )));
    }
    Ok(())
}

/// `fontSize` の範囲 (1..=canvas_height) を検証する。 pbt クレートから境界値テストするため `pub`。
pub fn validate_font_size(size: u32, canvas_height: usize) -> Result<(), TextOverlayError> {
    if size == 0 {
        return Err(TextOverlayError::InvalidFontSize(
            "fontSize must be >= 1".to_owned(),
        ));
    }
    if size as usize > canvas_height {
        return Err(TextOverlayError::InvalidFontSize(format!(
            "fontSize {} exceeds canvas_height {}",
            size, canvas_height
        )));
    }
    Ok(())
}

/// `fontName` を path traversal 対策込みで検証して canonical なフォントパスを返す。
///
/// 文字種チェック (`/`, `\`, `..`, NUL の不在) と canonicalize 後の `--font-search-root`
/// 配下チェックのみを行う。 フォントの実体読み込みと parse は呼び出し側 (`TextOverlayLayer::resolve_font_face`)
/// がキャッシュ込みで処理する。
pub(super) fn validate_font_name_and_resolve_path(
    name: &str,
    config: &TextOverlayConfig,
) -> Result<PathBuf, TextOverlayError> {
    if name.is_empty() {
        return Err(TextOverlayError::InvalidFontName(
            "fontName must not be empty".to_owned(),
        ));
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") || name.contains('\0') {
        return Err(TextOverlayError::InvalidFontName(format!(
            "fontName must not contain '/', '\\', '..' or NUL (got: {name:?})"
        )));
    }
    let font_path = config.font_search_root.join(name);
    let canonical = font_path.canonicalize().map_err(|e| {
        TextOverlayError::FontResolveFailed(format!(
            "failed to canonicalize {}: {}",
            font_path.display(),
            e
        ))
    })?;
    if !canonical.starts_with(&config.font_search_root) {
        return Err(TextOverlayError::FontResolveFailed(format!(
            "font path {} escapes search root",
            canonical.display()
        )));
    }
    Ok(canonical)
}

/// `TextOverlayPatch` を既存の `TextOverlaySpec` に適用する純粋関数。
/// 指定されたフィールドだけが更新される (`None` は現状維持)。
pub(super) fn apply_patch(mut spec: TextOverlaySpec, patch: TextOverlayPatch) -> TextOverlaySpec {
    if let Some(text) = patch.text {
        spec.text = text;
    }
    if let Some(x) = patch.x {
        spec.x = x;
    }
    if let Some(y) = patch.y {
        spec.y = y;
    }
    if let Some(size) = patch.font_size {
        spec.font_size = size;
    }
    if let Some(color) = patch.font_color_argb {
        spec.font_color_argb = color;
    }
    if let Some(name) = patch.font_name {
        spec.font_name = name;
    }
    if let Some(z) = patch.z {
        spec.z = z;
    }
    spec
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config() -> TextOverlayConfig {
        TextOverlayConfig::build(
            Some(PathBuf::from("testdata/fonts")),
            Some("PublicSans-Regular.ttf".to_owned()),
        )
        .expect("テスト用 config が組み立てられる")
        .expect("両方指定なので Some")
    }

    /// `validate_font_name_and_resolve_path` が `..` を含む名前を拒否する。
    #[test]
    fn validate_font_name_rejects_dotdot() {
        let config = make_config();
        let err = validate_font_name_and_resolve_path("../etc/passwd", &config)
            .expect_err("`..` を含む名前は拒否される");
        assert!(
            matches!(err, TextOverlayError::InvalidFontName(_)),
            "InvalidFontName が返る: {err:?}"
        );
    }

    /// `/` を含む名前を拒否する。
    #[test]
    fn validate_font_name_rejects_slash() {
        let config = make_config();
        let err = validate_font_name_and_resolve_path("foo/bar.ttf", &config)
            .expect_err("'/' を含む名前は拒否される");
        assert!(
            matches!(err, TextOverlayError::InvalidFontName(_)),
            "InvalidFontName が返る: {err:?}"
        );
    }

    /// 空文字列も拒否する。
    #[test]
    fn validate_font_name_rejects_empty() {
        let config = make_config();
        let err =
            validate_font_name_and_resolve_path("", &config).expect_err("空文字列は拒否される");
        assert!(
            matches!(err, TextOverlayError::InvalidFontName(_)),
            "InvalidFontName が返る: {err:?}"
        );
    }

    /// 実在フォント名は解決成功し、`font_search_root` 配下のパスが返る。
    #[test]
    fn validate_font_name_resolves_real_font() {
        let config = make_config();
        let resolved = validate_font_name_and_resolve_path("PublicSans-Regular.ttf", &config)
            .expect("実在フォントは解決成功");
        assert!(
            resolved.starts_with(&config.font_search_root),
            "解決結果は探索ルート配下に収まる: {:?}",
            resolved
        );
    }

    /// `text` のバイト数上限を超えるとエラー。
    #[test]
    fn validate_text_rejects_too_long() {
        let text = "a".repeat(TEXT_MAX_BYTES + 1);
        let err = validate_text(&text).expect_err("上限超過は拒否される");
        assert!(
            matches!(err, TextOverlayError::InvalidText(_)),
            "InvalidText が返る: {err:?}"
        );
    }

    /// `text` の行数上限を超えるとエラー。
    #[test]
    fn validate_text_rejects_too_many_lines() {
        let text = "\n".repeat(TEXT_MAX_LINES);
        let err = validate_text(&text).expect_err("行数上限超過は拒否される");
        assert!(
            matches!(err, TextOverlayError::InvalidText(_)),
            "InvalidText が返る: {err:?}"
        );
    }

    /// `fontSize = 0` は拒否される。
    #[test]
    fn validate_font_size_rejects_zero() {
        let err = validate_font_size(0, 1080).expect_err("0 は拒否される");
        assert!(
            matches!(err, TextOverlayError::InvalidFontSize(_)),
            "InvalidFontSize が返る: {err:?}"
        );
    }

    /// `fontSize > canvas_height` は拒否される。
    #[test]
    fn validate_font_size_rejects_too_large() {
        let err = validate_font_size(1081, 1080).expect_err("canvas_height 超過は拒否される");
        assert!(
            matches!(err, TextOverlayError::InvalidFontSize(_)),
            "InvalidFontSize が返る: {err:?}"
        );
    }

    /// `apply_patch`: 指定したフィールドだけが更新される。
    #[test]
    fn apply_patch_updates_only_specified_fields() {
        let original = TextOverlaySpec {
            text: "before".to_owned(),
            x: 10,
            y: 20,
            font_size: 30,
            font_color_argb: 0xFFFFFFFF,
            font_name: "PublicSans-Regular.ttf".to_owned(),
            z: 0,
        };
        let patch = TextOverlayPatch {
            text: Some("after".to_owned()),
            x: Some(100),
            ..Default::default()
        };
        let updated = apply_patch(original, patch);
        assert_eq!(updated.text, "after", "text は更新される");
        assert_eq!(updated.x, 100, "x は更新される");
        assert_eq!(updated.y, 20, "y は維持される");
        assert_eq!(updated.font_size, 30, "font_size は維持される");
    }
}
