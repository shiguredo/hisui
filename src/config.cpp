#include "config.hpp"

#include <libyuv/scale.h>
#include <spdlog/spdlog.h>

#include <functional>
#include <memory>
#include <stdexcept>
#include <type_traits>
#include <utility>
#include <vector>

#include <CLI/App.hpp>
#include <CLI/Config.hpp>
#include <CLI/Formatter.hpp>
#include <boost/rational.hpp>

#ifdef NDEBUG
#define OPTIONS_FOR_TUNING ""
#define OPTIONS_FOR_DEVELOPING ""
#define OPTIONS_FOR_BACKWARD_COMPATIBILITY ""
#else
#define OPTIONS_FOR_TUNING "Options for tuning"
#define OPTIONS_FOR_DEVELOPING "Options for developing"
#define OPTIONS_FOR_BACKWARD_COMPATIBILITY "Options for backwards compatibility"
#endif

namespace {

class MyFormatter : public CLI::Formatter {
 public:
  std::string make_option_opts(const CLI::Option*) const override { return ""; }
};

struct NonNegativeMultipleOf4Validator : public CLI::Validator {
  NonNegativeMultipleOf4Validator() : Validator("NONNEGATIVEMULTIPLEOF4") {
    func_ = [](std::string& number_str) {
      std::uint64_t number;
      if (!CLI::detail::lexical_cast(number_str, number)) {
        return std::string("Failed parsing number: (") + number_str + ')';
      }
      if (number % 4) {
        return std::string("Number not multiple of 4: (") + number_str + ')';
      }
      return std::string();
    };
  }
};

const NonNegativeMultipleOf4Validator NonNegativeMultipleOf4;

}  // namespace

namespace hisui {

void set_cli_options(CLI::App* app, Config* config) {
  app->formatter(std::make_shared<MyFormatter>());
  app->add_option("-f,--in-metadata-file", config->in_metadata_filename,
                  "Metadata filename (REQUIRED)")
      ->check(CLI::ExistingFile)
      ->required();

  std::vector<std::pair<std::string, config::OutContainer>> out_container_assoc{
      {"WebM", config::OutContainer::WebM},
      {"MP4", config::OutContainer::MP4},
  };
  app->add_option("--out-container", config->out_container,
                  "Output container type (WebM/MP4) default: WebM")
      ->transform(
          CLI::CheckedTransformer(out_container_assoc, CLI::ignore_case));

  std::vector<std::pair<std::string, std::uint32_t>> out_video_codec_assoc{
      {"VP8", config::OutVideoCodec::VP8},
      {"VP9", config::OutVideoCodec::VP9},
  };
  app->add_option("--out-video-codec", config->out_video_codec,
                  "Video codec (VP8/VP9) default: VP9")
      ->transform(CLI::CheckedTransformer(out_video_codec_assoc));

  std::vector<std::pair<std::string, config::OutAudioCodec>> out_audio_codec{
      {"Opus", config::OutAudioCodec::Opus},
#ifdef USE_FDK_AAC
      {"FDK-AAC", config::OutAudioCodec::FDK_AAC},
      {"AAC", config::OutAudioCodec::FDK_AAC},
#endif
  };
  app->add_option("--out-audio-codec", config->out_audio_codec,
                  "Audio codec (Opus/AAC) default: Opus (hisui supports "
                  "AAC only in WebM)")
#ifndef USE_FDK_AAC
      ->group("")
#endif
      ->transform(CLI::CheckedTransformer(out_audio_codec, CLI::ignore_case));

  app->add_option("--out-video-frame-rate", config->out_video_frame_rate,
                  "Video frame rate (INTEGER/RATIONAL) default: 25)");

  app->add_option("--out-webm-file", config->out_filename, "Output filename")
      ->group(OPTIONS_FOR_BACKWARD_COMPATIBILITY);
  app->add_option("--out-file", config->out_filename, "Output filename");

  app->add_option("--max-columns", config->max_columns,
                  "Max columns (POSITIVE INTEGER) default: 3")
      ->check(CLI::PositiveNumber);

  app->add_option(
         "--libvpx-cq-level", config->libvpx_cq_level,
         "libvpx Constrained Quality level (NON NAGATIVE INTEGER) default: 10")
      ->check(CLI::Range(0, 63));
  app->add_option(
         "--libvpx-min-q", config->libvpx_min_q,
         "libvpx minimum (best) quantizer (NON NEGATIVE INTEGER) default: 3")
      ->check(CLI::Range(0, 63));
  app->add_option(
         "--libvpx-max-q", config->libvpx_max_q,
         "libvpx maximum (worst) quantizer (NON NEGATIVE INTEGER) default: 40")
      ->check(CLI::Range(0, 63));

  app->add_option("--out-opus-bit-rate", config->out_opus_bit_rate,
                  "Opus bit rate (kbps, POSITIVE INTEGER). default: 65536")
      ->check(CLI::PositiveNumber);

  app->add_option("--out-aac-bit-rate", config->out_aac_bit_rate,
                  "AAC bit rate (kbps, POSITIVE INTEGER). default: 64000")
      ->check(CLI::PositiveNumber);

  std::vector<std::pair<std::string, config::MP4Muxer>> mp4_muxer_assoc{
      {"simple", config::MP4Muxer::Simple},
      {"faststart", config::MP4Muxer::Faststart},
  };
  app->add_option("--mp4-muxer", config->mp4_muxer,
                  "MP4 muxer (Faststart/Simple). default: Faststart")
      ->transform(CLI::CheckedTransformer(mp4_muxer_assoc, CLI::ignore_case));

  app->add_option("--dir-for-faststart",
                  config->directory_for_faststart_intermediate_file,
                  "Directory for intermediate files of faststart "
                  "muxer. default: metadata directory");

  app->add_option("--openh264", config->openh264,
                  "OpenH264 dynamic library path");

  app->add_flag("--verbose", config->verbose, "Verbose mode");
  app->add_flag("--audio-only", config->audio_only, "Audio only mode");
  app->add_option("--show-progress-bar", config->show_progress_bar,
                  "Toggle to show progress bar. default: true");

  app->add_option("--out-video-bit-rate", config->out_video_bit_rate,
                  "Video bit rate (kbps, POSITIVE INTEGER). default: 200 x "
                  "(number of inputs)")
      ->check(CLI::PositiveNumber)
      ->group(OPTIONS_FOR_TUNING);

  app->add_option(
         "--scaling-width", config->scaling_width,
         "Scaling width per grid (NON NAGATIVE multiple of 4. auto mode: "
         "0) default: 320")
      ->check(NonNegativeMultipleOf4)
      ->group(OPTIONS_FOR_TUNING);

  app->add_option("--scaling-height", config->scaling_height,
                  "Scaling height per grid (NON NAGATIVE multiple of 4, auto "
                  "mode: 0) default: 240")
      ->check(NonNegativeMultipleOf4)
      ->group(OPTIONS_FOR_TUNING);

  std::vector<std::pair<std::string, libyuv::FilterMode>>
      libyuv_filter_mode_assoc{
          {"none", libyuv::kFilterNone},
          {"linear", libyuv::kFilterLinear},
          {"bilinear", libyuv::kFilterBilinear},
          {"box", libyuv::kFilterBox},
      };
  app->add_option("--libyuv-filter-mode", config->libyuv_filter_mode,
                  "libyuv filter mode (none/linear/bilinear/box) default: box")
      ->transform(
          CLI::CheckedTransformer(libyuv_filter_mode_assoc, CLI::ignore_case))
      ->group(OPTIONS_FOR_TUNING);

  app->add_option("--libvpx-threads", config->libvpx_threads,
                  "libvpx max number of threads (NON NEGATIVE INTEGER) "
                  "default: 0 (use default)")
      ->check(CLI::NonNegativeNumber)
      ->group(OPTIONS_FOR_TUNING);

  app->add_option("--libvp9-frame-parallel", config->libvp9_frame_parallel,
                  "libvp9 frame parallel decodability features "
                  "default: 1")
      ->check(CLI::Range(0, 1))
      ->group(OPTIONS_FOR_TUNING);

  app->add_option("--libvpx-cpu-used", config->libvpx_cpu_used,
                  "libvpx cpu used (-16, 16) "
                  "default: 4")
      ->check(CLI::Range(-16, 16))
      ->group(OPTIONS_FOR_TUNING);

  app->add_option("--libvp9-tile-columns", config->libvp9_tile_columns,
                  "libvp9 number of tile columns to use, log2 "
                  "default: 0 (0, 6)")
      ->check(CLI::Range(0, 6))
      ->group(OPTIONS_FOR_TUNING);

  std::vector<std::pair<std::string, spdlog::level::level_enum>>
      log_level_assoc{
          {"trace", spdlog::level::trace},
          {"debug", spdlog::level::debug},
          {"info", spdlog::level::info},
          {"warn", spdlog::level::warn},
          {"error", spdlog::level::err},
          {"critical", spdlog::level::critical},
          {"off", spdlog::level::off},
      };
  app->add_option(
         "-l,--log-level", config->log_level,
         "Log level (trace/debug/info/warn/error/critical/off) default: info")
      ->transform(CLI::CheckedTransformer(log_level_assoc, CLI::ignore_case))
      ->group(OPTIONS_FOR_DEVELOPING);

  std::vector<std::pair<std::string, config::VideoComposer>>
      video_composer_assoc{
          {"grid", config::VideoComposer::Grid},
          {"parallel-grid", config::VideoComposer::ParallelGrid},
      };
  app->add_option("--video-composer", config->video_composer, "video composer")
      ->transform(
          CLI::CheckedTransformer(video_composer_assoc, CLI::ignore_case))
      ->group(OPTIONS_FOR_DEVELOPING);

  std::vector<std::pair<std::string, config::VideoScaler>> video_scaler_assoc{
      {"simple", config::VideoScaler::Simple},
      {"preserve-aspect-ratio", config::VideoScaler::PreserveAspectRatio},
  };
  app->add_option("--video-scaler", config->video_scaler, "video scaler")
      ->transform(CLI::CheckedTransformer(video_scaler_assoc, CLI::ignore_case))
      ->group(OPTIONS_FOR_DEVELOPING);

  std::vector<std::pair<std::string, config::AudioMixer>> audio_mixer_assoc{
      {"simple", config::AudioMixer::Simple},
      {"vttoth", config::AudioMixer::Vttoth}};
  app->add_option("--audio-mixer", config->audio_mixer, "audio mixer")
      ->transform(CLI::CheckedTransformer(audio_mixer_assoc, CLI::ignore_case))
      ->group(OPTIONS_FOR_DEVELOPING);
}

}  // namespace hisui
