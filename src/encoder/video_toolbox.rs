use std::collections::VecDeque;

use crate::{
    encoder::VideoEncoderOptions,
    sample_entry::SharedSampleEntry,
    types::{CodecName, EvenUsize},
    video::h264,
    video::h265,
    video::{FrameRate, RawVideoFrame, VideoFormat, VideoFrame, VideoFrameSize},
};

// pending_output に同時保持できる最大フレーム数。
// sample_entry 確定までの健全状態の出力フレーム数 (通常は 0 〜 数フレーム) に
// 余裕を持たせた値で、これを超えたら異常状態として Err を返す。
const MAX_PENDING_OUTPUT_FRAMES: usize = 64;

#[derive(Debug)]
pub struct VideoToolboxEncoder {
    inner: shiguredo_video_toolbox::Encoder,
    input_queue: VecDeque<RawVideoFrame>,
    // sample_entry 確定済みフレームを保持する出力キュー。next_encoded_frame で先頭から取り出す。
    output_queue: VecDeque<VideoFrame>,
    // sample_entry 未確定の間にエンコードされたフレームを一時退避する内部バッファ。
    // sample_entry 確定時に drain して output_queue にフラッシュする。
    // H.265 経路では h265_sample_entry が空入力でも Ok を返すため初回反復で必ず確定し、
    // この pending_output は仕様上常に空のまま運用される。
    pending_output: VecDeque<VideoFrame>,
    // 最初の出力フレームの SPS/PPS から確定するサンプルエントリー。確定後は全フレームに載せる。
    sample_entry: Option<SharedSampleEntry>,
    width: EvenUsize,
    height: EvenUsize,
    format: VideoFormat,
    fps: FrameRate,
    keyframe_request_pending: bool,
}

impl VideoToolboxEncoder {
    pub fn new_h264(options: &VideoEncoderOptions) -> crate::Result<Self> {
        let width = options.width;
        let height = options.height;
        let mut config = options.encode_params.video_toolbox_h264.clone();
        config.width = u32::try_from(width.get())
            .map_err(|_| crate::Error::new("video width is too large for VideoToolbox"))?;
        config.height = u32::try_from(height.get())
            .map_err(|_| crate::Error::new("video height is too large for VideoToolbox"))?;
        config.average_bitrate = Some(options.bitrate as u64);
        config.fps_numerator = options.frame_rate.numerator.get() as u32;
        config.fps_denominator = options.frame_rate.denumerator.get() as u32;
        if !matches!(config.codec, shiguredo_video_toolbox::CodecConfig::H264(_)) {
            return Err(crate::Error::new(
                "BUG: VideoToolbox H.264 config must use H264 codec settings",
            ));
        }
        let inner = shiguredo_video_toolbox::Encoder::new(config)?;
        Ok(Self {
            inner,
            input_queue: VecDeque::new(),
            output_queue: VecDeque::new(),
            pending_output: VecDeque::new(),
            sample_entry: None,
            width,
            height,
            format: VideoFormat::H264,
            fps: options.frame_rate,
            keyframe_request_pending: false,
        })
    }

    pub fn new_h265(options: &VideoEncoderOptions) -> crate::Result<Self> {
        let width = options.width;
        let height = options.height;
        let mut config = options.encode_params.video_toolbox_h265.clone();
        config.width = u32::try_from(width.get())
            .map_err(|_| crate::Error::new("video width is too large for VideoToolbox"))?;
        config.height = u32::try_from(height.get())
            .map_err(|_| crate::Error::new("video height is too large for VideoToolbox"))?;
        config.average_bitrate = Some(options.bitrate as u64);
        config.fps_numerator = options.frame_rate.numerator.get() as u32;
        config.fps_denominator = options.frame_rate.denumerator.get() as u32;
        if !matches!(config.codec, shiguredo_video_toolbox::CodecConfig::Hevc(_)) {
            return Err(crate::Error::new(
                "BUG: VideoToolbox H.265 config must use HEVC codec settings",
            ));
        }
        let inner = shiguredo_video_toolbox::Encoder::new(config)?;
        Ok(Self {
            inner,
            input_queue: VecDeque::new(),
            output_queue: VecDeque::new(),
            pending_output: VecDeque::new(),
            sample_entry: None,
            width,
            height,
            format: VideoFormat::H265,
            fps: options.frame_rate,
            keyframe_request_pending: false,
        })
    }

    pub fn codec(&self) -> CodecName {
        if self.format == VideoFormat::H264 {
            CodecName::H264
        } else {
            CodecName::H265
        }
    }

    pub fn encode(&mut self, frame: RawVideoFrame) -> crate::Result<()> {
        let (y_plane, u_plane, v_plane) = frame.as_i420_planes()?;
        self.inner.encode(
            &shiguredo_video_toolbox::FrameData::I420 {
                y: y_plane,
                u: u_plane,
                v: v_plane,
            },
            &shiguredo_video_toolbox::EncodeOptions {
                force_key_frame: self.keyframe_request_pending,
            },
        )?;
        self.keyframe_request_pending = false;

        // Video Toolbox のエンコーダーは非同期で動作し、
        // エンコードが終わるまでは入力バッファへの参照を保持する必要があるので、
        // バッファもキューに入れておく。
        // (将来的にはこの辺りはエンコーダー内で隠蔽した方が使いやすそう）
        self.input_queue.push_back(frame);

        self.handle_encoded()?;

        Ok(())
    }

    // 内部エンコーダのフラッシュ後に保留フレームが残っていれば、最初の keyframe が
    // 一度も出ずに終端した異常状態として Err を返す。
    pub fn finish(&mut self) -> crate::Result<()> {
        self.inner.finish()?;
        self.handle_encoded()?;
        if !self.pending_output.is_empty() {
            let discarded = self.pending_output.len();
            self.pending_output.clear();
            return Err(crate::Error::new(format!(
                "video_toolbox encoder finished without establishing sample_entry; {} frames discarded",
                discarded
            )));
        }
        Ok(())
    }

    pub fn next_encoded_frame(&mut self) -> Option<VideoFrame> {
        self.output_queue.pop_front()
    }

    pub fn request_keyframe(&mut self) {
        self.keyframe_request_pending = true;
    }

    fn handle_encoded(&mut self) -> crate::Result<()> {
        while let Some(frame) = self.inner.next_frame()? {
            let input_frame = self
                .input_queue
                .pop_front()
                .ok_or_else(|| crate::Error::new("encoded frame produced without input frame"))?;

            // 最初の出力フレームの SPS/PPS からサンプルエントリーを確定して保持し、
            // 以後は全出力フレームに保持済みのサンプルエントリーを載せる。
            // shiguredo_video_toolbox は keyframe 出力時のみ SPS / PPS を返すため、
            // 非 keyframe フレームでは frame.sps_list / pps_list が空になる。H.264 経路では
            // openh264 経路と同様に空 SPS / PPS でのサンプルエントリー構築をスキップし、
            // 次の keyframe を待つ (空入力で h264_sample_entry_from_sps_pps_lists を呼ぶと
            // Err になりエンコーダが落ちるため)。
            // sample_entry_just_established は「この反復で None → Some に遷移したか」を表し、
            // 退避していた保留フレームをフラッシュするかどうかの判定に使う。
            let was_none = self.sample_entry.is_none();
            if was_none {
                let sample_entry_opt = if self.format == VideoFormat::H264 {
                    if frame.sps_list.is_empty() || frame.pps_list.is_empty() {
                        None
                    } else {
                        let (entry, _frame_size) = h264::h264_sample_entry_from_sps_pps_lists(
                            frame.sps_list.clone(),
                            frame.pps_list.clone(),
                        )?;
                        Some(entry)
                    }
                } else {
                    Some(h265::h265_sample_entry(
                        self.width,
                        self.height,
                        self.fps,
                        frame.vps_list.clone(),
                        frame.sps_list.clone(),
                        frame.pps_list.clone(),
                    )?)
                };
                if let Some(sample_entry) = sample_entry_opt {
                    self.sample_entry = Some(SharedSampleEntry::new(sample_entry));
                }
            }
            let sample_entry_just_established = was_none && self.sample_entry.is_some();

            let frame_out = VideoFrame {
                data: frame.data,
                format: self.format,
                keyframe: frame.keyframe,
                size: Some(VideoFrameSize {
                    width: self.width.get(),
                    height: self.height.get(),
                }),
                timestamp: input_frame.as_video_frame().timestamp,
                sample_entry: self.sample_entry.clone(),
            };

            if self.sample_entry.is_some() {
                // 確定済み: output_queue に直接 push。
                // 確定が今反復で起きた場合だけ、pending_output に溜まっていた退避フレームに
                // 確定済み sample_entry を載せて先にフラッシュする。退避は出力順 = 入力順で
                // 並んでいるため、フラッシュ後に当該反復のフレームを積むことで PTS 順序を維持する。
                if sample_entry_just_established {
                    let entry = self
                        .sample_entry
                        .clone()
                        .expect("確定処理直後なので Some が保証されている");
                    for mut pending in self.pending_output.drain(..) {
                        pending.sample_entry = Some(entry.clone());
                        self.output_queue.push_back(pending);
                    }
                }
                self.output_queue.push_back(frame_out);
            } else {
                // 未確定: 内部バッファに退避する。
                // 上限超過時は異常状態として Err を返すが、エンコーダ自体は使用可能と
                // して扱うため pending_output だけ clear して呼び出し側の再開を許す。
                if self.pending_output.len() >= MAX_PENDING_OUTPUT_FRAMES {
                    self.pending_output.clear();
                    return Err(crate::Error::new(format!(
                        "video_toolbox encoder pending output overflow before sample_entry is established (limit={})",
                        MAX_PENDING_OUTPUT_FRAMES
                    )));
                }
                self.pending_output.push_back(frame_out);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;
    use std::sync::Arc;

    use super::*;
    use crate::video::VideoFrameSize;

    fn options() -> VideoEncoderOptions {
        VideoEncoderOptions {
            codec: CodecName::H264,
            engines: None,
            bitrate: 500_000,
            width: EvenUsize::truncating_new(64),
            height: EvenUsize::truncating_new(64),
            frame_rate: FrameRate {
                numerator: NonZeroUsize::MIN.saturating_add(29),
                denumerator: NonZeroUsize::MIN,
            },
            encode_params: crate::encoder::default_video_encode_config_for_rpc(),
        }
    }

    // 64x64 の I420 グレーフレームを作る。openh264 のテストと同じ流儀。
    fn raw_i420_frame(ts_ms: u64) -> RawVideoFrame {
        let (width, height) = (64usize, 64usize);
        let y_size = width * height;
        let uv_size = (width / 2) * (height / 2);
        let data: Vec<u8> = std::iter::repeat_n(16u8, y_size)
            .chain(std::iter::repeat_n(128u8, uv_size * 2))
            .collect();
        let frame = VideoFrame {
            data,
            format: VideoFormat::I420,
            keyframe: true,
            size: Some(VideoFrameSize { width, height }),
            timestamp: std::time::Duration::from_millis(ts_ms),
            sample_entry: None,
        };
        RawVideoFrame::from_i420_video_frame(Arc::new(frame)).expect("有効な I420 フレームのはず")
    }

    // 全出力フレームに sample_entry が載る不変条件を検証する。
    // VideoToolbox は最初の keyframe で SPS/PPS が確定し、以降は保持値を伝播する。
    // 同時に、sample_entry 確定後は pending_output が空であることを観測し、
    // 退避設計の事後条件 (確定タイミングで drain される) を確認する。
    fn assert_every_output_frame_has_sample_entry(
        mut encoder: VideoToolboxEncoder,
    ) -> crate::Result<()> {
        let mut output_count = 0;
        for i in 0..10u64 {
            encoder.encode(raw_i420_frame(i * 33))?;
            while let Some(frame) = encoder.next_encoded_frame() {
                assert!(
                    frame.sample_entry.is_some(),
                    "出力フレームに sample_entry が載っていない（フレーム番号: {output_count}）"
                );
                output_count += 1;
            }
            // sample_entry 確定後は pending_output が空のままになる事後条件を確認する。
            if encoder.sample_entry.is_some() {
                assert!(
                    encoder.pending_output.is_empty(),
                    "sample_entry 確定後に pending_output が残存している（残存数: {}）",
                    encoder.pending_output.len()
                );
            }
        }
        encoder.finish()?;
        while let Some(frame) = encoder.next_encoded_frame() {
            assert!(
                frame.sample_entry.is_some(),
                "finish 後の出力フレームに sample_entry が載っていない"
            );
            output_count += 1;
        }
        // finish 後も pending_output は空であるはず。
        assert!(
            encoder.pending_output.is_empty(),
            "finish 後に pending_output が残存している（残存数: {}）",
            encoder.pending_output.len()
        );
        // 全フレーム付与を確認するには 2 フレーム以上の出力が必要。
        assert!(
            output_count >= 2,
            "出力フレーム数が少なすぎる（確認には 2 フレーム以上必要）: {output_count}"
        );
        Ok(())
    }

    #[test]
    fn video_toolbox_h264_sets_sample_entry_on_every_output_frame() -> crate::Result<()> {
        let encoder = VideoToolboxEncoder::new_h264(&options())?;
        assert_every_output_frame_has_sample_entry(encoder)
    }

    #[test]
    fn video_toolbox_h265_sets_sample_entry_on_every_output_frame() -> crate::Result<()> {
        let encoder = VideoToolboxEncoder::new_h265(&options())?;
        assert_every_output_frame_has_sample_entry(encoder)
    }
}
