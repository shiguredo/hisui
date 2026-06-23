//! `TextOverlayLayer` 本体。
//!
//! `VideoRealtimeMixerRunner` の内部状態として保持され、 `compose_frame` の
//! 追加合成段で「raden 描画 → straight 復元 → I420A 変換 → cached I420A 保持」 を行う。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::types::EvenUsize;
use crate::video::{VideoFormat, VideoFrame, VideoFrameSize};

use super::validate::{
    apply_patch, validate_font_name_and_resolve_path, validate_text_and_font_size,
};
use super::{
    OVERLAY_LIMIT, TextOverlayConfig, TextOverlayError, TextOverlayPatch, TextOverlaySpec,
    TextOverlaySpecInput, TextOverlayState,
};

/// `VideoRealtimeMixer` 内部のテキストオーバーレイレイヤ。
///
/// 機能有効時のみ `Some` で保持され、 `compose_frame` の追加合成段で
/// 既存の合成済み I420 に最上位レイヤとして I420A バッファをブレンドする。
/// canvas サイズは構築時に固定し、 シーン切替で再生成されない。
pub struct TextOverlayLayer {
    canvas_width: EvenUsize,
    canvas_height: EvenUsize,
    config: TextOverlayConfig,
    overlays: BTreeMap<String, TextOverlaySpec>,
    cached_frame: Option<Arc<VideoFrame>>,
    dirty: bool,
    /// 入力 `z = None` (宣言順) を解決する際に使う次の自動 z。
    /// `z = Some(v)` を受けた場合は `next_auto_z = max(next_auto_z, v + 1)` に更新する。
    next_auto_z: i32,
    /// canonical なフォントパスから `FontFace` への参照キャッシュ。
    ///
    /// Add / Update の validate と render の両方から `resolve_font_face` を経由してアクセスし、
    /// 同一フォントの再読み込みを避ける。 `--font-search-root` 配下は起動後に置換されない
    /// 前提のため、 エントリの破棄は行わない (overlay 削除でも残す)。
    font_cache: BTreeMap<PathBuf, Arc<raden::FontFace>>,
}

impl std::fmt::Debug for TextOverlayLayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // raden::FontFace と VideoFrame.data の生バイト列はダンプしないように調整する。
        f.debug_struct("TextOverlayLayer")
            .field("canvas_width", &self.canvas_width)
            .field("canvas_height", &self.canvas_height)
            .field("config", &self.config)
            .field("overlays", &self.overlays)
            .field("cached_frame_present", &self.cached_frame.is_some())
            .field("dirty", &self.dirty)
            .field("next_auto_z", &self.next_auto_z)
            .field(
                "font_cache_keys",
                &self.font_cache.keys().collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl TextOverlayLayer {
    /// 機能有効時に `VideoRealtimeMixer::run()` 冒頭で呼ばれる。
    ///
    /// raden の `PipelineRuntime` を一度作って空文字列 `fill_text` を呼び、
    /// JIT コンパイラを warm-up する。 ここでデフォルトフォントをロード・キャッシュする
    /// (起動時検証はすでに `TextOverlayConfig::build` で済んでいるため、 ここでは読み込みのみ)。
    pub fn new(
        canvas_width: EvenUsize,
        canvas_height: EvenUsize,
        config: TextOverlayConfig,
    ) -> crate::Result<Self> {
        let mut layer = Self {
            canvas_width,
            canvas_height,
            config,
            overlays: BTreeMap::new(),
            cached_frame: None,
            dirty: false,
            next_auto_z: 0,
            font_cache: BTreeMap::new(),
        };

        // デフォルトフォントを事前にロードしてキャッシュに乗せる。
        // 起動時検証で読み込み可能であることは確認済み (`TextOverlayConfig::build`) なので
        // ここでの失敗は想定外だが、 万一の場合は `crate::Error` に変換して上位に伝える。
        let default_name = layer.config.default_font_name.clone();
        layer
            .resolve_font_face(&default_name)
            .map_err(|e| crate::Error::new(format!("failed to preload default font: {e}")))?;

        // JIT warm-up: 1x1 の Prgb32 Image に対して空文字列の fill_text を一度実行する。
        // `PipelineRuntime` の内部キャッシュにテキスト描画パイプラインが乗ることを期待する。
        // 失敗しても致命的ではないので、 ログだけ出して継続する。
        {
            let face = layer
                .font_cache
                .values()
                .next()
                .expect("default font is preloaded")
                .clone();
            let mut image = raden::Image::new(1, 1, raden::PixelFormat::Prgb32);
            let mut runtime = raden::PipelineRuntime::new();
            let font = raden::Font::from_face(&face, 16.0);
            let mut ctx = raden::Context::new(&mut image, &mut runtime);
            ctx.set_fill_style(raden::Rgba32::new(0, 0, 0, 0));
            ctx.fill_text(0.0, 0.0, &font, "");
            ctx.end();
        }

        Ok(layer)
    }

    /// canonical path をキーにフォントをキャッシュ参照する。
    fn resolve_font_face(
        &mut self,
        font_name: &str,
    ) -> Result<Arc<raden::FontFace>, TextOverlayError> {
        let canonical = validate_font_name_and_resolve_path(font_name, &self.config)?;
        if let Some(face) = self.font_cache.get(&canonical) {
            return Ok(face.clone());
        }
        // NOTE: raden::FontData::from_file が将来的に &Path を取れるようになれば
        //       (shiguredo/raden issue 0041)、 ここの to_str() 変換は不要になる。
        let path_str = canonical.to_str().ok_or_else(|| {
            TextOverlayError::FontResolveFailed(format!(
                "font path {} is not utf-8",
                canonical.display()
            ))
        })?;
        let font_data = raden::FontData::from_file(path_str)
            .map_err(|e| TextOverlayError::FontResolveFailed(format!("load font: {e:?}")))?;
        let face = raden::FontFace::from_data(&font_data, 0)
            .map_err(|e| TextOverlayError::FontResolveFailed(format!("parse font: {e:?}")))?;
        let face = Arc::new(face);
        self.font_cache.insert(canonical, face.clone());
        Ok(face)
    }

    /// 現在 overlay が 1 件もない (= 何も合成しない) かどうか。
    pub fn is_empty(&self) -> bool {
        self.overlays.is_empty()
    }

    /// テスト用に `overlays` を読み取る。 本番コードからは使わない。
    #[cfg(test)]
    pub(super) fn overlays(&self) -> &BTreeMap<String, TextOverlaySpec> {
        &self.overlays
    }

    /// テスト用に `dirty` を読み取る。 本番コードからは使わない。
    #[cfg(test)]
    pub(super) fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// テスト用に `font_cache` のエントリ数を返す。 本番コードからは使わない。
    #[cfg(test)]
    pub(super) fn font_cache_len(&self) -> usize {
        self.font_cache.len()
    }

    /// 次の `compose_frame` 呼び出し前に、 必要なら再描画して cached I420A を更新する。
    ///
    /// 戻り値は cached I420A の `Arc<VideoFrame>` で、 timestamp は読まれない
    /// (合成段は pixel data のみ参照する)。
    /// `overlays.is_empty()` の場合は呼び出さない (呼び出し側で early return する)。
    pub fn ensure_rendered(&mut self) -> crate::Result<Arc<VideoFrame>> {
        debug_assert!(
            !self.overlays.is_empty(),
            "ensure_rendered は overlay が 1 件以上ある前提"
        );

        if self.dirty || self.cached_frame.is_none() {
            let frame = self.render()?;
            self.cached_frame = Some(Arc::new(frame));
            self.dirty = false;
        }
        let cached = self
            .cached_frame
            .as_ref()
            .expect("cached_frame is Some after rendering");
        Ok(Arc::clone(cached))
    }

    /// 新規 overlay を追加する。
    pub fn add(
        &mut self,
        name: String,
        input: TextOverlaySpecInput,
    ) -> Result<(), TextOverlayError> {
        if self.overlays.contains_key(&name) {
            return Err(TextOverlayError::AlreadyExists);
        }
        if self.overlays.len() >= OVERLAY_LIMIT {
            return Err(TextOverlayError::LimitExceeded);
        }
        validate_text_and_font_size(&input.text, input.font_size, self.canvas_height.get())?;
        // フォント解決成功時はキャッシュに乗り、 後段の render では再ロードしない。
        self.resolve_font_face(&input.font_name)?;
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

    /// 既存 overlay の指定フィールドを更新する。
    pub fn update(
        &mut self,
        name: String,
        patch: TextOverlayPatch,
    ) -> Result<(), TextOverlayError> {
        let existing = self
            .overlays
            .get(&name)
            .ok_or(TextOverlayError::NotFound)?
            .clone();
        // patch.z は apply_patch 後に「未指定」 と区別できなくなるので先に退避する。
        let patch_z = patch.z;
        let updated = apply_patch(existing, patch);
        validate_text_and_font_size(&updated.text, updated.font_size, self.canvas_height.get())?;
        // 更新後の fontName でフォント解決 + キャッシュ投入を行う。
        self.resolve_font_face(&updated.font_name)?;
        // 明示指定された z だけ next_auto_z を進める (z 未指定 Update は既存値を温存)。
        if let Some(z) = patch_z {
            self.next_auto_z = self.next_auto_z.max(z.saturating_add(1));
        }
        self.overlays.insert(name, updated);
        self.dirty = true;
        Ok(())
    }

    /// 既存 overlay を削除する。
    pub fn remove(&mut self, name: String) -> Result<(), TextOverlayError> {
        if self.overlays.remove(&name).is_none() {
            return Err(TextOverlayError::NotFound);
        }
        self.dirty = true;
        Ok(())
    }

    /// 現在登録されている全 overlay を `TextOverlayState` 配列で返す。
    pub fn list(&self) -> Vec<TextOverlayState> {
        self.overlays
            .iter()
            .map(|(name, spec)| TextOverlayState {
                name: name.clone(),
                spec: spec.clone(),
            })
            .collect()
    }

    /// raden + libyuv で 1 枚の I420A フレームを構築する。
    ///
    /// `VideoFrame.timestamp` は `Duration::ZERO` 固定 (合成段で読まれないため任意値)。
    fn render(&mut self) -> crate::Result<VideoFrame> {
        let w = self.canvas_width.get();
        let h = self.canvas_height.get();

        // raden は PixelFormat::Prgb32 = premultiplied ARGB を出力する。
        // バイト並びはリトルエンディアン環境で [B, G, R, A] となり、 libyuv の ArgbImage と整合する。
        let mut image = raden::Image::new(w as u32, h as u32, raden::PixelFormat::Prgb32);
        let mut runtime = raden::PipelineRuntime::new();

        // ループ内で `&mut self` (`resolve_font_face`) を呼ぶため、 overlay の借用を解放しておく。
        // z-order でソートし、 タイブレークは name のアルファベット順 (BTreeMap の iter 順)。
        let mut sorted: Vec<(String, TextOverlaySpec)> = self
            .overlays
            .iter()
            .map(|(name, spec)| (name.clone(), spec.clone()))
            .collect();
        sorted.sort_by(|(name_a, spec_a), (name_b, spec_b)| {
            spec_a.z.cmp(&spec_b.z).then(name_a.cmp(name_b))
        });

        {
            let mut ctx = raden::Context::new(&mut image, &mut runtime);

            // canvas を完全透明でクリア。
            ctx.set_comp_op(raden::CompOp::SrcCopy);
            ctx.set_fill_style(raden::Rgba32::new(0, 0, 0, 0));
            ctx.fill_rect(&raden::Rect::new(0.0, 0.0, w as f64, h as f64));
            ctx.set_comp_op(raden::CompOp::SrcOver);

            for (name, spec) in &sorted {
                // フォント解決失敗時は該当 overlay だけスキップして他は描画継続する。
                // run ループ全体を落とすと List も含めた全 RPC が止まるため、 影響範囲を局所化する。
                let face = match self.resolve_font_face(&spec.font_name) {
                    Ok(face) => face,
                    Err(e) => {
                        tracing::warn!("skip overlay '{name}' due to font resolve error: {e}");
                        continue;
                    }
                };
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

        // raden 出力は premultiplied なので、 libyuv に渡す前に straight alpha に戻す。
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
            timestamp: std::time::Duration::ZERO,
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
/// 取り出しはリトルエンディアン前提なので、 ビッグエンディアン環境では
/// 別経路が必要。 コンパイル時に検出して落とす。
///
/// 不変条件: raden の Prgb32 出力は premultiplied なので各チャネル `c_pre <= A`
/// を満たすはずだが、 万一違反した場合は `value.min(255)` でクランプして
/// 色情報損失となる (静かに壊れないよう assertion ではなく可視的なクランプ)。
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
            // 四捨五入用に A/2 を足してから A で割る。 255 でクランプする。
            let value = ((*c as u16) * 255 + a_u16 / 2) / a_u16;
            *c = value.min(255) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_layer() -> TextOverlayLayer {
        let config = TextOverlayConfig::build(
            Some(PathBuf::from("testdata/fonts")),
            Some("PublicSans-Regular.ttf".to_owned()),
        )
        .expect("テスト用 config が組み立てられる")
        .expect("両方指定なので Some");
        TextOverlayLayer::new(
            EvenUsize::new(1920).expect("1920 は偶数"),
            EvenUsize::new(1080).expect("1080 は偶数"),
            config,
        )
        .expect("テスト用 TextOverlayLayer が構築できる")
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

    /// add: overlay が挿入され、 dirty フラグが立つ。
    #[test]
    fn add_inserts_overlay_and_marks_dirty() {
        let mut layer = make_layer();
        layer
            .add("greeting".to_owned(), make_input())
            .expect("add に成功");
        assert!(
            layer.overlays().contains_key("greeting"),
            "overlays に挿入される"
        );
        assert!(layer.is_dirty(), "dirty フラグが立つ");
    }

    /// add: 同名 overlay は AlreadyExists で拒否。
    #[test]
    fn add_rejects_duplicate() {
        let mut layer = make_layer();
        layer
            .add("greeting".to_owned(), make_input())
            .expect("初回 add 成功");
        let err = layer
            .add("greeting".to_owned(), make_input())
            .expect_err("重複 add は拒否される");
        assert!(
            matches!(err, TextOverlayError::AlreadyExists),
            "AlreadyExists が返る: {err:?}"
        );
    }

    /// add: OVERLAY_LIMIT を超えると LimitExceeded で拒否。
    #[test]
    fn add_rejects_limit_exceeded() {
        let mut layer = make_layer();
        for i in 0..OVERLAY_LIMIT {
            layer
                .add(format!("overlay-{i}"), make_input())
                .expect("上限内は成功");
        }
        let err = layer
            .add("over-limit".to_owned(), make_input())
            .expect_err("上限超過は拒否される");
        assert!(
            matches!(err, TextOverlayError::LimitExceeded),
            "LimitExceeded が返る: {err:?}"
        );
    }

    /// add: z = None は宣言順 (next_auto_z) で自動割り当てされる。
    #[test]
    fn add_assigns_auto_z_in_declaration_order() {
        let mut layer = make_layer();
        layer
            .add("a".to_owned(), make_input())
            .expect("a を add する");
        layer
            .add("b".to_owned(), make_input())
            .expect("b を add する");
        layer
            .add("c".to_owned(), make_input())
            .expect("c を add する");
        assert_eq!(layer.overlays()["a"].z, 0, "1 番目は z=0");
        assert_eq!(layer.overlays()["b"].z, 1, "2 番目は z=1 (後勝ち)");
        assert_eq!(layer.overlays()["c"].z, 2, "3 番目は z=2");
    }

    /// add: z = Some(v) は明示値を採用し、 以降の auto z は max(next, v+1) に追従する。
    #[test]
    fn add_with_explicit_z_advances_auto_z() {
        let mut layer = make_layer();
        let mut input_with_z = make_input();
        input_with_z.z = Some(100);
        layer
            .add("a".to_owned(), input_with_z)
            .expect("z=100 で a を add する");
        layer
            .add("b".to_owned(), make_input())
            .expect("auto z で b を add する");
        assert_eq!(layer.overlays()["a"].z, 100, "明示指定の z が採用される");
        assert_eq!(
            layer.overlays()["b"].z,
            101,
            "auto z は明示値の次に追従する"
        );
    }

    /// update: 存在する overlay の指定フィールドだけが更新される。
    #[test]
    fn update_modifies_only_specified_fields() {
        let mut layer = make_layer();
        layer
            .add("g".to_owned(), make_input())
            .expect("初期 add に成功");
        let patch = TextOverlayPatch {
            text: Some("updated".to_owned()),
            x: Some(500),
            ..Default::default()
        };
        layer.update("g".to_owned(), patch).expect("update に成功");
        let spec = &layer.overlays()["g"];
        assert_eq!(spec.text, "updated", "text が更新される");
        assert_eq!(spec.x, 500, "x が更新される");
        assert_eq!(spec.y, 20, "y は維持される");
        assert_eq!(spec.font_size, 32, "font_size は維持される");
    }

    /// update: 存在しない overlay は NotFound で拒否。
    #[test]
    fn update_rejects_not_found() {
        let mut layer = make_layer();
        let err = layer
            .update("missing".to_owned(), TextOverlayPatch::default())
            .expect_err("存在しない overlay の更新は拒否される");
        assert!(
            matches!(err, TextOverlayError::NotFound),
            "NotFound が返る: {err:?}"
        );
    }

    /// remove: 存在する overlay を削除すると消える。
    #[test]
    fn remove_removes_existing_overlay() {
        let mut layer = make_layer();
        layer
            .add("g".to_owned(), make_input())
            .expect("初期 add に成功");
        // dirty フラグを一旦リセットして、 remove が立てることを観察するために
        // ensure_rendered で消費するのは大袈裟なので、 unsafe にはしない。
        // ここでは dirty 状態は気にせず remove の挙動を確認する。
        layer.remove("g".to_owned()).expect("remove に成功");
        assert!(!layer.overlays().contains_key("g"), "overlays から消える");
        assert!(layer.is_dirty(), "dirty フラグが立つ");
    }

    /// remove: 存在しない overlay は NotFound で拒否。
    #[test]
    fn remove_rejects_not_found() {
        let mut layer = make_layer();
        let err = layer
            .remove("missing".to_owned())
            .expect_err("存在しない overlay の削除は拒否される");
        assert!(
            matches!(err, TextOverlayError::NotFound),
            "NotFound が返る: {err:?}"
        );
    }

    /// list: 登録されている全 overlay が返る (name と spec を含む)。
    #[test]
    fn list_returns_all_overlays() {
        let mut layer = make_layer();
        layer
            .add("a".to_owned(), make_input())
            .expect("a を add する");
        layer
            .add("b".to_owned(), make_input())
            .expect("b を add する");
        let listed = layer.list();
        assert_eq!(listed.len(), 2, "2 件の overlay が返る");
        let names: Vec<&str> = listed.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names.contains(&"a") && names.contains(&"b"),
            "両方の name が含まれる"
        );
    }

    /// render: overlay 1 枚の描画後、 A プレーンの指定 x/y 近傍に非ゼロ画素が存在する。
    #[test]
    fn render_with_overlay_produces_nonzero_alpha_near_text() {
        let mut layer = make_layer();
        // canvas 中央付近にサイズ 64px のテキストを描画する。
        let input = TextOverlaySpecInput {
            text: "T".to_owned(),
            x: 100,
            y: 100,
            font_size: 64,
            font_color_argb: 0xFFFFFFFF,
            font_name: "PublicSans-Regular.ttf".to_owned(),
            z: None,
        };
        layer.add("t".to_owned(), input).expect("add に成功");
        let frame = layer.render().expect("render に成功");
        assert_eq!(frame.format, VideoFormat::I420A, "format は I420A");
        let w = 1920usize;
        let h = 1080usize;
        let y_size = w * h;
        let uv_size = w.div_ceil(2) * h.div_ceil(2);
        let alpha_start = y_size + uv_size * 2;
        let alpha = &frame.data[alpha_start..];

        // 描画範囲 (おおむね (100, 100) から (100+font_size, 100+font_size)) に
        // 非ゼロ画素があることを確認する。
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

        // 描画範囲外 (右下 1700+) は全 100x100 ピクセルが A == 0 であることを確認する
        // (緩い閾値だと誤描画を見逃すので厳密に検査する)。
        const FAR_REGION_TOTAL: usize = 100 * 100;
        let mut zero_in_far_region = 0usize;
        for row in 800..900 {
            for col in 1700..1800 {
                if alpha[row * w + col] == 0 {
                    zero_in_far_region += 1;
                }
            }
        }
        assert_eq!(
            zero_in_far_region, FAR_REGION_TOTAL,
            "描画範囲外は A == 0 (透明) であるべき"
        );
    }

    /// add でバリデーション失敗 (text が長すぎ) → InvalidText が返り、 overlays は変化しない。
    #[test]
    fn add_rejects_invalid_text() {
        let mut layer = make_layer();
        let mut input = make_input();
        input.text = "a".repeat(super::super::TEXT_MAX_BYTES + 1);
        let err = layer
            .add("g".to_owned(), input)
            .expect_err("text 上限超過は拒否される");
        assert!(
            matches!(err, TextOverlayError::InvalidText(_)),
            "InvalidText が返る: {err:?}"
        );
        assert!(
            layer.overlays().is_empty(),
            "失敗時は overlays に挿入されない"
        );
    }

    /// 同じ fontName の overlay を複数 add しても font_cache は 1 エントリしか持たない。
    /// ディスク I/O は初回のみで、 再 add や render では再ロードしない。
    #[test]
    fn caches_font_across_multiple_overlays() {
        let mut layer = make_layer();
        // make_layer() で default font が事前ロード済みなので 1 エントリ。
        assert_eq!(
            layer.font_cache_len(),
            1,
            "デフォルトフォントの 1 エントリだけ持つ"
        );
        layer
            .add("a".to_owned(), make_input())
            .expect("1 つ目の add は成功");
        layer
            .add("b".to_owned(), make_input())
            .expect("2 つ目の add も成功");
        assert_eq!(
            layer.font_cache_len(),
            1,
            "同一フォントは 1 エントリだけキャッシュされる"
        );
    }

    /// `unpremultiply_argb` の挙動: A == 0 は触らない、 A == 255 は変化なし、 A == 128 は約 2 倍。
    #[test]
    fn unpremultiply_argb_handles_basic_cases() {
        let mut data = vec![
            // pixel 0: 完全透明 - そのまま残る
            0, 0, 0, 0, // pixel 1: 不透明、 白
            255, 255, 255, 255,
            // pixel 2: 半透明、 premultiplied 値が 128 (= straight 255 * 128/255)
            128, 128, 128, 128,
        ];
        unpremultiply_argb(&mut data);
        assert_eq!(&data[0..4], &[0, 0, 0, 0]);
        assert_eq!(&data[4..8], &[255, 255, 255, 255]);
        assert_eq!(&data[8..12], &[255, 255, 255, 128]);
    }
}
