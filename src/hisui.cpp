#include <bits/exception.h>
#include <spdlog/common.h>
#include <spdlog/fmt/fmt.h>
#include <spdlog/spdlog.h>

#include <stdexcept>
#include <string>

#include <CLI/App.hpp>
#include <CLI/Config.hpp>
#include <CLI/Formatter.hpp>

#include "config.hpp"
#include "metadata.hpp"
#include "muxer/async_webm_muxer.hpp"
#include "muxer/faststart_mp4_muxer.hpp"
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

  if (!config.openh264.empty()) {
    try {
      hisui::video::OpenH264Handler::open(config.openh264);
    } catch (const std::exception& e) {
      spdlog::warn("failed to open openh264 library: {}", e.what());
    }
  }

  hisui::MetadataSet metadata_set(
      hisui::parse_metadata(config.in_metadata_filename));

  if (config.screen_capture_metadata_filename != "") {
    metadata_set.setPrefered(
        hisui::parse_metadata(config.screen_capture_metadata_filename));
  }

  hisui::muxer::Muxer* muxer = nullptr;
  if (config.out_container == hisui::config::OutContainer::WebM) {
    muxer = new hisui::muxer::AsyncWebMMuxer(config, metadata_set);
  } else if (config.out_container == hisui::config::OutContainer::MP4) {
    if (config.mp4_muxer == hisui::config::MP4Muxer::Simple) {
      muxer = new hisui::muxer::SimpleMP4Muxer(config, metadata_set);
    } else if (config.mp4_muxer == hisui::config::MP4Muxer::Faststart) {
      muxer = new hisui::muxer::FaststartMP4Muxer(config, metadata_set);
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
