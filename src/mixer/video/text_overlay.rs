//! `VideoRealtimeMixer` の内部レイヤとして組み込まれるテキストオーバーレイ機能。
//!
//! 本モジュールは公開 API (Config / Error / 各種 Spec / 定数) を集約する。
//! `TextOverlayLayer` 本体は `layer` サブモジュールに、 検証関数は `validate`
//! サブモジュールに分離している。
//!
//! `VideoRealtimeMixer` 構築時に layer が生成され、 `compose_frame` の追加合成段で
//! `RealtimeI420Canvas::draw_frame_clipped` 経由で最上位レイヤとして合成される。

use std::path::PathBuf;

use tokio::sync::oneshot;

pub mod layer;
pub mod validate;

pub use self::layer::TextOverlayLayer;

/// テキストオーバーレイ機能の起動時設定。
///
/// フォント探索ルートと省略時フォント名を保持する。
#[derive(Debug, Clone)]
pub struct TextOverlayConfig {
    /// canonicalize 済みのフォント探索ルート (絶対パス)。
    ///
    /// 実際のフォントファイル参照時は `<font_search_root>/<font_name>` を canonicalize した
    /// 結果が、 この root の prefix を持つことを必ず確認する (path traversal 対策)。
    pub font_search_root: PathBuf,

    /// `fontName` が省略された場合に使う既定フォント名。
    ///
    /// `<font_search_root>/<default_font_name>` が `TextOverlayConfig::new` で
    /// 解決・読み込み可能であることを検証済み。
    pub default_font_name: String,
}

impl TextOverlayConfig {
    /// フォント探索ルートとデフォルトフォント名から起動時設定を構築する。
    ///
    /// canonicalize / path traversal 検証 / raden での読み込み試行を行う。
    pub fn new(font_search_root: PathBuf, default_font_name: String) -> Result<Self, crate::Error> {
        let canonical_root = font_search_root.canonicalize().map_err(|e| {
            crate::Error::new(format!(
                "failed to canonicalize font search root {}: {}",
                font_search_root.display(),
                e
            ))
        })?;
        let font_path = canonical_root.join(&default_font_name);
        let canonical_font = font_path.canonicalize().map_err(|e| {
            crate::Error::new(format!(
                "failed to canonicalize default font {}: {}",
                font_path.display(),
                e
            ))
        })?;
        if !canonical_font.starts_with(&canonical_root) {
            return Err(crate::Error::new(format!(
                "default font {} escapes font search root {}",
                canonical_font.display(),
                canonical_root.display()
            )));
        }
        // NOTE: raden::FontData::from_file が将来的に &Path を取れるようになれば
        //       (shiguredo/raden issue 0041)、 ここの to_str() 変換は不要になる。
        let path_str = canonical_font.to_str().ok_or_else(|| {
            crate::Error::new(format!(
                "default font path {} is not valid UTF-8",
                canonical_font.display()
            ))
        })?;
        let font_data = raden::FontData::from_file(path_str).map_err(|e| {
            crate::Error::new(format!(
                "failed to load default font {}: {e:?}",
                canonical_font.display()
            ))
        })?;
        raden::FontFace::from_data(&font_data, 0).map_err(|e| {
            crate::Error::new(format!(
                "failed to parse default font {}: {e:?}",
                canonical_font.display()
            ))
        })?;
        Ok(Self {
            font_search_root: canonical_root,
            default_font_name,
        })
    }
}

/// 1 mixer が同時に保持できるテキストオーバーレイの最大数。
pub const OVERLAY_LIMIT: usize = 1024;

/// `text` フィールドの最大バイト数。
pub const TEXT_MAX_BYTES: usize = 65536;

/// `text` フィールドの最大行数 (`\n` 区切り)。
pub const TEXT_MAX_LINES: usize = 1024;

/// テキストオーバーレイの仕様 (確定値)。
///
/// `font_color_argb` は `0xAARRGGBB` の straight alpha 値。 default (`HisuiCreateTextOverlay` で
/// 省略時) は `0xFFFFFFFF` (不透明白)。
/// `z` は確定値 (`TextOverlaySpecInput::z = None` の場合はレイヤが宣言順から確定する)。
#[derive(Debug, Clone)]
pub struct TextOverlaySpec {
    pub text: String,
    pub x: i64,
    pub y: i64,
    pub font_size: u32,
    pub font_color_argb: u32,
    pub font_name: String,
    pub z: i32,
}

/// `HisuiCreateTextOverlay` の入力 (確定前)。
///
/// `z = None` の場合はレイヤが「現在の最大 z + 1」を割り当てる (= 宣言順、 後勝ち)。
#[derive(Debug, Clone)]
pub struct TextOverlaySpecInput {
    pub text: String,
    pub x: i64,
    pub y: i64,
    pub font_size: u32,
    pub font_color_argb: u32,
    pub font_name: String,
    pub z: Option<i32>,
}

/// `HisuiUpdateTextOverlay` 用の部分更新パッチ。 `None` は省略 (= 現状維持)。
#[derive(Debug, Clone, Default)]
pub struct TextOverlayPatch {
    pub text: Option<String>,
    pub x: Option<i64>,
    pub y: Option<i64>,
    pub font_size: Option<u32>,
    pub font_color_argb: Option<u32>,
    pub font_name: Option<String>,
    pub z: Option<i32>,
}

/// `HisuiListTextOverlays` で返す現在状態。
#[derive(Debug, Clone)]
pub struct TextOverlayState {
    pub name: String,
    pub spec: TextOverlaySpec,
}

/// テキストオーバーレイ操作のエラー。 obsws ハンドラ側で `REQUEST_STATUS_*` にマップする。
#[derive(Debug, Clone)]
pub enum TextOverlayError {
    /// 同名 overlay が既に存在する (Create のみ)。
    AlreadyExists,
    /// 対象 overlay が存在しない (Update / Remove)。
    NotFound,
    /// `fontName` が `/` `\` `..` NUL バイトを含む等の文字種違反。
    InvalidFontName(String),
    /// `fontName` の解決失敗 (ファイルなし / ルート外 / フォント破損)。
    FontResolveFailed(String),
    /// `fontColor` の形式違反。
    InvalidColor(String),
    /// `fontSize` の範囲外。
    InvalidFontSize(String),
    /// `text` のバイト数 / 行数上限超過。
    InvalidText(String),
    /// `OVERLAY_LIMIT` 超過。
    LimitExceeded,
}

impl std::fmt::Display for TextOverlayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // obsws ハンドラから `error.to_string()` 経由でクライアント向け文言として
        // 使われるため、 フィールド名 (fontName / fontColor 等) を含む形で書く。
        match self {
            Self::AlreadyExists => write!(f, "text overlay already exists"),
            Self::NotFound => write!(f, "text overlay not found"),
            Self::InvalidFontName(s) => write!(f, "invalid fontName: {s}"),
            Self::FontResolveFailed(s) => write!(f, "fontName resolve failed: {s}"),
            Self::InvalidColor(s) => write!(f, "invalid fontColor: {s}"),
            Self::InvalidFontSize(s) => write!(f, "invalid fontSize: {s}"),
            Self::InvalidText(s) => write!(f, "invalid text: {s}"),
            Self::LimitExceeded => write!(f, "text overlay limit exceeded"),
        }
    }
}

impl std::error::Error for TextOverlayError {}

/// `VideoRealtimeMixer` のテキストオーバーレイ系 RPC バリアントに渡される追加メッセージ群。
///
/// `register_rpc_sender` は同一 processor で 1 sender しか登録できない (`media_pipeline.rs`)
/// ため、 既存の `VideoRealtimeMixerRpcMessage` enum 内のバリアントとして統合する。
/// ここで使われるバリアントは `VideoRealtimeMixerRpcMessage::TextOverlayAdd` 等。
///
/// reply 型はそれぞれ `Result<T, TextOverlayError>` (Add/Update/Remove は `T = ()`、
/// List は `T = Vec<TextOverlayState>`) で、 obsws ハンドラが reply を待ち受けてマップする。
#[derive(Debug)]
pub enum TextOverlayCommand {
    Add {
        name: String,
        input: TextOverlaySpecInput,
        reply_tx: oneshot::Sender<Result<(), TextOverlayError>>,
    },
    Update {
        name: String,
        patch: TextOverlayPatch,
        reply_tx: oneshot::Sender<Result<(), TextOverlayError>>,
    },
    Remove {
        name: String,
        reply_tx: oneshot::Sender<Result<(), TextOverlayError>>,
    },
    List {
        reply_tx: oneshot::Sender<Vec<TextOverlayState>>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 実在フォントを渡すと canonicalize 済みの設定が得られる。
    #[test]
    fn new_succeeds_with_real_font_in_testdata() {
        let config = TextOverlayConfig::new(
            PathBuf::from("testdata/fonts"),
            "PublicSans-Regular.ttf".to_owned(),
        )
        .expect("testdata の Public Sans Regular は解決・読み込み可能");
        assert!(
            config.font_search_root.is_absolute(),
            "canonicalize 後は絶対パスになる: {:?}",
            config.font_search_root
        );
        assert_eq!(config.default_font_name, "PublicSans-Regular.ttf");
    }

    /// 探索ルート配下に存在しないフォント名を渡すとエラー。
    #[test]
    fn new_returns_err_when_default_font_does_not_exist() {
        let err = TextOverlayConfig::new(
            PathBuf::from("testdata/fonts"),
            "nonexistent-font.ttf".to_owned(),
        )
        .expect_err("存在しないフォントはエラー");
        assert!(
            format!("{err:?}").contains("nonexistent-font"),
            "エラー文言にフォント名が含まれる: {err:?}"
        );
    }
}
