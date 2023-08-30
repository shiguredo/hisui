#include <bits/exception.h>
#include <spdlog/common.h>
#include <spdlog/fmt/bundled/format.h>
#include <spdlog/fmt/fmt.h>
#include <spdlog/spdlog.h>

#include <filesystem>
#include <ostream>
#include <stdexcept>
#include <string>

#include <CLI/App.hpp>
#include <CLI/Config.hpp>
#include <CLI/Formatter.hpp>

#include "audio/lyra_handler.hpp"
#include "config.hpp"
#include "constants.hpp"
#include "datetime.hpp"
#include "layout/compose.hpp"
#include "metadata.hpp"
#include "muxer/async_webm_muxer.hpp"
#include "muxer/faststart_mp4_muxer.hpp"
#include "muxer/muxer.hpp"
#include "muxer/simple_mp4_muxer.hpp"
#include "report/reporter.hpp"
#include "version/version.hpp"
#include "video/codec_engine.hpp"
#include "video/decoder_factory.hpp"
#include "video/openh264_handler.hpp"

#ifdef USE_ONEVPL
#include "video/vpl_decoder.hpp"
#include "video/vpl_encoder.hpp"
#include "video/vpl_session.hpp"
#endif

int main(int argc, char** argv) {
  CLI::App app{"hisui"};
  hisui::Config config;

  ::setenv("SVT_LOG", "-2", 1);
  ::setenv("LIBVA_MESSAGING_LEVEL", "0", 1);

#ifdef USE_ONEVPL
  try {
    hisui::video::VPLSession::open();
  } catch (const std::exception& e) {
    spdlog::debug("failed to open VPL session: {}", e.what());
  }
#endif

  try {
    hisui::set_cli_options(&app, &config);

    CLI11_PARSE(app, argc, argv);

    if (config.version) {
      std::cout << "Recording Composition Tool Hisui "
                << hisui::version::get_hisui_version() << std::endl;
      return EXIT_SUCCESS;
    }

    if (config.verbose) {
      spdlog::set_level(spdlog::level::debug);
    } else {
      spdlog::set_level(config.log_level);
    }
    spdlog::debug("log level={}", static_cast<uint32_t>(config.log_level));

    if (std::empty(config.lyra_model_path)) {
      if (const auto hisui_lyra_model_coeffs_path =
              std::getenv("HISUI_LYRA_MODEL_COEFFS_PATH")) {
        config.lyra_model_path = hisui_lyra_model_coeffs_path;
      }
      spdlog::debug("config.lyra_model_path={}", config.lyra_model_path);
    }

    if (!std::empty(config.openh264)) {
      try {
        hisui::video::OpenH264Handler::open(config.openh264);
      } catch (const std::exception& e) {
        spdlog::warn("failed to open openh264 library: {}", e.what());
      }
    }

    if (!std::empty(config.lyra_model_path)) {
      try {
        hisui::audio::LyraHandler::setModelPath(config.lyra_model_path);
      } catch (const std::exception& e) {
        spdlog::warn("failed to set lyra model path: {}", e.what());
        return EXIT_FAILURE;
      }
    }

    if (config.enabledReport()) {
      hisui::report::Reporter::open();
    }
  } catch (const std::exception& e) {
    spdlog::error("adjusting configuration failed: {}", e.what());
    return EXIT_FAILURE;
  }

  if (!std::empty(config.layout)) {
    hisui::video::DecoderFactory::setup(config);
    auto ret = hisui::layout::compose(config);

    if (hisui::video::OpenH264Handler::hasInstance()) {
      hisui::video::OpenH264Handler::close();
    }

    if (hisui::audio::LyraHandler::hasInstance()) {
      hisui::audio::LyraHandler::close();
    }

#ifdef USE_ONEVPL
    if (hisui::video::VPLSession::hasInstance()) {
      hisui::video::VPLSession::close();
    }
#endif

    return ret;
  }

  config.validate();

  if (config.video_codec_engines) {
    hisui::video::showCodecEngines();
    return EXIT_SUCCESS;
  }

  if (std::empty(config.in_metadata_filename)) {
    spdlog::error("-f,--in-metadata-file is required");
    return EXIT_FAILURE;
  }

  hisui::video::DecoderFactory::setup(config);

  hisui::muxer::Muxer* muxer = nullptr;

  boost::json::string normal_recording_id;
  try {
    hisui::MetadataSet metadata_set(
        hisui::parse_metadata(config.in_metadata_filename));

    if (!config.screen_capture_metadata_filename.empty()) {
      metadata_set.setPrefered(
          hisui::parse_metadata(config.screen_capture_metadata_filename));
    } else if (!config.screen_capture_connection_id.empty()) {
      metadata_set.split(config.screen_capture_connection_id);
    }
    normal_recording_id = metadata_set.getNormal().getRecordingID();

    if (config.out_container == hisui::config::OutContainer::WebM) {
      muxer = new hisui::muxer::AsyncWebMMuxer(
          config,
          hisui::muxer::AsyncWebMMuxerParameters{
              .audio_archive_items = metadata_set.getArchiveItems(),
              .normal_archives = metadata_set.getNormal().getArchiveItems(),
              .preferred_archives =
                  metadata_set.hasPreferred()
                      ? metadata_set.getPreferred().getArchiveItems()
                      : std::vector<hisui::ArchiveItem>{},
              .duration = metadata_set.getMaxStopTimeOffset(),
          });
    } else if (config.out_container == hisui::config::OutContainer::MP4) {
      if (config.mp4_muxer == hisui::config::MP4Muxer::Simple) {
        muxer = new hisui::muxer::SimpleMP4Muxer(
            config,
            hisui::muxer::MP4MuxerParameters{
                .audio_archive_items = metadata_set.getArchiveItems(),
                .normal_archives = metadata_set.getNormal().getArchiveItems(),
                .preferred_archives =
                    metadata_set.hasPreferred()
                        ? metadata_set.getPreferred().getArchiveItems()
                        : std::vector<hisui::ArchiveItem>{},
                .duration = metadata_set.getMaxStopTimeOffset(),
            });
      } else if (config.mp4_muxer == hisui::config::MP4Muxer::Faststart) {
        muxer = new hisui::muxer::FaststartMP4Muxer(
            config,
            hisui::muxer::MP4MuxerParameters{
                .audio_archive_items = metadata_set.getArchiveItems(),
                .normal_archives = metadata_set.getNormal().getArchiveItems(),
                .preferred_archives =
                    metadata_set.hasPreferred()
                        ? metadata_set.getPreferred().getArchiveItems()
                        : std::vector<hisui::ArchiveItem>{},
                .duration = metadata_set.getMaxStopTimeOffset(),
            });
      } else {
        throw std::runtime_error("config.mp4_muxer is invalid");
      }
    } else {
      throw std::runtime_error("config.out_container is invalid");
    }
  } catch (const std::exception& e) {
    spdlog::error("setting up muxer failed: {}", e.what());
    return EXIT_FAILURE;
  }

  try {
    muxer->setUp();
    muxer->run();
  } catch (const std::exception& e) {
    spdlog::error("muxing failed: {}", e.what());
    try {
      muxer->cleanUp();
    } catch (const std::exception& e) {
      spdlog::error("cleaning up muxer failed: {}", e.what());
    }
    if (config.enabledFailureReport()) {
      try {
        std::ofstream os(std::filesystem::path(config.failure_report) /
                         fmt::format("{}_{}_failure.json",
                                     hisui::datetime::get_current_utc_string(),
                                     normal_recording_id));
        os << hisui::report::Reporter::getInstance().makeFailureReport(
            e.what());
        hisui::report::Reporter::close();
      } catch (const std::exception& e) {
        spdlog::error("reporting(failure) failed: {}", e.what());
        return EXIT_FAILURE;
      }
    }
    return EXIT_FAILURE;
  }
  delete muxer;

  if (hisui::video::OpenH264Handler::hasInstance()) {
    hisui::video::OpenH264Handler::close();
  }

  if (hisui::audio::LyraHandler::hasInstance()) {
    hisui::audio::LyraHandler::close();
  }

#ifdef USE_ONEVPL
  if (hisui::video::VPLSession::hasInstance()) {
    hisui::video::VPLSession::close();
  }
#endif

  if (config.enabledSuccessReport()) {
    try {
      std::ofstream os(std::filesystem::path(config.success_report) /
                       fmt::format("{}_{}_success.json",
                                   hisui::datetime::get_current_utc_string(),
                                   normal_recording_id));
      os << hisui::report::Reporter::getInstance().makeSuccessReport();
      hisui::report::Reporter::close();
    } catch (const std::exception& e) {
      spdlog::error("reporting(success) failed: {}", e.what());
      return EXIT_FAILURE;
    }
  }

  return EXIT_SUCCESS;
}
