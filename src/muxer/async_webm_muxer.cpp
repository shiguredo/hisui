#include "muxer/async_webm_muxer.hpp"

#include <cxxabi.h>
#include <spdlog/spdlog.h>

#include <chrono>
#include <filesystem>
#include <future>
#include <iterator>
#include <optional>
#include <stdexcept>
#include <string>
#include <system_error>
#include <thread>
#include <tuple>
#include <vector>

#include "constants.hpp"
#include "muxer/audio_producer.hpp"
#include "muxer/video_producer.hpp"
#include "webm/output/context.hpp"

namespace hisui::muxer {

AsyncWebMMuxer::AsyncWebMMuxer(const hisui::Config& t_config,
                               const hisui::Metadata& t_metadata)
    : m_config(t_config), m_metadata(t_metadata) {
  if (m_config.out_webm_filename == "") {
    std::filesystem::path metadata_path(m_config.in_metadata_filename);
    auto webm_path = metadata_path.replace_extension(".webm");
    m_config.out_webm_filename = webm_path;
  }

  m_file = std::fopen(m_config.out_webm_filename.c_str(), "wb");
  if (!m_file) {
    throw std::runtime_error("Unable to open: " + m_config.out_webm_filename);
  }
  m_context = new hisui::webm::output::Context(m_file);

  if (m_config.out_video_bit_rate == 0) {
    m_config.out_video_bit_rate =
        static_cast<std::uint32_t>(std::size(m_metadata.getArchives())) *
        hisui::Constants::VIDEO_VPX_BIT_RATE_PER_FILE;
  }

  m_video_producer = new VideoProducer(m_config, m_metadata, m_context);
  m_audio_producer = new AudioProducer(m_config, m_metadata, m_context);
}

AsyncWebMMuxer::~AsyncWebMMuxer() {
  delete m_context;
  std::fclose(m_file);

  delete m_video_producer;
  delete m_audio_producer;
}

void AsyncWebMMuxer::addAndConsumeAudio(std::uint8_t* data,
                                        const std::size_t data_length,
                                        const std::uint64_t timestamp) {
  m_context->addAudioFrame(data, data_length, timestamp);
  delete[] data;
  m_audio_producer->bufferPop();
}

void AsyncWebMMuxer::addAndConsumeVideo(std::uint8_t* data,
                                        const std::size_t data_length,
                                        const std::uint64_t timestamp,
                                        const bool is_keyframe) {
  m_context->addVideoFrame(data, data_length, timestamp, is_keyframe);
  delete[] data;
  m_video_producer->bufferPop();
}

void AsyncWebMMuxer::run() {
  auto video_future =
      std::async(std::launch::async, &VideoProducer::produce, m_video_producer);

  auto audio_future =
      std::async(std::launch::async, &AudioProducer::produce, m_audio_producer);

  std::this_thread::sleep_for(std::chrono::milliseconds(100));

  bool video_finished = false;

  while (!m_audio_producer->isFinished()) {
    auto audio_front = m_audio_producer->bufferFront();
    if (!audio_front.has_value()) {
      spdlog::debug("audio queue is empty");
      std::this_thread::sleep_for(std::chrono::milliseconds(100));
      continue;
    }
    auto audio_timestamp = get<0>(audio_front.value());

    if (video_finished) {
      addAndConsumeAudio(get<1>(audio_front.value()),
                         get<2>(audio_front.value()), audio_timestamp);
      continue;
    }

    if (m_video_producer->isFinished()) {
      video_finished = true;
      addAndConsumeAudio(get<1>(audio_front.value()),
                         get<2>(audio_front.value()), audio_timestamp);
      continue;
    }

    auto video_front = m_video_producer->bufferFront();
    if (!video_front.has_value()) {
      spdlog::debug("video queue is empty (1)");
      std::this_thread::sleep_for(std::chrono::milliseconds(1000));
      continue;
    }
    auto video_timestamp = get<0>(video_front.value());

    if (video_timestamp <= audio_timestamp) {
      addAndConsumeVideo(get<1>(video_front.value()),
                         get<2>(video_front.value()), video_timestamp,
                         get<3>(video_front.value()));
      continue;
    }
    addAndConsumeAudio(get<1>(audio_front.value()), get<2>(audio_front.value()),
                       audio_timestamp);
  }

  spdlog::debug("audio was processed");

  if (video_finished) {
    spdlog::debug("video was processed");
    return;
  }

  spdlog::debug("video is processing");
  while (!m_video_producer->isFinished()) {
    auto video_front = m_video_producer->bufferFront();
    if (!video_front.has_value()) {
      spdlog::debug("video queue is empty (2)");
      std::this_thread::sleep_for(std::chrono::milliseconds(1000));
      continue;
    }

    addAndConsumeVideo(get<1>(video_front.value()), get<2>(video_front.value()),
                       get<0>(video_front.value()),
                       get<3>(video_front.value()));
  }

  spdlog::debug("video was processed");
}  // namespace hisui::muxer

}  // namespace hisui::muxer
