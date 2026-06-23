use std::collections::VecDeque;

use crate::{
    encoder::VideoEncoderOptions,
    sample_entry::SharedSampleEntry,
    video::h264::{self, H264_NALU_TYPE_SEI, H264AnnexBNalUnits},
    video::{RawVideoFrame, VideoFormat, VideoFrame},
};

// pending_output に同時保持できる最大フレーム数。
// sample_entry が確定する前に到達し得る健全状態の出力フレーム数 (通常は 0 〜 数フレーム)
// に余裕を持たせた値で、これを超えたら異常状態として Err を返す。
const MAX_PENDING_OUTPUT_FRAMES: usize = 64;

#[derive(Debug)]
pub struct Openh264Encoder {
    inner: shiguredo_openh264::Encoder,
    // sample_entry が確定済みのフレームを保持する出力キュー。
    // next_encoded_frame で先頭から取り出す。
    output_queue: VecDeque<VideoFrame>,
    // sample_entry 未確定の間にエンコードされたフレームを一時退避する内部バッファ。
    // SPS/PPS が揃って last_sample_entry が確定した時点で drain して
    // output_queue に流し、以降の保留は発生しない。
    pending_output: VecDeque<VideoFrame>,
    force_idr_pending: bool,
    // 最後に確定したサンプルエントリー。SPS/PPS を含むフレームで更新し、全出力フレームに載せる。
    // openh264 はキーフレーム要求等で SPS/PPS がストリーム途中で変わりうるため、最新値に追従する。
    last_sample_entry: Option<SharedSampleEntry>,
}

impl Openh264Encoder {
    pub fn new(
        lib: shiguredo_openh264::Openh264Library,
        options: &VideoEncoderOptions,
    ) -> crate::Result<Self> {
        let width = options.width.get();
        let height = options.height.get();
        let config = shiguredo_openh264::EncoderConfig {
            fps_numerator: options.frame_rate.numerator.get(),
            fps_denominator: options.frame_rate.denumerator.get(),
            width,
            height,
            target_bitrate: options.bitrate,
            ..options.encode_params.openh264.clone()
        };
        let inner = shiguredo_openh264::Encoder::new(lib, config)?;
        Ok(Self {
            inner,
            output_queue: VecDeque::new(),
            pending_output: VecDeque::new(),
            force_idr_pending: false,
            last_sample_entry: None,
        })
    }

    pub fn encode(&mut self, frame: RawVideoFrame) -> crate::Result<()> {
        let video_frame = frame.as_video_frame();
        let (y_plane, u_plane, v_plane) = frame.as_i420_planes()?;
        let encode_options = shiguredo_openh264::EncodeOptions {
            force_idr: self.force_idr_pending,
        };
        let encoded = self
            .inner
            .encode(y_plane, u_plane, v_plane, &encode_options)?;
        let Some(encoded) = encoded else {
            return Ok(());
        };

        // OpenH264 はキーフレーム要求時などに SPS/PPS が更新され得るため、
        // SPS/PPS を受け取ったフレームではサンプルエントリーを作り直して保持を更新する。
        // 以後は全出力フレームに保持済みの最新サンプルエントリーを載せる。
        // これにより、下流コンポーネントが参照するコーデック設定を最新化し、
        // 古いパラメータセット参照によるデコード失敗を避ける。
        //
        // sample_entry_just_established は「last_sample_entry が None から Some に
        // 遷移したターン」を表す。このターンでだけ pending_output に退避していた
        // 保留フレームをフラッシュする。すでに Some になっていた以降の更新では
        // pending_output は常に空のため drain は不要。
        let sample_entry_just_established = self.last_sample_entry.is_none()
            && !encoded.sps_list.is_empty()
            && !encoded.pps_list.is_empty();
        if !encoded.sps_list.is_empty() && !encoded.pps_list.is_empty() {
            let (sample_entry, _frame_size) = h264::h264_sample_entry_from_sps_pps_lists(
                encoded.sps_list.clone(),
                encoded.pps_list.clone(),
            )?;
            self.last_sample_entry = Some(SharedSampleEntry::new(sample_entry));
        }

        // AnnexB から MP4 向けの形式に変換する
        let mut data = Vec::new();
        for nal in H264AnnexBNalUnits::new(&encoded.data) {
            let nal = nal?;
            if nal.ty == H264_NALU_TYPE_SEI {
                // 一部のタイプは無視する
                continue;
            }

            data.extend_from_slice(&(nal.data.len() as u32).to_be_bytes());
            data.extend_from_slice(nal.data);
        }

        let is_keyframe = matches!(
            encoded.frame_type,
            shiguredo_openh264::FrameType::Idr | shiguredo_openh264::FrameType::I
        );
        if self.force_idr_pending && is_keyframe {
            self.force_idr_pending = false;
        }

        let frame_out = VideoFrame {
            data,
            format: VideoFormat::H264,
            keyframe: is_keyframe,
            size: Some(frame.size()),
            timestamp: video_frame.timestamp,
            sample_entry: self.last_sample_entry.clone(),
        };

        if self.last_sample_entry.is_some() {
            // sample_entry 確定済み: output_queue に直接積む。
            // 確定が今ターンで起きた場合だけ、pending_output に溜まっていた
            // 保留フレームに sample_entry を載せて先にフラッシュする。
            // 退避は出力順 = 入力順で並んでいるため、フラッシュ後に当該ターンの
            // フレームを積むことで PTS 順序を維持する。
            if sample_entry_just_established {
                let entry = self
                    .last_sample_entry
                    .clone()
                    .expect("確定処理直後なので Some が保証されている");
                for mut pending in self.pending_output.drain(..) {
                    pending.sample_entry = Some(entry.clone());
                    self.output_queue.push_back(pending);
                }
            }
            self.output_queue.push_back(frame_out);
        } else {
            // sample_entry 未確定: 内部バッファに退避する。
            // 上限超過時は異常状態として Err を返すが、エンコーダ自体は使用可能と
            // して扱うため pending_output だけ clear して呼び出し側の再開を許す。
            if self.pending_output.len() >= MAX_PENDING_OUTPUT_FRAMES {
                self.pending_output.clear();
                return Err(crate::Error::new(format!(
                    "openh264 encoder pending output overflow before sample_entry is established (limit={})",
                    MAX_PENDING_OUTPUT_FRAMES
                )));
            }
            self.pending_output.push_back(frame_out);
        }

        Ok(())
    }

    // 内部エンコーダ側のフラッシュは持たないため、保留フレームが残っていれば
    // 異常終了として Err を返す (一度も keyframe が出ずに finish された状態)。
    pub fn finish(&mut self) -> crate::Result<()> {
        if !self.pending_output.is_empty() {
            let discarded = self.pending_output.len();
            self.pending_output.clear();
            return Err(crate::Error::new(format!(
                "openh264 encoder finished without establishing sample_entry; {} frames discarded",
                discarded
            )));
        }
        Ok(())
    }

    pub fn next_encoded_frame(&mut self) -> Option<VideoFrame> {
        self.output_queue.pop_front()
    }

    pub fn request_keyframe(&mut self) {
        self.force_idr_pending = true;
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroUsize;
    use std::sync::Arc;

    use super::*;
    use crate::types::{CodecName, EvenUsize};
    use crate::video::{FrameRate, VideoFrameSize};

    // openh264 ライブラリを環境変数 OPENH264_PATH からロードする。
    // 未設定の場合は None を返す（テストスキップ）。
    fn load_openh264_lib() -> Option<shiguredo_openh264::Openh264Library> {
        let path = std::env::var("OPENH264_PATH").ok()?;
        Some(
            shiguredo_openh264::Openh264Library::load(path)
                .expect("openh264 ライブラリのロードに失敗した"),
        )
    }

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

    // 64x64 の I420 グレーフレームを作る。
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
    // openh264 は最初の出力フレームに SPS/PPS が含まれ、以降は last_sample_entry を
    // 全フレームに伝播させる。2 フレーム目以降でも Some になることを確認する。
    // あわせて sample_entry 確定後に pending_output が空であることも観測する
    // (確定タイミングで drain される設計の事後条件)。
    fn assert_every_output_frame_has_sample_entry(
        mut encoder: Openh264Encoder,
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
            if encoder.last_sample_entry.is_some() {
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
    fn openh264_sets_sample_entry_on_every_output_frame() -> crate::Result<()> {
        // OPENH264_PATH が未設定の環境ではスキップする。
        let Some(lib) = load_openh264_lib() else {
            eprintln!("OPENH264_PATH が未設定のためスキップ");
            return Ok(());
        };
        assert_every_output_frame_has_sample_entry(Openh264Encoder::new(lib, &options())?)
    }

    // 各出力フレームに載った sample_entry が、エンコーダが保持する最新値
    // (last_sample_entry) と一致することを検証する。openh264 は SPS/PPS を含むフレームでだけ
    // サンプルエントリーを作り直し、SPS/PPS を含まない P フレームには保持済みの最新値を載せる。
    // is_some だけでは「何か載っている」ことしか確認できず、保持値の伝播を検証できないため、
    // ここで実体の一致まで確認する。
    fn assert_carries_latest_sample_entry(encoder: &Openh264Encoder, frame: &VideoFrame) {
        assert!(
            frame.sample_entry.is_some(),
            "出力フレームに sample_entry が載っていない"
        );
        assert_eq!(
            frame.sample_entry.as_ref().map(|e| e.get()),
            encoder.last_sample_entry.as_ref().map(|e| e.get()),
            "出力フレームの sample_entry がエンコーダ保持の最新値と一致しない"
        );
    }

    #[test]
    fn openh264_sets_sample_entry_after_keyframe_request() -> crate::Result<()> {
        // OPENH264_PATH が未設定の環境ではスキップする。
        let Some(lib) = load_openh264_lib() else {
            eprintln!("OPENH264_PATH が未設定のためスキップ");
            return Ok(());
        };
        let mut encoder = Openh264Encoder::new(lib, &options())?;

        // 数フレームエンコードして初期状態を確定させる。最初の出力フレームで sample_entry が確定する。
        for i in 0..3u64 {
            encoder.encode(raw_i420_frame(i * 33))?;
            while let Some(frame) = encoder.next_encoded_frame() {
                assert_carries_latest_sample_entry(&encoder, &frame);
            }
        }

        // キーフレーム要求後は SPS/PPS を含むフレームで last_sample_entry が作り直され、
        // 以降の全フレーム (SPS/PPS を含まない P フレームも含む) にその保持値が伝播することを確認する。
        encoder.request_keyframe();
        for i in 3..8u64 {
            encoder.encode(raw_i420_frame(i * 33))?;
            while let Some(frame) = encoder.next_encoded_frame() {
                assert_carries_latest_sample_entry(&encoder, &frame);
            }
        }

        Ok(())
    }
}
