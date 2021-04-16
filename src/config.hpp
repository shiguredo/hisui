#pragma once

#include <libyuv/scale.h>
#include <spdlog/common.h>

#include <cstddef>
#include <cstdint>
#include <string>

#include <boost/cstdint.hpp>
#include <boost/rational.hpp>

#include "constants.hpp"

namespace CLI {

class App;

}

namespace hisui {

namespace config {

enum struct AudioMixer {
  Simple,
  Vttoth,
};

enum OutVideoCodec {
  VP8 = hisui::Constants::VP8_FOURCC,
  VP9 = hisui::Constants::VP9_FOURCC,
};

enum struct VideoComposer {
  Grid,
  ParallelGrid,
};

enum struct VideoScaler {
  Simple,
  PreserveAspectRatio,
};

enum struct OutContainer {
  WebM,
  MP4,
};

enum struct MP4Muxer {
  Simple,
  Faststart,
};

enum struct OutAudioCodec {
  Opus,
  FDK_AAC,
};

}  // namespace config

class Config {
 public:
  std::string in_metadata_filename;
  std::string screen_capture_metadata_filename = "";
  config::OutVideoCodec out_video_codec = config::OutVideoCodec::VP9;
  config::OutContainer out_container = config::OutContainer::WebM;
  std::uint32_t out_video_bit_rate = 0;

  boost::rational<std::uint64_t> out_video_frame_rate =
      boost::rational<std::uint64_t>(25, 1);
  std::uint32_t libvpx_cq_level = 30;
  std::uint32_t libvpx_min_q = 10;
  std::uint32_t libvpx_max_q = 50;
  std::uint32_t out_opus_bit_rate = Constants::OPUS_DEFAULT_BIT_RATE;
  std::uint32_t out_aac_bit_rate = Constants::FDK_AAC_DEFAULT_BIT_RATE;

  std::string out_filename = "";
  std::string directory_for_faststart_intermediate_file = "";

  std::size_t max_columns = 3;

  bool verbose = false;
  bool audio_only = false;

  // 以降は SPEC.rst にないオプション
  bool show_progress_bar = true;

#ifdef NDEBUG
  spdlog::level::level_enum log_level = spdlog::level::info;
#else
  spdlog::level::level_enum log_level = spdlog::level::debug;
#endif
  std::uint32_t scaling_width = 320;
  std::uint32_t scaling_height = 240;

  std::uint32_t screen_capture_width = 960;
  std::uint32_t screen_capture_height = 640;
  std::uint32_t screen_capture_bit_rate = 1000;

  std::uint32_t libvpx_threads = 0;
  std::int32_t libvpx_cpu_used = 8;
  std::uint32_t libvp9_frame_parallel = 1;
  std::uint32_t libvp9_tile_columns = 0;
  std::uint32_t libvp9_row_mt = 0;

  libyuv::FilterMode libyuv_filter_mode = libyuv::kFilterBox;

  config::VideoComposer video_composer = config::VideoComposer::Grid;
  config::VideoScaler video_scaler = config::VideoScaler::PreserveAspectRatio;
  std::string openh264 = "";

  config::AudioMixer audio_mixer = config::AudioMixer::Simple;
  bool mix_screen_capture_audio = false;

  config::MP4Muxer mp4_muxer = config::MP4Muxer::Faststart;
  config::OutAudioCodec out_audio_codec = config::OutAudioCodec::Opus;
};

void set_cli_options(CLI::App* app, Config* config);

}  // namespace hisui
