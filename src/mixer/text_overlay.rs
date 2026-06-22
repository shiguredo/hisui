use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::oneshot;

use crate::media_pipeline::ProcessorHandle;
use crate::types::EvenUsize;
use crate::video::{FrameRate, VideoFormat, VideoFrame, VideoFrameSize};
use crate::{MediaFrame, TrackId};

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

/// ACK/SYN back-pressure の閾値。`VideoRealtimeMixerRunner` / `ColorSource` と同じ値。
const MAX_NOACKED_COUNT: u64 = 100;

/// `TextOverlayProcessor` の出力 TrackId 文字列 (常駐インスタンスのため固定)。
pub const TEXT_OVERLAY_TRACK_ID: &str = "program:text_overlay";

/// `TextOverlayProcessor` の ProcessorId 文字列 (常駐インスタンスのため固定)。
pub const TEXT_OVERLAY_PROCESSOR_ID: &str = "program:text_overlay_processor";

/// 1 processor が同時に保持できるテキストオーバーレイの最大数 (DoS 対策の上限)。
pub const OVERLAY_LIMIT: usize = 64;

/// `text` フィールドの最大バイト数。
pub const TEXT_MAX_BYTES: usize = 4096;

/// `text` フィールドの最大行数 (`\n` 区切り)。
pub const TEXT_MAX_LINES: usize = 64;

/// テキストオーバーレイの仕様 (確定値)。
///
/// `font_color_argb` は `0xAARRGGBB` の straight alpha 値。default (`HisuiCreateTextOverlay` で
/// 省略時) は `0xFFFFFFFF` (不透明白)。
/// `z` は確定値 (`TextOverlaySpecInput::z = None` の場合は Processor が宣言順から確定する)。
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
/// `z = None` の場合は Processor が「現在の最大 z + 1」を割り当てる (= 宣言順、後勝ち)。
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

/// `HisuiUpdateTextOverlay` 用の部分更新パッチ。`None` は省略 (= 現状維持)。
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

/// テキストオーバーレイ操作のエラー。obsws ハンドラ側で `REQUEST_STATUS_*` にマップする。
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
    /// raden 描画失敗。
    RenderFailed(String),
    /// `OVERLAY_LIMIT` 超過。
    LimitExceeded,
}

impl std::fmt::Display for TextOverlayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AlreadyExists => write!(f, "text overlay already exists"),
            Self::NotFound => write!(f, "text overlay not found"),
            Self::InvalidFontName(s) => write!(f, "invalid fontName: {s}"),
            Self::FontResolveFailed(s) => write!(f, "font resolve failed: {s}"),
            Self::InvalidColor(s) => write!(f, "invalid fontColor: {s}"),
            Self::InvalidFontSize(s) => write!(f, "invalid fontSize: {s}"),
            Self::InvalidText(s) => write!(f, "invalid text: {s}"),
            Self::RenderFailed(s) => write!(f, "render failed: {s}"),
            Self::LimitExceeded => write!(f, "text overlay limit exceeded"),
        }
    }
}

impl std::error::Error for TextOverlayError {}

/// `TextOverlayProcessor` に対する内部 RPC メッセージ。
///
/// `register_rpc_sender` パターン (`VideoRealtimeMixer` 参考) で送受信する。
#[derive(Debug)]
pub enum TextOverlayRpcMessage {
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

/// テキストオーバーレイ描画用の processor。
///
/// 1 processor インスタンスが canvas 全体に対する全テキストオーバーレイを描画し、
/// 1 本の透過 I420A track として publish する。
/// canvas サイズ・フレームレートは起動時固定 (`new` 時に確定)、シーン切替で再生成されない。
pub struct TextOverlayProcessor {
    canvas_width: EvenUsize,
    canvas_height: EvenUsize,
    frame_rate: FrameRate,
    config: TextOverlayConfig,
}

impl TextOverlayProcessor {
    pub fn new(
        canvas_width: EvenUsize,
        canvas_height: EvenUsize,
        frame_rate: FrameRate,
        config: TextOverlayConfig,
    ) -> Self {
        Self {
            canvas_width,
            canvas_height,
            frame_rate,
            config,
        }
    }

    /// run ループ。
    ///
    /// TextOverlayProcessor は初期 processor 集合に含まれず、後発の subscriber を
    /// 前提とするため `wait_subscribers_ready` は呼ばない。
    pub async fn run(self, handle: ProcessorHandle) -> crate::Result<()> {
        let canvas_width = self.canvas_width;
        let canvas_height = self.canvas_height;
        let frame_rate = self.frame_rate;
        let config = self.config;

        // 内部 RPC 受信チャネル
        let (rpc_tx, mut rpc_rx) = tokio::sync::mpsc::unbounded_channel::<TextOverlayRpcMessage>();
        handle.register_rpc_sender(rpc_tx).await.map_err(|e| {
            crate::Error::new(format!("failed to register text overlay rpc sender: {e}"))
        })?;

        let mut tx = handle
            .publish_track(TrackId::new(TEXT_OVERLAY_TRACK_ID))
            .await?;
        handle.notify_ready();

        let mut state = ProcessorState::new(canvas_width, canvas_height, config);
        let mut frame_index = 0u64;
        let mut noacked_sent = 0u64;
        let start = tokio::time::Instant::now();
        let mut ack = Some(tx.send_syn());

        loop {
            let timestamp = super::video::frames_to_timestamp(frame_rate, frame_index);
            tokio::select! {
                _ = tokio::time::sleep_until(start + timestamp) => {
                    if noacked_sent > MAX_NOACKED_COUNT {
                        if let Some(a) = ack.take() {
                            a.await;
                        }
                        ack = Some(tx.send_syn());
                        noacked_sent = 0;
                    }

                    // overlay が 1 つも無い間はフレーム送信をスキップする。
                    // mixer 側は pending_frames 空の InputTrack を合成に含めないため、
                    // 結果として透過レイヤがそのまま素通りする。
                    if !state.has_overlays() {
                        frame_index = frame_index.saturating_add(1);
                        continue;
                    }

                    let frame = state.frame_for(timestamp)?;
                    if !tx.send_media(MediaFrame::Video(frame)) {
                        break;
                    }
                    noacked_sent = noacked_sent.saturating_add(1);
                    frame_index = frame_index.saturating_add(1);
                }
                msg = rpc_rx.recv() => {
                    let Some(msg) = msg else {
                        // RPC 送信側が全て drop された = 通常はこのケースには到達しない。
                        // ただし test 等で発生しうるので break する。
                        break;
                    };
                    state.handle_rpc(msg);
                }
            }
        }

        Ok(())
    }
}

/// `TextOverlayProcessor` の run ループ内で更新される実体状態。
struct ProcessorState {
    canvas_width: EvenUsize,
    canvas_height: EvenUsize,
    config: TextOverlayConfig,
    overlays: BTreeMap<String, TextOverlaySpec>,
    cached_frame: Option<Arc<VideoFrame>>,
    dirty: bool,
    /// 入力 `z = None` (宣言順) を解決する際に使う次の自動 z。
    /// `z = Some(v)` を受けた場合は `next_auto_z = max(next_auto_z, v + 1)` に更新する。
    next_auto_z: i32,
}

impl ProcessorState {
    fn new(canvas_width: EvenUsize, canvas_height: EvenUsize, config: TextOverlayConfig) -> Self {
        Self {
            canvas_width,
            canvas_height,
            config,
            overlays: BTreeMap::new(),
            cached_frame: None,
            dirty: false,
            next_auto_z: 0,
        }
    }

    fn has_overlays(&self) -> bool {
        !self.overlays.is_empty()
    }

    fn handle_rpc(&mut self, msg: TextOverlayRpcMessage) {
        match msg {
            TextOverlayRpcMessage::Add {
                name,
                input,
                reply_tx,
            } => {
                let _ = reply_tx.send(self.add(name, input));
            }
            TextOverlayRpcMessage::Update {
                name,
                patch,
                reply_tx,
            } => {
                let _ = reply_tx.send(self.update(name, patch));
            }
            TextOverlayRpcMessage::Remove { name, reply_tx } => {
                let _ = reply_tx.send(self.remove(name));
            }
            TextOverlayRpcMessage::List { reply_tx } => {
                let _ = reply_tx.send(self.list());
            }
        }
    }

    fn add(&mut self, name: String, input: TextOverlaySpecInput) -> Result<(), TextOverlayError> {
        if self.overlays.contains_key(&name) {
            return Err(TextOverlayError::AlreadyExists);
        }
        if self.overlays.len() >= OVERLAY_LIMIT {
            return Err(TextOverlayError::LimitExceeded);
        }
        validate_spec_fields(
            &input.text,
            input.font_size,
            &input.font_name,
            &self.config,
            self.canvas_height.get(),
        )?;
        let resolved_z = match input.z {
            Some(z) => {
                self.next_auto_z = self.next_auto_z.max(z.saturating_add(1));
                z
            }
            None => {
                let z = self.next_auto_z;
                self.next_auto_z = self.next_auto_z.saturating_add(1);
                z
            }
        };
        let spec = TextOverlaySpec {
            text: input.text,
            x: input.x,
            y: input.y,
            font_size: input.font_size,
            font_color_argb: input.font_color_argb,
            font_name: input.font_name,
            z: resolved_z,
        };
        self.overlays.insert(name, spec);
        self.dirty = true;
        Ok(())
    }

    fn update(&mut self, name: String, patch: TextOverlayPatch) -> Result<(), TextOverlayError> {
        let existing = self
            .overlays
            .get(&name)
            .ok_or(TextOverlayError::NotFound)?
            .clone();
        let updated = apply_patch(existing, patch);
        validate_spec_fields(
            &updated.text,
            updated.font_size,
            &updated.font_name,
            &self.config,
            self.canvas_height.get(),
        )?;
        // 明示 z 更新で next_auto_z を後方互換に保つ
        self.next_auto_z = self.next_auto_z.max(updated.z.saturating_add(1));
        self.overlays.insert(name, updated);
        self.dirty = true;
        Ok(())
    }

    fn remove(&mut self, name: String) -> Result<(), TextOverlayError> {
        if self.overlays.remove(&name).is_none() {
            return Err(TextOverlayError::NotFound);
        }
        self.dirty = true;
        Ok(())
    }

    fn list(&self) -> Vec<TextOverlayState> {
        self.overlays
            .iter()
            .map(|(name, spec)| TextOverlayState {
                name: name.clone(),
                spec: spec.clone(),
            })
            .collect()
    }

    /// 指定 timestamp の I420A フレームを返す (cached が valid ならそれをタイムスタンプだけ差し替えて返す)。
    fn frame_for(&mut self, timestamp: Duration) -> crate::Result<Arc<VideoFrame>> {
        if self.dirty || self.cached_frame.is_none() {
            let new_frame = self.render(timestamp)?;
            self.cached_frame = Some(Arc::new(new_frame));
            self.dirty = false;
            return Ok(self
                .cached_frame
                .as_ref()
                .expect("cached_frame was just assigned")
                .clone());
        }
        // タイムスタンプだけ差し替えた版を返す (data は Vec<u8> なので clone される)。
        let cached = self
            .cached_frame
            .as_ref()
            .expect("cached_frame is Some when dirty is false");
        let mut frame = (**cached).clone();
        frame.timestamp = timestamp;
        Ok(Arc::new(frame))
    }

    /// raden + libyuv で 1 枚の I420A フレームを構築する。
    fn render(&self, timestamp: Duration) -> crate::Result<VideoFrame> {
        let w = self.canvas_width.get();
        let h = self.canvas_height.get();

        // raden は PixelFormat::Prgb32 = premultiplied ARGB を出力する。
        // バイト並びはリトルエンディアン環境で [B, G, R, A] となり、libyuv の ArgbImage と整合する。
        let mut image = raden::Image::new(w as u32, h as u32, raden::PixelFormat::Prgb32);
        let mut runtime = raden::PipelineRuntime::new();
        {
            let mut ctx = raden::Context::new(&mut image, &mut runtime);

            // canvas を完全透明 (RGBA = 0x00000000) でクリアする。
            ctx.set_comp_op(raden::CompOp::SrcCopy);
            ctx.set_fill_style(raden::Rgba32::new(0, 0, 0, 0));
            ctx.fill_rect(&raden::Rect::new(0.0, 0.0, w as f64, h as f64));
            ctx.set_comp_op(raden::CompOp::SrcOver);

            // z-order でソート (タイブレークは name のアルファベット順 = BTreeMap の iter 順)。
            let mut sorted: Vec<(&String, &TextOverlaySpec)> = self.overlays.iter().collect();
            sorted.sort_by(|(name_a, spec_a), (name_b, spec_b)| {
                spec_a.z.cmp(&spec_b.z).then(name_a.cmp(name_b))
            });

            for (_, spec) in sorted {
                let canonical = validate_font_name_and_resolve(&spec.font_name, &self.config)
                    .map_err(|e| crate::Error::new(format!("{e}")))?;
                let path_str = canonical
                    .to_str()
                    .ok_or_else(|| crate::Error::new("font path not utf-8".to_owned()))?;
                let font_data = raden::FontData::from_file(path_str)
                    .map_err(|e| crate::Error::new(format!("load font: {e:?}")))?;
                let face = raden::FontFace::from_data(&font_data, 0)
                    .map_err(|e| crate::Error::new(format!("parse font: {e:?}")))?;
                let font = raden::Font::from_face(&face, spec.font_size as f64);

                let (a, r, g, b) = (
                    ((spec.font_color_argb >> 24) & 0xFF) as u8,
                    ((spec.font_color_argb >> 16) & 0xFF) as u8,
                    ((spec.font_color_argb >> 8) & 0xFF) as u8,
                    (spec.font_color_argb & 0xFF) as u8,
                );
                ctx.set_fill_style(raden::Rgba32::new(r, g, b, a));
                // raden の fill_text はベースライン座標で描画するので ascent 分下げる。
                let baseline_y = spec.y as f64 + font.ascent();
                ctx.fill_text(spec.x as f64, baseline_y, &font, &spec.text);
            }

            ctx.end();
        }

        // raden 出力は premultiplied なので、libyuv に渡す前に straight alpha に戻す。
        let mut argb = image.data().to_vec();
        unpremultiply_argb(&mut argb);

        // libyuv で I420 + Alpha に変換する。
        let y_size = w * h;
        let uv_w = w.div_ceil(2);
        let uv_h = h.div_ceil(2);
        let uv_size = uv_w * uv_h;
        let mut y_plane = vec![0u8; y_size];
        let mut u_plane = vec![0u8; uv_size];
        let mut v_plane = vec![0u8; uv_size];
        let mut alpha = vec![0u8; y_size];

        let argb_image = shiguredo_libyuv::ArgbImage {
            data: &argb,
            stride: w * 4,
        };
        let mut i420 = shiguredo_libyuv::I420ImageMut {
            y: &mut y_plane,
            y_stride: w,
            u: &mut u_plane,
            u_stride: uv_w,
            v: &mut v_plane,
            v_stride: uv_w,
        };
        shiguredo_libyuv::argb_to_i420_alpha(
            &argb_image,
            &mut i420,
            &mut alpha,
            w,
            shiguredo_libyuv::ImageSize::new(w, h),
        )
        .map_err(|e| crate::Error::new(format!("argb_to_i420_alpha: {e}")))?;

        // I420A レイアウト: [Y | U | V | A] の連続バッファ。
        let mut data = Vec::with_capacity(y_size + uv_size * 2 + y_size);
        data.extend_from_slice(&y_plane);
        data.extend_from_slice(&u_plane);
        data.extend_from_slice(&v_plane);
        data.extend_from_slice(&alpha);

        Ok(VideoFrame {
            data,
            format: VideoFormat::I420A,
            keyframe: true,
            size: Some(VideoFrameSize {
                width: w,
                height: h,
            }),
            timestamp,
            sample_entry: None,
        })
    }
}

/// premultiplied ARGB バッファを straight alpha 形式に戻す。
///
/// 各ピクセルは 4 バイト (リトルエンディアン環境では `[B, G, R, A]`) で並ぶ。
/// A == 0 のピクセルは RGB が 0 になっているのでそのまま残す。
/// A > 0 のピクセルは `RGB_straight = (RGB_pre * 255 + A/2) / A` で復元する。
///
/// raden の `Prgb32` (u32 = 0xAARRGGBB) のバイト順とこの関数の `chunk[3] = A`
/// 取り出しはリトルエンディアン前提なので、ビッグエンディアン環境では
/// 別経路が必要。コンパイル時に検出して落とす。
const _: () = assert!(
    cfg!(target_endian = "little"),
    "text overlay rendering assumes little-endian (raden Prgb32 layout)",
);
fn unpremultiply_argb(data: &mut [u8]) {
    for chunk in data.chunks_exact_mut(4) {
        let a = chunk[3];
        if a == 0 {
            continue;
        }
        let a_u16 = a as u16;
        for c in &mut chunk[..3] {
            // 四捨五入用に A/2 を足してから A で割る。255 でクランプする。
            let value = ((*c as u16) * 255 + a_u16 / 2) / a_u16;
            *c = value.min(255) as u8;
        }
    }
}

fn validate_spec_fields(
    text: &str,
    font_size: u32,
    font_name: &str,
    config: &TextOverlayConfig,
    canvas_height: usize,
) -> Result<(), TextOverlayError> {
    validate_text(text)?;
    validate_font_size(font_size, canvas_height)?;
    validate_font_name_and_resolve(font_name, config)?;
    // fontColor は u32 に詰める時点で必ず妥当 (`0xAARRGGBB` 範囲内) なので、ここでの追加検証は不要。
    // 値域チェックは obsws ハンドラの正規表現マッチで担保する。
    Ok(())
}

fn validate_text(text: &str) -> Result<(), TextOverlayError> {
    if text.len() > TEXT_MAX_BYTES {
        return Err(TextOverlayError::InvalidText(format!(
            "text exceeds maximum {} bytes (got {})",
            TEXT_MAX_BYTES,
            text.len()
        )));
    }
    let lines = text.matches('\n').count() + 1;
    if lines > TEXT_MAX_LINES {
        return Err(TextOverlayError::InvalidText(format!(
            "text exceeds maximum {} lines (got {})",
            TEXT_MAX_LINES, lines
        )));
    }
    Ok(())
}

fn validate_font_size(size: u32, canvas_height: usize) -> Result<(), TextOverlayError> {
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

/// `fontName` を path traversal 対策込みで検証してフォントファイルの canonical パスを返す。
fn validate_font_name_and_resolve(
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
    let path_str = canonical
        .to_str()
        .ok_or_else(|| TextOverlayError::FontResolveFailed("font path not utf-8".to_owned()))?;
    let font_data = raden::FontData::from_file(path_str)
        .map_err(|e| TextOverlayError::FontResolveFailed(format!("load font: {e:?}")))?;
    raden::FontFace::from_data(&font_data, 0)
        .map_err(|e| TextOverlayError::FontResolveFailed(format!("parse font: {e:?}")))?;
    Ok(canonical)
}

fn apply_patch(mut spec: TextOverlaySpec, patch: TextOverlayPatch) -> TextOverlaySpec {
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

    fn make_config() -> TextOverlayConfig {
        TextOverlayConfig::build(
            Some(PathBuf::from("testdata/fonts")),
            Some("PublicSans-Regular.ttf".to_owned()),
        )
        .expect("テスト用 config が組み立てられる")
        .expect("両方指定なので Some")
    }

    /// `validate_font_name_and_resolve` が `..` を含む名前を拒否する。
    #[test]
    fn validate_font_name_rejects_dotdot() {
        let config = make_config();
        let err = validate_font_name_and_resolve("../etc/passwd", &config)
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
        let err = validate_font_name_and_resolve("foo/bar.ttf", &config)
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
        let err = validate_font_name_and_resolve("", &config).expect_err("空文字列は拒否される");
        assert!(
            matches!(err, TextOverlayError::InvalidFontName(_)),
            "InvalidFontName が返る: {err:?}"
        );
    }

    /// 実在フォント名は解決成功し、`font_search_root` 配下のパスが返る。
    #[test]
    fn validate_font_name_resolves_real_font() {
        let config = make_config();
        let resolved = validate_font_name_and_resolve("PublicSans-Regular.ttf", &config)
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
        let text = "\n".repeat(TEXT_MAX_LINES); // 行数 = 改行数 + 1 = LIMIT + 1
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

    /// `unpremultiply_argb` の挙動: A == 0 は触らない、A == 255 は変化なし、A == 128 は約 2 倍。
    #[test]
    fn unpremultiply_argb_handles_basic_cases() {
        let mut data = vec![
            // pixel 0: 完全透明 - そのまま残る
            0, 0, 0, 0, // pixel 1: 不透明、白
            255, 255, 255,
            255, // pixel 2: 半透明、premultiplied 値が 128 (= straight 255 * 128/255)
            128, 128, 128, 128,
        ];
        unpremultiply_argb(&mut data);
        // pixel 0: 透明はそのまま 0
        assert_eq!(&data[0..4], &[0, 0, 0, 0]);
        // pixel 1: A=255 は値変化なし (255 * 255 / 255 = 255)
        assert_eq!(&data[4..8], &[255, 255, 255, 255]);
        // pixel 2: A=128 で premultiplied 128 → straight 約 255
        // 計算: (128 * 255 + 64) / 128 = (32640 + 64) / 128 = 32704 / 128 = 255
        assert_eq!(&data[8..12], &[255, 255, 255, 128]);
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

    // ProcessorState の内部ロジックテスト

    fn make_state() -> ProcessorState {
        let canvas_width = EvenUsize::new(1920).expect("1920 は偶数");
        let canvas_height = EvenUsize::new(1080).expect("1080 は偶数");
        let config = make_config();
        ProcessorState::new(canvas_width, canvas_height, config)
    }

    fn make_input() -> TextOverlaySpecInput {
        TextOverlaySpecInput {
            text: "hello".to_owned(),
            x: 10,
            y: 20,
            font_size: 32,
            font_color_argb: 0xFFFFFFFF,
            font_name: "PublicSans-Regular.ttf".to_owned(),
            z: None,
        }
    }

    /// add: overlay が挿入され、dirty フラグが立つ。
    #[test]
    fn processor_state_add_inserts_overlay_and_marks_dirty() {
        let mut state = make_state();
        state
            .add("greeting".to_owned(), make_input())
            .expect("add 成功");
        assert!(
            state.overlays.contains_key("greeting"),
            "overlays に挿入される"
        );
        assert!(state.dirty, "dirty フラグが立つ");
    }

    /// add: 同名 overlay は AlreadyExists で拒否。
    #[test]
    fn processor_state_add_rejects_duplicate() {
        let mut state = make_state();
        state
            .add("greeting".to_owned(), make_input())
            .expect("初回 add 成功");
        let err = state
            .add("greeting".to_owned(), make_input())
            .expect_err("重複 add は拒否される");
        assert!(
            matches!(err, TextOverlayError::AlreadyExists),
            "AlreadyExists が返る: {err:?}"
        );
    }

    /// add: OVERLAY_LIMIT を超えると LimitExceeded で拒否。
    #[test]
    fn processor_state_add_rejects_limit_exceeded() {
        let mut state = make_state();
        for i in 0..OVERLAY_LIMIT {
            state
                .add(format!("overlay-{i}"), make_input())
                .expect("上限内は成功");
        }
        let err = state
            .add("over-limit".to_owned(), make_input())
            .expect_err("上限超過は拒否される");
        assert!(
            matches!(err, TextOverlayError::LimitExceeded),
            "LimitExceeded が返る: {err:?}"
        );
    }

    /// add: z = None は宣言順 (next_auto_z) で自動割り当てされる。
    #[test]
    fn processor_state_add_assigns_auto_z_in_declaration_order() {
        let mut state = make_state();
        state.add("a".to_owned(), make_input()).expect("add a");
        state.add("b".to_owned(), make_input()).expect("add b");
        state.add("c".to_owned(), make_input()).expect("add c");
        assert_eq!(state.overlays["a"].z, 0, "1 番目は z=0");
        assert_eq!(state.overlays["b"].z, 1, "2 番目は z=1 (後勝ち)");
        assert_eq!(state.overlays["c"].z, 2, "3 番目は z=2");
    }

    /// add: z = Some(v) は明示値を採用し、以降の auto z は max(next, v+1) に追従する。
    #[test]
    fn processor_state_add_with_explicit_z_advances_auto_z() {
        let mut state = make_state();
        let mut input_with_z = make_input();
        input_with_z.z = Some(100);
        state
            .add("a".to_owned(), input_with_z)
            .expect("add a with z=100");
        state
            .add("b".to_owned(), make_input())
            .expect("add b with auto z");
        assert_eq!(state.overlays["a"].z, 100, "明示指定の z が採用される");
        assert_eq!(state.overlays["b"].z, 101, "auto z は明示値の次に追従する");
    }

    /// update: 存在する overlay の指定フィールドだけが更新される。
    #[test]
    fn processor_state_update_modifies_only_specified_fields() {
        let mut state = make_state();
        state.add("g".to_owned(), make_input()).expect("初期 add");
        let patch = TextOverlayPatch {
            text: Some("updated".to_owned()),
            x: Some(500),
            ..Default::default()
        };
        state.update("g".to_owned(), patch).expect("update 成功");
        let spec = &state.overlays["g"];
        assert_eq!(spec.text, "updated", "text が更新される");
        assert_eq!(spec.x, 500, "x が更新される");
        assert_eq!(spec.y, 20, "y は維持される");
        assert_eq!(spec.font_size, 32, "font_size は維持される");
    }

    /// update: 存在しない overlay は NotFound で拒否。
    #[test]
    fn processor_state_update_rejects_not_found() {
        let mut state = make_state();
        let err = state
            .update("missing".to_owned(), TextOverlayPatch::default())
            .expect_err("存在しない overlay の更新は拒否される");
        assert!(
            matches!(err, TextOverlayError::NotFound),
            "NotFound が返る: {err:?}"
        );
    }

    /// remove: 存在する overlay を削除すると消える。
    #[test]
    fn processor_state_remove_removes_existing_overlay() {
        let mut state = make_state();
        state.add("g".to_owned(), make_input()).expect("初期 add");
        state.dirty = false;
        state.remove("g".to_owned()).expect("remove 成功");
        assert!(!state.overlays.contains_key("g"), "overlays から消える");
        assert!(state.dirty, "dirty フラグが立つ");
    }

    /// remove: 存在しない overlay は NotFound で拒否。
    #[test]
    fn processor_state_remove_rejects_not_found() {
        let mut state = make_state();
        let err = state
            .remove("missing".to_owned())
            .expect_err("存在しない overlay の削除は拒否される");
        assert!(
            matches!(err, TextOverlayError::NotFound),
            "NotFound が返る: {err:?}"
        );
    }

    /// list: 登録されている全 overlay が返る (name と spec を含む)。
    #[test]
    fn processor_state_list_returns_all_overlays() {
        let mut state = make_state();
        state.add("a".to_owned(), make_input()).expect("add a");
        state.add("b".to_owned(), make_input()).expect("add b");
        let listed = state.list();
        assert_eq!(listed.len(), 2, "2 件の overlay が返る");
        let names: Vec<&str> = listed.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names.contains(&"a") && names.contains(&"b"),
            "両方の name が含まれる"
        );
    }

    /// render: overlay が無いときは透過 I420A (A プレーン全 0) が返る。
    #[test]
    fn processor_state_render_empty_returns_transparent_frame() {
        let state = make_state();
        let frame = state
            .render(Duration::from_secs(0))
            .expect("空 overlay の render は成功");
        assert_eq!(frame.format, VideoFormat::I420A, "format は I420A");
        let w = 1920usize;
        let h = 1080usize;
        let y_size = w * h;
        let uv_size = w.div_ceil(2) * h.div_ceil(2);
        assert_eq!(
            frame.data.len(),
            y_size + uv_size * 2 + y_size,
            "I420A のレイアウト [Y | U | V | A] の合計バイト数"
        );
        let alpha_start = y_size + uv_size * 2;
        assert!(
            frame.data[alpha_start..].iter().all(|&a| a == 0),
            "overlay 無しなら A プレーンは全 0 (完全透明)"
        );
    }

    /// render: overlay 1 枚の描画後、A プレーンの指定 x/y 近傍に非ゼロ画素が存在する。
    #[test]
    fn processor_state_render_with_overlay_produces_nonzero_alpha_near_text() {
        let mut state = make_state();
        // canvas 中央付近にサイズ 64px のテキストを描画する
        let input = TextOverlaySpecInput {
            text: "T".to_owned(),
            x: 100,
            y: 100,
            font_size: 64,
            font_color_argb: 0xFFFFFFFF,
            font_name: "PublicSans-Regular.ttf".to_owned(),
            z: None,
        };
        state.add("t".to_owned(), input).expect("add 成功");
        let frame = state.render(Duration::from_secs(0)).expect("render 成功");
        assert_eq!(frame.format, VideoFormat::I420A, "format は I420A");
        let w = 1920usize;
        let h = 1080usize;
        let y_size = w * h;
        let uv_size = w.div_ceil(2) * h.div_ceil(2);
        let alpha_start = y_size + uv_size * 2;
        let alpha = &frame.data[alpha_start..];

        // 描画範囲 (おおむね (100, 100) から (100+font_size, 100+font_size)) に非ゼロ画素があることを確認する。
        // raden の baseline ベース計算で実際の描画 y は y + font.ascent() だが、
        // font_size 64px に対するアセントは概ね 50-60px なので (100, 100)-(200, 200) には含まれる。
        let mut nonzero_in_region = 0usize;
        for row in 50..250 {
            for col in 50..250 {
                if alpha[row * w + col] > 0 {
                    nonzero_in_region += 1;
                }
            }
        }
        assert!(
            nonzero_in_region > 0,
            "描画した文字 'T' の周辺領域に A != 0 のピクセルがあるはず"
        );

        // 描画範囲外 (右下 1700+) には A == 0 が広く分布していることを確認する (誤検出回避)。
        let mut zero_in_far_region = 0usize;
        for row in 800..900 {
            for col in 1700..1800 {
                if alpha[row * w + col] == 0 {
                    zero_in_far_region += 1;
                }
            }
        }
        assert!(
            zero_in_far_region > 9000,
            "描画範囲外は A == 0 (透明) のピクセルが大半 (got nonzero={})",
            10000 - zero_in_far_region
        );
    }

    /// add でバリデーション失敗 (text が長すぎ) → InvalidText が返り、overlays は変化しない。
    #[test]
    fn processor_state_add_rejects_invalid_text() {
        let mut state = make_state();
        let mut input = make_input();
        input.text = "a".repeat(TEXT_MAX_BYTES + 1);
        let err = state
            .add("g".to_owned(), input)
            .expect_err("text 上限超過は拒否される");
        assert!(
            matches!(err, TextOverlayError::InvalidText(_)),
            "InvalidText が返る: {err:?}"
        );
        assert!(
            state.overlays.is_empty(),
            "失敗時は overlays に挿入されない"
        );
    }
}
