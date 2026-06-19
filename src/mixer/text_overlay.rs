use std::path::PathBuf;

/// テキストオーバーレイ機能の起動時設定。
///
/// CLI 引数 `--font-search-root` と `--default-font` の両方が指定された場合のみ
/// `Some` となり、テキストオーバーレイ機能が有効になる。
/// 両方とも未指定の場合は `None` で、機能は無効として扱う (obsws の
/// `HisuiCreateTextOverlay` 等は `RESOURCE_ACTION_NOT_SUPPORTED` を返す予定)。
/// 片方のみ指定された場合は起動時にエラーとする。
#[derive(Debug, Clone)]
pub struct TextOverlayConfig {
    /// canonicalize 済みのフォント探索ルート (絶対パス)。
    ///
    /// 実際のフォントファイル参照時は `<font_search_root>/<font_name>` を canonicalize した
    /// 結果が、この root の prefix を持つことを必ず確認する (path traversal 対策)。
    pub font_search_root: PathBuf,

    /// `HisuiCreateTextOverlay` で `fontName` が省略された場合に使う既定フォント名。
    ///
    /// `<font_search_root>/<default_font_name>` が起動時に解決・読み込み可能であることを
    /// `TextOverlayConfig::build` で検証済み。
    pub default_font_name: String,
}

impl TextOverlayConfig {
    /// CLI 引数から `TextOverlayConfig` を構築する。
    ///
    /// - 両方 `None`: `Ok(None)` (機能無効として正常起動)
    /// - 片方のみ `Some`: `Err` (CLI 引数の組として不整合のため起動失敗)
    /// - 両方 `Some`: 起動時検証 (canonicalize、root 内チェック、raden での読み込み試行) を
    ///   経て `Ok(Some(...))` を返す。検証失敗時は `Err`
    pub fn build(
        font_search_root: Option<PathBuf>,
        default_font_name: Option<String>,
    ) -> Result<Option<Self>, String> {
        match (font_search_root, default_font_name) {
            (None, None) => Ok(None),
            (Some(_), None) => Err("--font-search-root requires --default-font".to_owned()),
            (None, Some(_)) => Err("--default-font requires --font-search-root".to_owned()),
            (Some(root), Some(name)) => {
                let canonical_root = root.canonicalize().map_err(|e| {
                    format!(
                        "failed to canonicalize --font-search-root {}: {}",
                        root.display(),
                        e
                    )
                })?;
                let font_path = canonical_root.join(&name);
                let canonical_font = font_path.canonicalize().map_err(|e| {
                    format!(
                        "failed to canonicalize default font {}: {}",
                        font_path.display(),
                        e
                    )
                })?;
                if !canonical_font.starts_with(&canonical_root) {
                    return Err(format!(
                        "default font {} escapes --font-search-root {}",
                        canonical_font.display(),
                        canonical_root.display()
                    ));
                }
                let path_str = canonical_font.to_str().ok_or_else(|| {
                    format!(
                        "default font path {} is not valid UTF-8",
                        canonical_font.display()
                    )
                })?;
                let font_data = raden::FontData::from_file(path_str).map_err(|e| {
                    format!(
                        "failed to load default font {}: {:?}",
                        canonical_font.display(),
                        e
                    )
                })?;
                raden::FontFace::from_data(&font_data, 0).map_err(|e| {
                    format!(
                        "failed to parse default font {}: {:?}",
                        canonical_font.display(),
                        e
                    )
                })?;
                Ok(Some(Self {
                    font_search_root: canonical_root,
                    default_font_name: name,
                }))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 両方未指定なら機能無効 (Ok(None))。
    #[test]
    fn build_returns_none_when_both_unspecified() {
        let result = TextOverlayConfig::build(None, None).expect("両方未指定は正常起動");
        assert!(result.is_none(), "両方未指定では機能無効を表す None になる");
    }

    /// font-search-root のみ指定はエラー。
    #[test]
    fn build_returns_err_when_only_root_specified() {
        let err = TextOverlayConfig::build(Some(PathBuf::from("/")), None)
            .expect_err("片方のみ指定はエラーになる");
        assert!(
            err.contains("--default-font"),
            "エラー文言で --default-font 不足を伝える: {err}"
        );
    }

    /// default-font のみ指定はエラー。
    #[test]
    fn build_returns_err_when_only_default_font_specified() {
        let err = TextOverlayConfig::build(None, Some("foo.ttf".to_owned()))
            .expect_err("片方のみ指定はエラーになる");
        assert!(
            err.contains("--font-search-root"),
            "エラー文言で --font-search-root 不足を伝える: {err}"
        );
    }

    /// 両方指定で実在フォントなら Ok(Some(...))。
    #[test]
    fn build_succeeds_with_real_font_in_testdata() {
        let root = PathBuf::from("testdata/fonts");
        let config = TextOverlayConfig::build(
            Some(root.clone()),
            Some("PublicSans-Regular.ttf".to_owned()),
        )
        .expect("testdata の Public Sans Regular は解決・読み込み可能")
        .expect("両方指定なら Some が返る");
        assert!(
            config.font_search_root.is_absolute(),
            "canonicalize 後は絶対パスになる: {:?}",
            config.font_search_root
        );
        assert_eq!(config.default_font_name, "PublicSans-Regular.ttf");
    }

    /// 探索ルート配下に存在しないフォント名を指定するとエラー。
    #[test]
    fn build_returns_err_when_default_font_does_not_exist() {
        let err = TextOverlayConfig::build(
            Some(PathBuf::from("testdata/fonts")),
            Some("nonexistent-font.ttf".to_owned()),
        )
        .expect_err("存在しないフォントはエラー");
        assert!(
            err.contains("nonexistent-font"),
            "エラー文言にフォント名が含まれる: {err}"
        );
    }
}
