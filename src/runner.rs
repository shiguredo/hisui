use std::{collections::HashSet, time::Instant};

use indicatif::{ProgressBar, ProgressStyle};
use orfail::OrFail;
use shiguredo_openh264::Openh264Library;

use crate::{
    audio::AudioDataReceiver,
    channel::{self, ErrorFlag},
    command_line_args::Args,
    decoder::{VideoDecoder, VideoDecoderOptions},
    encoder::{AudioEncoder, AudioEncoderThread, VideoEncoder, VideoEncoderThread},
    layout::Layout,
    metadata::{ContainerFormat, RecordingMetadata},
    mixer_audio::AudioMixerThread,
    mixer_video::VideoMixerThread,
    source::{AudioSourceThread, VideoSourceThread},
    stats::{Seconds, SharedStats, WriterStats},
    types::CodecName,
    video::VideoFrameReceiver,
    writer_mp4::Mp4Writer,
};

#[derive(Debug)]
pub struct Runner {
    args: Args,
}

impl Runner {
    pub fn new(args: Args) -> Self {
        Self { args }
    }

    pub fn run(&mut self) -> orfail::Result<()> {
        // 利用する CPU コア数を制限する
        if let Some(cores) = self.args.cpu_cores {
            limit_cpu_cores(cores).or_fail()?;
        }

        // 統計情報の準備
        // （コストは高くないので、簡潔さを優先して統計を出力しない場合でも統計値のカウントは常に行うようにしている）
        let stats = SharedStats::new();
        let start_time = Instant::now();

        // レイアウトを準備
        let layout = self.create_layout().or_fail()?;
        log::debug!("layout: {layout:?}");

        // 必要に応じて openh264 の共有ライブラリを読み込む
        let openh264_lib =
            if let Some(path) = self.args.openh264.as_ref().filter(|_| layout.has_video()) {
                Some(Openh264Library::load(path).or_fail()?)
            } else {
                None
            };

        // 映像および音声ソースの準備
        let error_flag = ErrorFlag::new();
        let (audio_source_rxs, video_source_rxs) = create_audio_and_video_sources(
            &layout,
            error_flag.clone(),
            stats.clone(),
            openh264_lib.clone(),
        )
        .or_fail()?;

        // プログレスバーを準備
        let progress_bar = self.create_progress_bar(&layout);

        // 変換後のファイルのパスを決定
        let out_file_path = if let Some(path) = self.args.out_file.clone() {
            path
        } else if !layout.has_video() && layout.has_audio() {
            layout.base_path.join("output.mp4a")
        } else {
            layout.base_path.join("output.mp4")
        };

        // 映像ミキサーとエンコーダーを準備
        let encoded_video_rx = self
            .create_video_mixer_and_encoder(
                &layout,
                openh264_lib.clone(),
                error_flag.clone(),
                stats.clone(),
                video_source_rxs,
            )
            .or_fail()?;

        // 音声ミキサーとエンコーダーを準備
        let encoded_audio_rx = self
            .create_audio_mixer_and_encoder(
                &layout,
                error_flag.clone(),
                stats.clone(),
                audio_source_rxs,
            )
            .or_fail()?;

        // 合成後の映像と音声への MP4 への書き出しを行う
        // この処理以降にやることはないので、メインスレッドで実行してしまう
        let mut mp4_writer =
            Mp4Writer::new(&out_file_path, &layout, encoded_audio_rx, encoded_video_rx)
                .or_fail()?;

        while let Some(timestamp) = mp4_writer.poll().or_fail()? {
            progress_bar.set_position(timestamp.as_secs());
            if error_flag.get() {
                // ファイル読み込み、デコード、合成、エンコード、のいずれかで失敗したものがあるとここに来る
                log::error!("The composition process was aborted");
                break;
            }
        }

        // 全ての処理が完了したので、プログレスバーと統計処理の後始末を行う
        progress_bar.finish();
        self.finish_stats(stats, &mp4_writer, start_time);

        if error_flag.get() {
            // エラー発生時は終了コードを変える
            std::process::exit(1);
        }

        Ok(())
    }

    fn create_layout(&self) -> orfail::Result<Layout> {
        if let Some(layout_file_path) = &self.args.layout {
            let layout_json = std::fs::read_to_string(layout_file_path)
                .or_fail_with(|e| format!("failed to read {}: {e}", layout_file_path.display()))?;
            Layout::from_layout_json(
                layout_file_path,
                &layout_json,
                self.args.out_video_frame_rate,
            )
            .or_fail()
        } else if let Some(report_file_path) = &self.args.in_metadata_file {
            let report = RecordingMetadata::from_file(report_file_path).or_fail()?;
            log::debug!("loaded recording report: {report:?}");
            Layout::from_recording_report(
                report_file_path,
                &report,
                self.args.audio_only,
                self.args.max_columns.get(),
                self.args.out_video_frame_rate,
            )
            .or_fail()
        } else {
            // 引数バリデーションによってここには来ない
            unreachable!()
        }
    }

    fn create_progress_bar(&self, layout: &Layout) -> ProgressBar {
        let progress_bar = if self.args.show_progress_bar {
            ProgressBar::new(layout.duration().as_secs())
        } else {
            ProgressBar::hidden()
        };
        progress_bar.set_style(
        ProgressStyle::default_bar()
            .template(
                "{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len}s ({eta})",
            )
            .unwrap()
            .progress_chars("#>-"),
    );
        progress_bar
    }

    fn create_video_mixer_and_encoder(
        &self,
        layout: &Layout,
        openh264_lib: Option<Openh264Library>,
        error_flag: ErrorFlag,
        stats: SharedStats,
        video_source_rxs: Vec<VideoFrameReceiver>,
    ) -> orfail::Result<VideoFrameReceiver> {
        if !layout.has_video() {
            // 映像が処理対象外の場合には、ダミーのレシーバーを返す
            let (_, rx) = channel::sync_channel();
            return Ok(rx);
        }

        let mixed_video_rx = VideoMixerThread::start(
            error_flag.clone(),
            layout.clone(),
            video_source_rxs,
            stats.clone(),
        );

        let encoder = match self.args.out_video_codec {
            CodecName::Vp8 => VideoEncoder::new_vp8(
                &layout,
                self.args.libvpx_cq_level,
                self.args.libvpx_min_q,
                self.args.libvpx_max_q,
            )
            .or_fail()?,
            CodecName::Vp9 => VideoEncoder::new_vp9(
                &layout,
                self.args.libvpx_cq_level,
                self.args.libvpx_min_q,
                self.args.libvpx_max_q,
            )
            .or_fail()?,
            #[cfg(target_os = "macos")]
            CodecName::H264 if openh264_lib.is_none() => {
                // openh264 が明示的に指定されている場合にはそちらを優先する
                VideoEncoder::new_video_toolbox_h264(&layout).or_fail()?
            }
            CodecName::H264 => {
                let lib = openh264_lib.or_fail()?;
                VideoEncoder::new_openh264(lib, &layout).or_fail()?
            }
            #[cfg(target_os = "macos")]
            CodecName::H265 => VideoEncoder::new_video_toolbox_h265(&layout).or_fail()?,
            #[cfg(not(target_os = "macos"))]
            CodecName::H265 => {
                return Err(orfail::Failure::new("no available H.265 encoder"));
            }
            CodecName::Av1 => VideoEncoder::new_svt_av1(&layout).or_fail()?,
            _ => unreachable!(),
        };
        let encoded_video_rx =
            VideoEncoderThread::start(error_flag.clone(), mixed_video_rx, encoder, stats.clone());
        Ok(encoded_video_rx)
    }

    fn create_audio_mixer_and_encoder(
        &self,
        layout: &Layout,
        error_flag: ErrorFlag,
        stats: SharedStats,
        audio_source_rxs: Vec<AudioDataReceiver>,
    ) -> orfail::Result<AudioDataReceiver> {
        if !layout.has_audio() {
            // 音声が処理対象外の場合には、ダミーのレシーバーを返す
            let (_, rx) = channel::sync_channel();
            return Ok(rx);
        }

        let mixed_audio_rx = AudioMixerThread::start(
            error_flag.clone(),
            layout.clone(),
            audio_source_rxs,
            stats.clone(),
        );

        let audio_encoder = match self.args.out_audio_codec {
            #[cfg(feature = "fdk-aac")]
            CodecName::Aac => AudioEncoder::new_fdk_aac(self.args.out_aac_bit_rate).or_fail()?,
            #[cfg(all(not(feature = "fdk-aac"), target_os = "macos"))]
            CodecName::Aac => {
                AudioEncoder::new_audio_toolbox_aac(self.args.out_aac_bit_rate).or_fail()?
            }
            #[cfg(all(not(feature = "fdk-aac"), not(target_os = "macos")))]
            CodecName::Aac => {
                return Err(orfail::Failure::new("AAC output is not supported"));
            }
            CodecName::Opus => AudioEncoder::new_opus(self.args.out_opus_bit_rate).or_fail()?,
            _ => unreachable!(),
        };
        let encoded_audio_rx = AudioEncoderThread::start(
            error_flag.clone(),
            mixed_audio_rx,
            audio_encoder,
            stats.clone(),
        );
        Ok(encoded_audio_rx)
    }

    fn finish_stats(&self, stats: SharedStats, mp4_writer: &Mp4Writer, start_time: Instant) {
        stats.with_lock(|stats| {
            stats.elapsed_seconds = Seconds::new(start_time.elapsed());
            stats
                .writers
                .push(WriterStats::Mp4(mp4_writer.stats().clone()));
        });
        stats.with_lock(|stats| {
            log::debug!("stats: {}", nojson::Json(&stats));

            if let Some(path) = &self.args.out_stats_file {
                // 統計が出力できなくても全体を失敗扱いにはしない

                let json = nojson::json(|f| {
                    f.set_indent_size(2);
                    f.set_spacing(true);
                    f.value(&stats)
                })
                .to_string();
                if let Err(e) = std::fs::write(path, json) {
                    log::warn!(
                        "failed to write stats JSON: path={}, reason={e}",
                        path.display()
                    );
                }
            }
        });
    }
}

#[cfg(target_os = "macos")]
fn limit_cpu_cores(_cores: usize) -> orfail::Result<()> {
    // MacOS ではコア数制限はできない
    log::warn!("`--cpu-cores` option is ignored on MacOS");
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn limit_cpu_cores(cores: usize) -> orfail::Result<()> {
    unsafe {
        let mut cpu_set = std::mem::MaybeUninit::zeroed().assume_init();
        libc::CPU_ZERO(&mut cpu_set);

        for i in 0..cores {
            libc::CPU_SET(i, &mut cpu_set);
        }

        let pid = libc::getpid();
        (libc::sched_setaffinity(pid, std::mem::size_of::<libc::cpu_set_t>(), &cpu_set) == 0)
            .or_fail_with(|()| {
                format!(
                    "Failed to set CPU affinity: {}",
                    std::io::Error::last_os_error()
                )
            })?;
    }
    Ok(())
}

fn create_audio_and_video_sources(
    layout: &Layout,
    error_flag: ErrorFlag,
    stats: SharedStats,
    openh264_lib: Option<Openh264Library>,
) -> orfail::Result<(Vec<AudioDataReceiver>, Vec<VideoFrameReceiver>)> {
    let audio_source_ids = layout.audio_source_ids().collect::<HashSet<_>>();
    let video_source_ids = layout.video_source_ids().collect::<HashSet<_>>();

    let mut audio_source_rxs = Vec::new();
    let mut video_source_rxs = Vec::new();
    for (source_id, source_info) in &layout.sources {
        if audio_source_ids.contains(source_id) && source_info.audio {
            let source_rx = if source_info.format == ContainerFormat::Webm {
                AudioSourceThread::start(error_flag.clone(), source_info, stats.clone())
                    .or_fail()?
            } else {
                AudioSourceThread::start(error_flag.clone(), source_info, stats.clone())
                    .or_fail()?
            };
            audio_source_rxs.push(source_rx);
        }
        if video_source_ids.contains(source_id) && source_info.video {
            let options = VideoDecoderOptions {
                openh264_lib: openh264_lib.clone(),
            };
            let decoder = VideoDecoder::new(options);
            let source_rx = if source_info.format == ContainerFormat::Webm {
                VideoSourceThread::start(error_flag.clone(), source_info, decoder, stats.clone())
                    .or_fail()?
            } else {
                VideoSourceThread::start(error_flag.clone(), source_info, decoder, stats.clone())
                    .or_fail()?
            };
            video_source_rxs.push(source_rx);
        }
    }
    Ok((audio_source_rxs, video_source_rxs))
}
