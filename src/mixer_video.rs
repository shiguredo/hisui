use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use orfail::OrFail;

use crate::{
    channel::{self, ErrorFlag},
    layout::Layout,
    layout_region::Region,
    metadata::SourceId,
    stats::{MixerStats, Seconds, SharedStats, VideoMixerStats, VideoResolution},
    types::{EvenUsize, PixelPosition},
    video::{VideoFormat, VideoFrame, VideoFrameReceiver, VideoFrameSyncSender},
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
pub struct VideoMixerThread {
    inputs: Vec<Input>,
    layout: Layout,
    output_tx: VideoFrameSyncSender,
    current_frames: HashMap<SourceId, ResizeCachedVideoFrame>,
    eos_source_ids: HashSet<SourceId>,
    last_mixed_frame: Option<VideoFrame>,
    last_input_update_time: Duration,
    stats: VideoMixerStats,
}

impl VideoMixerThread {
    pub fn start(
        error_flag: ErrorFlag,
        layout: Layout,
        input_rxs: Vec<VideoFrameReceiver>,
        shared_stats: SharedStats,
    ) -> VideoFrameReceiver {
        let (tx, rx) = channel::sync_channel();
        let resolution = layout.resolution;
        let mut this = Self {
            inputs: input_rxs
                .into_iter()
                .map(|rx| Input {
                    source_id: None,
                    rx,
                })
                .collect(),
            layout,
            output_tx: tx,
            current_frames: HashMap::new(),
            eos_source_ids: HashSet::new(),
            last_mixed_frame: None,
            last_input_update_time: Duration::ZERO,
            stats: VideoMixerStats {
                output_video_resolution: VideoResolution {
                    width: resolution.width().get(),
                    height: resolution.height().get(),
                },
                ..Default::default()
            },
        };
        std::thread::spawn(move || {
            log::debug!("video mixer started");
            if let Err(e) = this.run().or_fail() {
                error_flag.set();
                this.stats.error = true;
                log::error!("failed to mix video sources: {e}");
            }
            log::debug!("video mixer finished");

            shared_stats.with_lock(|stats| {
                stats.mixers.push(MixerStats::Video(this.stats));
            });
        });
        rx
    }

    fn run(&mut self) -> orfail::Result<()> {
        while let Some(frame) = self.next_output_frame().or_fail()? {
            let Some(last_frame) = self.last_mixed_frame.take() else {
                // 最初のフレームの場合はバッファに溜めて次に進む
                self.last_mixed_frame = Some(frame);
                continue;
            };
            self.last_mixed_frame = Some(frame);

            if !self.output_tx.send(last_frame) {
                // 受信側がすでに閉じている場合にはこれ以上処理しても仕方がないので終了する
                log::info!("receiver of mixed video stream has been closed");
                return Ok(());
            }
        }

        // バッファに残っていた最後のフレームを送る
        if let Some(last_frame) = self.last_mixed_frame.take()
            && !self.output_tx.send(last_frame)
        {
            log::info!("receiver of mixed video stream has been closed");
        }

        Ok(())
    }

    // フレーム数に対応するタイムスタンプを求める
    fn frames_to_timestamp(&self, frames: u64) -> Duration {
        Duration::from_secs(frames * self.layout.frame_rate.denumerator.get() as u64)
            / self.layout.frame_rate.numerator.get() as u32
    }

    fn next_input_timestamp(&self) -> Duration {
        self.frames_to_timestamp(
            self.stats.total_output_video_frame_count
                + self.stats.total_extended_video_frame_count
                + self.stats.total_trimmed_video_frame_count,
        )
    }

    fn next_output_timestamp(&self) -> Duration {
        self.frames_to_timestamp(
            self.stats.total_output_video_frame_count + self.stats.total_extended_video_frame_count,
        )
    }

    fn next_output_duration(&self) -> Duration {
        // 丸め誤差が蓄積しないように次のフレームのタイスタンプとの差をとる
        self.frames_to_timestamp(
            self.stats.total_output_video_frame_count
                + self.stats.total_extended_video_frame_count
                + 1,
        ) - self.next_output_timestamp()
    }

    // 必要に応じて、現在の合成対象となっているフレーム群を更新する
    fn maybe_update_current_frame(
        &mut self,
        now: Duration,
        input: &mut Input,
    ) -> orfail::Result<()> {
        while let Some(frame) = input.rx.peek() {
            if now < frame.timestamp {
                // この入力フレームは、まだ表示時刻に達していない
                break;
            }

            let frame = input.rx.recv().or_fail()?;
            let source_id = frame.source_id.clone().or_fail()?;
            if input.source_id.is_none() {
                input.source_id = Some(source_id.clone());
            }
            self.current_frames
                .insert(source_id, ResizeCachedVideoFrame::new(frame));
            self.last_input_update_time = now;
            self.stats.total_input_video_frame_count += 1;
        }
        Ok(())
    }

    fn next_output_frame(&mut self) -> orfail::Result<Option<VideoFrame>> {
        loop {
            let mut now = self.next_input_timestamp();

            // トリム対象期間ならその分はスキップする
            while self.layout.is_in_trim_span(now) {
                self.stats.total_trimmed_video_frame_count += 1;
                now = self.next_input_timestamp();
            }

            // EOS に到達したソースの最後のフレームは、その表示時刻を過ぎたら破棄する
            //
            // なお、EOS ではないソースの場合は、仮に途中のフレーム間のギャップがあったとしても、
            // 破棄はせずに連続しているものとして扱う
            // (入力ファイル作成時の数値計算誤差などで、僅かなギャップが生じたとしても、
            //  その部分を黒塗りにしたくはないため)
            self.current_frames.retain(|source_id, f| {
                if !self.eos_source_ids.contains(source_id) {
                    // まだ EOS に達していない
                    return true;
                }
                if now < f.end_timestamp() {
                    // まだ表示時刻に収まっている
                    return true;
                }

                self.last_input_update_time = now;
                false
            });

            // 表示対象のフレームを更新する
            for mut input in std::mem::take(&mut self.inputs) {
                self.maybe_update_current_frame(now, &mut input).or_fail()?;
                if input.rx.peek().is_some() {
                    // この入力（ソース）にはまだフレームが残っている
                    self.inputs.push(input);
                } else if let Some(source_id) = input.source_id {
                    // EOS に達したソースを覚えておく（最終フレーム破棄判定用)
                    self.eos_source_ids.insert(source_id);
                } else {
                    // 一つも映像フレームを受信せずに EOS に達したソースがある場合はここに来る
                    // (特に何もする必要はない)
                }
            }

            if self.inputs.is_empty() && self.current_frames.is_empty() {
                // 全部の入力フレームを処理した
                return Ok(None);
            }

            let elapsed_since_last_input = now.saturating_sub(self.last_input_update_time);
            if elapsed_since_last_input > TIMESTAMP_GAP_THRESHOLD {
                (elapsed_since_last_input <= TIMESTAMP_GAP_ERROR_THRESHOLD)
                    .or_fail_with(|()| "too large timestamp gap".to_owned())?;

                // 一定期間、入力の更新がない場合には、合成ではなく一つ前のフレームの尺を調整することで対応する
                let duration = self.next_output_duration();
                let last_frame = self.last_mixed_frame.as_mut().expect("infallible");

                last_frame.duration += duration;
                self.stats.total_extended_video_frame_count += 1;
                self.stats.total_output_video_frame_seconds += duration; // 出力フレーム数は増えないけど尺は伸びる

                continue;
            }

            // 現在のフレームを合成する
            let (result, elapsed) = Seconds::elapsed(|| self.mix().or_fail());
            self.stats.total_processing_seconds += elapsed;
            return result.map(Some);
        }
    }

    fn mix(&mut self) -> orfail::Result<VideoFrame> {
        let timestamp = self.next_output_timestamp();
        let duration = self.next_output_duration();

        let mut canvas = Canvas::new(
            self.layout.resolution.width(),
            self.layout.resolution.height(),
        );

        for region in &self.layout.video_regions {
            Self::mix_region(&mut canvas, region, &mut self.current_frames).or_fail()?;
        }

        self.stats.total_output_video_frame_count += 1;
        self.stats.total_output_video_frame_seconds += duration;

        Ok(VideoFrame {
            // 固定値
            source_id: None,    // 合成後は常に None となる
            sample_entry: None, // 生データにはエンプルエントリーは存在しない
            keyframe: true,     // 生データはすべてキーフレーム扱い
            format: VideoFormat::I420,

            // 可変値
            timestamp,
            duration,
            width: self.layout.resolution.width(),
            height: self.layout.resolution.height(),
            data: canvas.data,
        })
    }

    fn mix_region(
        canvas: &mut Canvas,
        region: &Region,
        current_frames: &mut HashMap<SourceId, ResizeCachedVideoFrame>,
    ) -> orfail::Result<()> {
        // [NOTE] ここで実質的にやりたいのは外枠を引くことだけなので、リージョン全体を塗りつぶすのは少し過剰
        //        (必要に応じて最適化する)
        let background_frame =
            VideoFrame::mono_color(region.background_color, region.width, region.height);
        canvas
            .draw_frame(region.position, &background_frame)
            .or_fail()?;

        let mut frames = Vec::new();
        for (source_id, frame) in current_frames {
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
}

#[derive(Debug)]
struct ResizeCachedVideoFrame {
    original: VideoFrame,
    resized: Vec<((EvenUsize, EvenUsize), VideoFrame)>, // (width, height) => resized frame
}

impl ResizeCachedVideoFrame {
    fn new(original: VideoFrame) -> Self {
        Self {
            original,
            resized: Vec::new(),
        }
    }

    fn end_timestamp(&self) -> Duration {
        self.original.end_timestamp()
    }

    fn resize(&mut self, width: EvenUsize, height: EvenUsize) -> orfail::Result<&VideoFrame> {
        if self.original.width == width && self.original.height == height {
            // リサイズ不要
            return Ok(&self.original);
        }

        // [NOTE]
        // resized の要素数は、通常は 1 で多くても 2~3 である想定なので線形探索で十分
        if self
            .resized
            .iter()
            .find(|x| x.0 == (width, height))
            .is_none()
        {
            // キャッシュにないので新規リサイズが必要
            let mut frame = self.original.clone();
            frame.resize(width, height).or_fail()?;
            self.resized.push(((width, height), frame));
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
struct Input {
    source_id: Option<SourceId>,
    rx: channel::Receiver<VideoFrame>,
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
        (frame.width <= self.width).or_fail()?;
        (frame.height <= self.height).or_fail()?;

        // Y成分の描画
        let offset_x = position.x.get();
        let offset_y = position.y.get();
        let y_size = frame.width.get() * frame.height.get();
        let y_data = &frame.data[..y_size];
        for y in 0..frame.height.get() {
            let i = y * frame.width.get();
            self.draw_y_line(offset_x, offset_y + y, &y_data[i..][..frame.width.get()]);
        }

        // U成分の描画
        let offset_x = position.x.get() / 2;
        let offset_y = position.y.get() / 2;
        let u_size = (frame.width.get() / 2) * (frame.height.get() / 2);
        let u_data = &frame.data[y_size..][..u_size];
        for y in 0..frame.height.get() / 2 {
            let i = y * (frame.width.get() / 2);
            self.draw_u_line(
                offset_x,
                offset_y + y,
                &u_data[i..][..frame.width.get() / 2],
            );
        }

        // V成分の描画
        let v_data = &frame.data[y_size + u_size..][..u_size];
        for y in 0..frame.height.get() / 2 {
            let i = y * (frame.width.get() / 2);
            self.draw_v_line(
                offset_x,
                offset_y + y,
                &v_data[i..][..frame.width.get() / 2],
            );
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
