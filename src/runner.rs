use orfail::OrFail;
use shiguredo_openh264::Openh264Library;

use crate::{
    command_line_args::Args,
    composer::{ComposeResult, Composer},
    layout::Layout,
    metadata::RecordingMetadata,
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

        // 変換後のファイルのパスを決定
        let out_file_path = if let Some(path) = self.args.out_file.clone() {
            path
        } else if !layout.has_video() && layout.has_audio() {
            layout.base_path.join("output.mp4a")
        } else {
            layout.base_path.join("output.mp4")
        };

        // Composer を作成して設定
        let mut composer = Composer::new(layout);
        composer.out_video_codec = self.args.out_video_codec;
        composer.out_audio_codec = self.args.out_audio_codec;
        composer.openh264_lib = openh264_lib;
        composer.show_progress_bar = self.args.show_progress_bar;
        composer.max_cpu_cores = self.args.cpu_cores;
        composer.out_stats_file = self.args.out_stats_file.clone();
        composer.libvpx_cq_level = self.args.libvpx_cq_level;
        composer.libvpx_min_q = self.args.libvpx_min_q;
        composer.libvpx_max_q = self.args.libvpx_max_q;
        composer.out_aac_bit_rate = self.args.out_aac_bit_rate;
        composer.out_opus_bit_rate = self.args.out_opus_bit_rate;

        // 合成を実行
        let ComposeResult { stats: _, success } = composer.compose(&out_file_path).or_fail()?;

        if !success {
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
}
