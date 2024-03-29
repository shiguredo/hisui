#include "layout/compose.hpp"

#include <spdlog/spdlog.h>

#include <memory>

#include "config.hpp"
#include "constants.hpp"
#include "datetime.hpp"
#include "layout/av1_video_producer.hpp"
#include "layout/metadata.hpp"
#include "layout/openh264_video_producer.hpp"
#include "layout/vpx_video_producer.hpp"
#include "muxer/async_webm_muxer.hpp"
#include "muxer/faststart_mp4_muxer.hpp"
#include "muxer/muxer.hpp"
#include "muxer/no_video_producer.hpp"
#include "muxer/simple_mp4_muxer.hpp"
#include "report/reporter.hpp"
#include "video/openh264_handler.hpp"

#ifdef USE_ONEVPL
#include "layout/vpl_video_producer.hpp"
#include "video/vpl_encoder.hpp"
#include "video/vpl_session.hpp"
#endif

namespace hisui::layout {

int compose(const hisui::Config& t_config) {
  auto config = t_config;
  auto metadata = hisui::layout::parse_metadata(config);
  metadata.copyToConfig(&config);

  config.validate();

  std::shared_ptr<hisui::muxer::Muxer> muxer;
  std::shared_ptr<muxer::VideoProducer> video_producer;
  try {
    if (config.audio_only) {
      video_producer = std::make_shared<muxer::NoVideoProducer>();
    } else {
      if (config.out_video_codec == hisui::config::OutVideoCodec::H264) {
        if (config.h264_encoder == hisui::config::H264Encoder::OpenH264) {
          if (!hisui::video::OpenH264Handler::hasInstance()) {
            throw std::runtime_error("OpenH264 library is not loaded");
          }
          video_producer = std::make_shared<OpenH264VideoProducer>(
              config, OpenH264VideoProducerParameters{
                          .regions = metadata.getRegions(),
                          .resolution = metadata.getResolution(),
                          .duration = metadata.getMaxEndTime(),
                          .timescale = config.out_container ==
                                               hisui::config::OutContainer::WebM
                                           ? hisui::Constants::NANO_SECOND
                                           : 16000,  // TODO(haruyama): 整理する
                      });
        }
#ifdef USE_ONEVPL
        if (config.h264_encoder == hisui::config::H264Encoder::OneVPL) {
          auto fourcc = hisui::Constants::H264_FOURCC;
          if (!(hisui::video::VPLSession::hasInstance() &&
                hisui::video::VPLEncoder::isSupported(fourcc))) {
            throw std::runtime_error("oneVPL H.264 encoder is not supported");
          }
          video_producer = std::make_shared<VPLVideoProducer>(
              config,
              VPLVideoProducerParameters{
                  .regions = metadata.getRegions(),
                  .resolution = metadata.getResolution(),
                  .duration = metadata.getMaxEndTime(),
                  .timescale =
                      config.out_container == hisui::config::OutContainer::WebM
                          ? hisui::Constants::NANO_SECOND
                          : 16000,  // TODO(haruyama): 整理する
              },
              fourcc);
        }
#endif

        // Unspecified
        if (!video_producer) {
#ifdef USE_ONEVPL
          if (hisui::video::VPLSession::hasInstance() &&
              hisui::video::VPLEncoder::isSupported(
                  hisui::Constants::H264_FOURCC)) {
            auto fourcc = hisui::Constants::H264_FOURCC;
            spdlog::debug("use VPLVideoProducer");
            video_producer = std::make_shared<VPLVideoProducer>(
                config,
                VPLVideoProducerParameters{
                    .regions = metadata.getRegions(),
                    .resolution = metadata.getResolution(),
                    .duration = metadata.getMaxEndTime(),
                    .timescale = config.out_container ==
                                         hisui::config::OutContainer::WebM
                                     ? hisui::Constants::NANO_SECOND
                                     : 16000,  // TODO(haruyama): 整理する
                },
                fourcc);
          } else  // NOLINT
#endif
              if (hisui::video::OpenH264Handler::hasInstance()) {
            spdlog::debug("use OpenH264VideoProducer");
            video_producer = std::make_shared<OpenH264VideoProducer>(
                config,
                OpenH264VideoProducerParameters{
                    .regions = metadata.getRegions(),
                    .resolution = metadata.getResolution(),
                    .duration = metadata.getMaxEndTime(),
                    .timescale = config.out_container ==
                                         hisui::config::OutContainer::WebM
                                     ? hisui::Constants::NANO_SECOND
                                     : 16000,  // TODO(haruyama): 整理する
                });
          } else {
            throw std::runtime_error("H.264 dncoder is unavailable");
          }
        }
      } else if (config.out_video_codec == hisui::config::OutVideoCodec::AV1) {
        video_producer = std::make_shared<AV1VideoProducer>(
            config, AV1VideoProducerParameters{
                        .regions = metadata.getRegions(),
                        .resolution = metadata.getResolution(),
                        .duration = metadata.getMaxEndTime(),
                        .timescale = config.out_container ==
                                             hisui::config::OutContainer::WebM
                                         ? hisui::Constants::NANO_SECOND
                                         : 16000,  // TODO(haruyama): 整理する
                    });
      } else {
        video_producer = std::make_shared<VPXVideoProducer>(
            config, VPXVideoProducerParameters{
                        .regions = metadata.getRegions(),
                        .resolution = metadata.getResolution(),
                        .duration = metadata.getMaxEndTime(),
                        .timescale = config.out_container ==
                                             hisui::config::OutContainer::WebM
                                         ? hisui::Constants::NANO_SECOND
                                         : 16000,  // TODO(haruyama): 整理する
                    });
      }
    }
  } catch (const std::exception& e) {
    spdlog::error("setting up video_producer failed: {}", e.what());
    return EXIT_FAILURE;
  }

  auto audio_archive_items = metadata.getAudioArchiveItems();

  try {
    if (config.out_container == hisui::config::OutContainer::WebM) {
      muxer = std::make_shared<hisui::muxer::AsyncWebMMuxer>(
          config, hisui::muxer::AsyncWebMMuxerParametersForLayout{
                      .audio_archive_items = audio_archive_items,
                      .video_producer = video_producer,
                      .duration = metadata.getMaxEndTime()});

    } else if (config.out_container == hisui::config::OutContainer::MP4) {
      auto params = hisui::muxer::MP4MuxerParametersForLayout{
          .audio_archive_items = audio_archive_items,
          .video_producer = video_producer,
          .duration = metadata.getMaxEndTime()};
      if (config.mp4_muxer == hisui::config::MP4Muxer::Simple) {
        muxer = std::make_shared<hisui::muxer::SimpleMP4Muxer>(config, params);
      } else if (config.mp4_muxer == hisui::config::MP4Muxer::Faststart) {
        muxer =
            std::make_shared<hisui::muxer::FaststartMP4Muxer>(config, params);
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
        std::ofstream os(
            std::filesystem::path(config.failure_report) /
            fmt::format("{}_layout_failure.json",
                        hisui::datetime::get_current_utc_string()));
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

  if (config.enabledSuccessReport()) {
    try {
      std::ofstream os(std::filesystem::path(config.success_report) /
                       fmt::format("{}_layout_success.json",
                                   hisui::datetime::get_current_utc_string()));
      os << hisui::report::Reporter::getInstance().makeSuccessReport();
      hisui::report::Reporter::close();
    } catch (const std::exception& e) {
      spdlog::error("reporting(success) failed: {}", e.what());
      return EXIT_FAILURE;
    }
  }

  return EXIT_SUCCESS;
}

}  // namespace hisui::layout
