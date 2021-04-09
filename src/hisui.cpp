#include <bits/exception.h>
#include <spdlog/fmt/fmt.h>
#include <spdlog/spdlog.h>

#include <fstream>
#include <initializer_list>
#include <iterator>
#include <map>
#include <stdexcept>
#include <string>
#include <vector>

#include <CLI/App.hpp>
#include <CLI/Config.hpp>
#include <CLI/Formatter.hpp>
#include <boost/json.hpp>

#include "config.hpp"
#include "metadata.hpp"
#include "muxer/async_webm_muxer.hpp"
#include "muxer/faststart_mp4_muxer.hpp"
#include "muxer/multi_channel_async_webm_muxer.hpp"
#include "muxer/muxer.hpp"
#include "muxer/simple_mp4_muxer.hpp"
#include "video/openh264_handler.hpp"

int main(int argc, char** argv) {
  CLI::App app{"hisui"};
  hisui::Config config;

  hisui::set_cli_options(&app, &config);

  CLI11_PARSE(app, argc, argv);

  if (config.out_container == hisui::config::OutContainer::WebM &&
      config.out_audio_codec == hisui::config::OutAudioCodec::FDK_AAC) {
    spdlog::error("hisui does not support AAC output in WebM");
    return 1;
  }

  if (config.verbose) {
    spdlog::set_level(spdlog::level::debug);
  } else {
    spdlog::set_level(config.log_level);
  }
  spdlog::debug("log level={}", config.log_level);

  const hisui::Metadata metadata =
      hisui::parse_metadata(config.in_metadata_filename);

  if (!config.openh264.empty()) {
    try {
      hisui::video::OpenH264Handler::open(config.openh264);
    } catch (const std::exception& e) {
      spdlog::warn("failed to open openh264 library: {}", e.what());
    }
  }

  hisui::muxer::Muxer* muxer = nullptr;
  if (config.out_container == hisui::config::OutContainer::WebM) {
    if (config.in_multi_channel_metadata_filename == "") {
      muxer = new hisui::muxer::AsyncWebMMuxer(config, metadata);
    } else {
      const hisui::Metadata alternative_metadata =
          hisui::parse_metadata(config.in_multi_channel_metadata_filename);
      muxer = new hisui::muxer::MultiChannelAsyncWebMMuxer(
          config, metadata, alternative_metadata);
    }
  } else if (config.out_container == hisui::config::OutContainer::MP4) {
    if (config.mp4_muxer == hisui::config::MP4Muxer::Simple) {
      muxer = new hisui::muxer::SimpleMP4Muxer(config, metadata);
    } else if (config.mp4_muxer == hisui::config::MP4Muxer::Faststart) {
      muxer = new hisui::muxer::FaststartMP4Muxer(config, metadata);
    } else {
      throw std::runtime_error("config.mp4_muxer is invalid");
    }
  } else {
    throw std::runtime_error("config.out_container is invalid");
  }
  try {
    muxer->setUp();
    muxer->run();
  } catch (const std::exception& e) {
    spdlog::error("muxing failed: {}", e.what());
    muxer->cleanUp();
    return 1;
  }
  delete muxer;

  if (!config.openh264.empty()) {
    hisui::video::OpenH264Handler::close();
  }

  return 0;
}
