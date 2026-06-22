use std::collections::VecDeque;

use crate::{
    encoder::VideoEncoderOptions,
    sample_entry::SharedSampleEntry,
    types::{CodecName, EvenUsize},
    video::h264,
    video::h265,
    video::{FrameRate, RawVideoFrame, VideoFormat, VideoFrame, VideoFrameSize},
};

#[derive(Debug)]
pub struct VideoToolboxEncoder {
    inner: shiguredo_video_toolbox::Encoder,
    input_queue: VecDeque<RawVideoFrame>,
    output_queue: VecDeque<VideoFrame>,
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

    pub fn finish(&mut self) -> crate::Result<()> {
        self.inner.finish()?;
        self.handle_encoded()?;
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
            // shiguredo_video_toolbox は keyframe 出力時のみ VPS / SPS / PPS を返すため、
            // 非 keyframe フレームでは frame.vps_list / sps_list / pps_list が空になる。
            // H.264 / H.265 のどちらの経路でも空入力でサンプルエントリー構築をスキップし、
            // 次の keyframe を待つ (空入力で新ヘルパー関数を呼ぶと SPS パースで Err になり
            // エンコーダが落ちるため)。
            if self.sample_entry.is_none() {
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
                } else if frame.vps_list.is_empty()
                    || frame.sps_list.is_empty()
                    || frame.pps_list.is_empty()
                {
                    None
                } else {
                    let (entry, _frame_size) = h265::h265_sample_entry_from_vps_sps_pps_lists(
                        frame.vps_list.clone(),
                        frame.sps_list.clone(),
                        frame.pps_list.clone(),
                        self.fps,
                    )?;
                    Some(entry)
                };
                if let Some(sample_entry) = sample_entry_opt {
                    self.sample_entry = Some(SharedSampleEntry::new(sample_entry));
                }
            }

            self.output_queue.push_back(VideoFrame {
                data: frame.data,
                format: self.format,
                keyframe: frame.keyframe,
                size: Some(VideoFrameSize {
                    width: self.width.get(),
                    height: self.height.get(),
                }),
                timestamp: input_frame.as_video_frame().timestamp,
                sample_entry: self.sample_entry.clone(),
            });
        }
        Ok(())
    }
}
