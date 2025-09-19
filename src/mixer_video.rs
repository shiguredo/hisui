use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::Duration,
};

use orfail::OrFail;

use crate::{
    layout::{Layout, Resolution, TrimSpans},
    layout_region::Region,
    media::MediaStreamId,
    metadata::SourceId,
    processor::{
        MediaProcessor, MediaProcessorInput, MediaProcessorOutput, MediaProcessorSpec,
        MediaProcessorWorkloadHint,
    },
    stats::{ProcessorStats, VideoMixerStats, VideoResolution},
    types::{EvenUsize, PixelPosition},
    video::{FrameRate, VideoFormat, VideoFrame},
};

// 入力がこの期間更新されなかった場合には、以後は合成ではなく
// 一つ前の合成フレームの尺を伸ばすことで対処する
// (入力のバグによって、非現実的な数のフレームが合成されるのを防ぐため）
//
// 値は適当なので必要に応じて調整すること
const TIMESTAMP_GAP_THRESHOLD: Duration = Duration::from_secs(60);

// 入力がこの期間更新されなかった場合には、尺調整ではなくエラーとする（最終的な安全弁）
//
// 値は適当なので必要に応じて調整すること
const TIMESTAMP_GAP_ERROR_THRESHOLD: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug)]
struct ResizeCachedVideoFrame {
    original: Arc<VideoFrame>,
    resized: Vec<((EvenUsize, EvenUsize), VideoFrame)>, // (width, height) => resized frame
    resize_filter_mode: shiguredo_libyuv::FilterMode,
}

impl ResizeCachedVideoFrame {
    fn new(original: Arc<VideoFrame>, resize_filter_mode: shiguredo_libyuv::FilterMode) -> Self {
        Self {
            original,
            resized: Vec::new(),
            resize_filter_mode,
        }
    }

    fn start_timestamp(&self) -> Duration {
        self.original.timestamp
    }

    fn end_timestamp(&self) -> Duration {
        self.original.end_timestamp()
    }

    fn source_id(&self) -> Option<&SourceId> {
        self.original.source_id.as_ref()
    }

    fn resize(&mut self, width: EvenUsize, height: EvenUsize) -> orfail::Result<&VideoFrame> {
        if self.original.width == width.get() && self.original.height == height.get() {
            // リサイズ不要
            return Ok(&self.original);
        }

        // [NOTE]
        // resized の要素数は、通常は 1 で多くても 2~3 である想定なので線形探索で十分
        if !self.resized.iter().any(|x| x.0 == (width, height)) {
            // キャッシュにないので新規リサイズが必要
            if let Some(resized) = self
                .original
                .resize(width, height, self.resize_filter_mode)
                .or_fail()?
            {
                self.resized.push(((width, height), resized));
            }
        }

        // キャッシュから対応するサイズのフレームを取得する
        let cached = self
            .resized
            .iter()
            .find_map(|x| (x.0 == (width, height)).then_some(&x.1))
            .expect("infallible");
        Ok(cached)
    }
}

#[derive(Debug)]
struct Canvas {
    width: EvenUsize,
    height: EvenUsize,
    data: Vec<u8>,
}

impl Canvas {
    fn new(width: EvenUsize, height: EvenUsize) -> Self {
        Self {
            width,
            height,
            data: VideoFrame::black(width, height).data,
        }
    }

    fn draw_frame(&mut self, position: PixelPosition, frame: &VideoFrame) -> orfail::Result<()> {
        (frame.format == VideoFormat::I420).or_fail()?;
        (frame.width <= self.width.get()).or_fail()?;
        (frame.height <= self.height.get()).or_fail()?;

        // セルの解像度は偶数前提なので、奇数になることはない
        // (入力が奇数の場合でもリサイズによって常に偶数解像度になる）
        (frame.width % 2 == 0).or_fail()?;
        (frame.height % 2 == 0).or_fail()?;

        // Y成分の描画
        let offset_x = position.x.get();
        let offset_y = position.y.get();
        let y_size = frame.width * frame.height;
        let y_data = &frame.data[..y_size];
        for y in 0..frame.height {
            let i = y * frame.width;
            self.draw_y_line(offset_x, offset_y + y, &y_data[i..][..frame.width]);
        }

        // U成分の描画
        let offset_x = position.x.get() / 2;
        let offset_y = position.y.get() / 2;
        let u_size = (frame.width / 2) * (frame.height / 2);
        let u_data = &frame.data[y_size..][..u_size];
        for y in 0..frame.height / 2 {
            let i = y * (frame.width / 2);
            self.draw_u_line(offset_x, offset_y + y, &u_data[i..][..frame.width / 2]);
        }

        // V成分の描画
        let v_data = &frame.data[y_size + u_size..][..u_size];
        for y in 0..frame.height / 2 {
            let i = y * (frame.width / 2);
            self.draw_v_line(offset_x, offset_y + y, &v_data[i..][..frame.width / 2]);
        }

        Ok(())
    }

    fn draw_y_line(&mut self, x: usize, y: usize, line: &[u8]) {
        let i = x + y * self.width.get();
        self.data[i..][..line.len()].copy_from_slice(line);
    }

    fn draw_u_line(&mut self, x: usize, y: usize, line: &[u8]) {
        let y_size = self.width.get() * self.height.get();
        let i = x + y * (self.width.get() / 2);
        let offset = y_size + i;
        self.data[offset..][..line.len()].copy_from_slice(line);
    }

    fn draw_v_line(&mut self, x: usize, y: usize, line: &[u8]) {
        let y_size = self.width.get() * self.height.get();
        let u_size = (self.width.get() / 2) * (self.height.get() / 2);
        let i = x + y * (self.width.get() / 2);
        let offset = y_size + u_size + i;
        self.data[offset..][..line.len()].copy_from_slice(line);
    }
}

#[derive(Debug, Clone)]
pub struct VideoMixerSpec {
    pub regions: Vec<Region>,
    pub frame_rate: FrameRate,
    pub resolution: Resolution,
    pub trim_spans: TrimSpans,
    pub resize_filter_mode: shiguredo_libyuv::FilterMode,
}

impl VideoMixerSpec {
    pub fn from_layout(layout: &Layout) -> Self {
        Self {
            regions: layout.video_regions.clone(),
            frame_rate: layout.frame_rate,
            resolution: layout.resolution,
            trim_spans: layout.trim_spans.clone(),
            resize_filter_mode: shiguredo_libyuv::FilterMode::Box,
        }
    }
}

#[derive(Debug)]
pub struct VideoMixer {
    spec: VideoMixerSpec,
    input_streams: HashMap<MediaStreamId, InputStream>,
    output_stream_id: MediaStreamId,
    last_mixed_frame: Option<VideoFrame>,
    stats: VideoMixerStats,
}

impl VideoMixer {
    pub fn new(
        spec: VideoMixerSpec,
        input_stream_ids: Vec<MediaStreamId>,
        output_stream_id: MediaStreamId,
    ) -> Self {
        let resolution = spec.resolution;
        Self {
            spec,
            input_streams: input_stream_ids
                .into_iter()
                .map(|id| (id, InputStream::default()))
                .collect(),
            output_stream_id,
            last_mixed_frame: None,
            stats: VideoMixerStats {
                output_video_resolution: VideoResolution {
                    width: resolution.width().get(),
                    height: resolution.height().get(),
                },
                ..Default::default()
            },
        }
    }

    pub fn stats(&self) -> &VideoMixerStats {
        &self.stats
    }

    fn next_input_timestamp(&self) -> Duration {
        self.frames_to_timestamp(
            self.stats.total_output_video_frame_count.get()
                + self.stats.total_extended_video_frame_count.get()
                + self.stats.total_trimmed_video_frame_count.get(),
        )
    }

    // フレーム数に対応するタイムスタンプを求める
    fn frames_to_timestamp(&self, frames: u64) -> Duration {
        Duration::from_secs(frames * self.spec.frame_rate.denumerator.get() as u64)
            / self.spec.frame_rate.numerator.get() as u32
    }

    fn next_output_timestamp(&self) -> Duration {
        self.frames_to_timestamp(
            self.stats.total_output_video_frame_count.get()
                + self.stats.total_extended_video_frame_count.get(),
        )
    }

    fn next_output_duration(&self) -> Duration {
        // 丸め誤差が蓄積しないように次のフレームのタイスタンプとの差をとる
        self.frames_to_timestamp(
            self.stats.total_output_video_frame_count.get()
                + self.stats.total_extended_video_frame_count.get()
                + 1,
        ) - self.next_output_timestamp()
    }

    fn mix(&mut self, now: Duration) -> orfail::Result<VideoFrame> {
        let timestamp = self.next_output_timestamp();
        let duration = self.next_output_duration();

        let mut canvas = Canvas::new(self.spec.resolution.width(), self.spec.resolution.height());

        for region in &self.spec.regions {
            Self::mix_region(&mut canvas, region, &mut self.input_streams, now).or_fail()?;
        }

        self.stats.total_output_video_frame_count.add(1);
        self.stats.total_output_video_frame_duration.add(duration);

        Ok(VideoFrame {
            // 固定値
            source_id: None,    // 合成後は常に None となる
            sample_entry: None, // 生データにはエンプルエントリーは存在しない
            keyframe: true,     // 生データはすべてキーフレーム扱い
            format: VideoFormat::I420,

            // 可変値
            timestamp,
            duration,
            width: self.spec.resolution.width().get(),
            height: self.spec.resolution.height().get(),
            data: canvas.data,
        })
    }

    fn mix_region(
        canvas: &mut Canvas,
        region: &Region,
        input_streams: &mut HashMap<MediaStreamId, InputStream>,
        now: Duration,
    ) -> orfail::Result<()> {
        // [NOTE] ここで実質的にやりたいのは外枠を引くことだけなので、リージョン全体を塗りつぶすのは少し過剰
        //        (必要に応じて最適化する)
        let background_frame =
            VideoFrame::mono_color(region.background_color, region.width, region.height);
        canvas
            .draw_frame(region.position, &background_frame)
            .or_fail()?;

        let mut frames = Vec::new();
        for input_stream in input_streams.values_mut() {
            let Some(frame) = input_stream.frame_queue.front_mut() else {
                continue;
            };
            if now < frame.start_timestamp() {
                continue;
            }
            let Some(source_id) = frame.source_id() else {
                continue;
            };
            let Some(source) = region.grid.assigned_sources.get(source_id) else {
                // このグリッドには含まれないソースなので飛ばす
                continue;
            };
            frames.push((source, frame));
        }

        // 同じセルに割りあてられたソースが複数ある場合には
        // 一番優先度が高いものを採用する
        frames.sort_by_key(|(s, _)| (s.cell_index, s.priority));
        frames.dedup_by_key(|(s, _)| s.cell_index);

        for (source, frame) in frames {
            let mut position = region.cell_position(source.cell_index);
            let (frame_width, frame_height) = region.decide_frame_size(&frame.original);
            position.x +=
                EvenUsize::truncating_new((region.grid.cell_width - frame_width).get() / 2);
            position.y +=
                EvenUsize::truncating_new((region.grid.cell_height - frame_height).get() / 2);
            let resized_frame = frame.resize(frame_width, frame_height).or_fail()?;
            canvas.draw_frame(position, resized_frame).or_fail()?;
        }

        Ok(())
    }

    fn gap_until_next_frame_change(&self, now: Duration) -> Duration {
        let mut next_start_timestamp = Duration::MAX;
        for input_stream in self.input_streams.values() {
            let Some((current, next)) = input_stream
                .frame_queue
                .front()
                .zip(input_stream.frame_queue.get(1))
            else {
                continue;
            };
            if now < current.start_timestamp() {
                continue;
            }
            next_start_timestamp = next_start_timestamp.min(next.start_timestamp());
        }
        if next_start_timestamp == Duration::MAX {
            Duration::ZERO
        } else {
            next_start_timestamp.saturating_sub(now)
        }
    }
}

impl MediaProcessor for VideoMixer {
    fn spec(&self) -> MediaProcessorSpec {
        MediaProcessorSpec {
            input_stream_ids: self.input_streams.keys().copied().collect(),
            output_stream_ids: vec![self.output_stream_id],
            stats: ProcessorStats::VideoMixer(self.stats.clone()),
            workload_hint: MediaProcessorWorkloadHint::VIDEO_MIXER,
        }
    }

    fn process_input(&mut self, input: MediaProcessorInput) -> orfail::Result<()> {
        let input_stream = self.input_streams.get_mut(&input.stream_id).or_fail()?;
        if let Some(sample) = input.sample {
            // キューに要素を追加する
            let frame = sample.expect_video_frame().or_fail()?;
            (frame.format == VideoFormat::I420).or_fail()?;

            input_stream
                .frame_queue
                .push_back(ResizeCachedVideoFrame::new(
                    frame,
                    self.spec.resize_filter_mode,
                ));
            self.stats.total_input_video_frame_count.add(1);
        } else {
            input_stream.eos = true;
        }
        Ok(())
    }

    fn process_output(&mut self) -> orfail::Result<MediaProcessorOutput> {
        loop {
            let mut now = self.next_input_timestamp();

            // トリム対象期間ならその分はスキップする
            while self.spec.trim_spans.contains(now) {
                self.stats.total_trimmed_video_frame_count.add(1);
                now = self.next_input_timestamp();
            }

            // 表示対象のフレームを更新する
            for (input_stream_id, input_stream) in &mut self.input_streams {
                if !input_stream.pop_outdated_frame(now) {
                    // 十分な数のフレームが溜まっていないので入力を待つ
                    return Ok(MediaProcessorOutput::pending(*input_stream_id));
                }
            }

            // EOS 判定
            if self
                .input_streams
                .values()
                .all(|s| s.eos && s.frame_queue.is_empty())
            {
                if let Some(frame) = self.last_mixed_frame.take() {
                    // バッファにフレームが残っていたらそれを返す
                    return Ok(MediaProcessorOutput::video_frame(
                        self.output_stream_id,
                        frame,
                    ));
                }
                return Ok(MediaProcessorOutput::Finished);
            }

            // 入力のタイムスタンプに極端なギャップがある場合の対応
            let gap = self.gap_until_next_frame_change(now);
            if gap > TIMESTAMP_GAP_THRESHOLD {
                (gap <= TIMESTAMP_GAP_ERROR_THRESHOLD)
                    .or_fail_with(|()| "too large timestamp gap".to_owned())?;

                // 一定期間、入力の更新がない場合には、合成ではなく一つ前のフレームの尺を調整することで対応する
                let duration = self.next_output_duration();
                let last_frame = self.last_mixed_frame.as_mut().expect("infallible");

                last_frame.duration += duration;
                self.stats.total_extended_video_frame_count.add(1);
                self.stats.total_output_video_frame_duration.add(duration); // 出力フレーム数は増えないけど尺は伸びる

                continue;
            }

            // 現在のフレームを合成する
            let mixed_frame = self.mix(now).or_fail()?;

            if let Some(frame) = self.last_mixed_frame.replace(mixed_frame) {
                return Ok(MediaProcessorOutput::video_frame(
                    self.output_stream_id,
                    frame,
                ));
            }
        }
    }
}

#[derive(Debug, Default)]
struct InputStream {
    eos: bool,
    frame_queue: VecDeque<ResizeCachedVideoFrame>,
}

impl InputStream {
    fn pop_outdated_frame(&mut self, now: Duration) -> bool {
        loop {
            let Some(current_frame) = self.frame_queue.front() else {
                if self.eos {
                    // EOS & キューにフレームが残っていない
                    return true;
                } else {
                    // EOS ではないけどキューが空なので入力待ち
                    return false;
                }
            };
            if now < current_frame.end_timestamp() {
                // まだ現在のフレームの表示時刻範囲内
                return true;
            }

            let Some(next_frame) = self.frame_queue.get(1) else {
                if self.eos {
                    // EOS に到達し、表示時刻も超過した
                    self.frame_queue.clear();
                    return true;
                } else {
                    // EOS ではないなら、現在のフレームの破棄タイミングを判断するために次の入力を待つ
                    return false;
                }
            };
            if now < next_frame.start_timestamp() {
                // まだ現在のフレームの表示時刻範囲内
                //
                // [NOTE]
                // 前のフレームの表示時刻を過ぎていても、
                // 次のフレームの表示時刻に到達するまでは前のフレームを使い続ける
                //
                // これは、入力ファイル作成時の数値計算誤差などで、
                // 僅かなギャップが生じたとしても、その部分を黒塗りにしたくはないため
                return true;
            }

            // 次のフレームの表示時刻になった
            //
            // なお、入力 FPS よりも出力 FPS の方が小さい場合には、
            // 次のフレームがすぐに破棄される可能性もあるので、
            // ここで終わりにしないで、ループしている
            self.frame_queue.pop_front();
        }
    }
}
