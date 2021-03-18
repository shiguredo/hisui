#include "muxer/muxer.hpp"

#include <cxxabi.h>
#include <spdlog/spdlog.h>

#include <chrono>
#include <future>
#include <optional>
#include <system_error>
#include <thread>

#include <boost/exception/exception.hpp>
#include <boost/rational.hpp>

#include "frame.hpp"
#include "muxer/audio_producer.hpp"
#include "muxer/video_producer.hpp"

namespace hisui::muxer {

void Muxer::mux() {
  auto video_future =
      std::async(std::launch::async, &VideoProducer::produce, m_video_producer);

  auto audio_future =
      std::async(std::launch::async, &AudioProducer::produce, m_audio_producer);

  std::this_thread::sleep_for(std::chrono::milliseconds(100));

  bool video_finished = false;

  while (!m_audio_producer->isFinished()) {
    const auto audio_front = m_audio_producer->bufferFront();
    if (!audio_front.has_value()) {
      spdlog::debug("audio queue is empty");
      std::this_thread::sleep_for(std::chrono::milliseconds(100));
      continue;
    }
    const auto audio_timestamp = audio_front.value().timestamp;

    if (video_finished) {
      appendAudio(audio_front.value());
      continue;
    }

    if (m_video_producer->isFinished()) {
      video_finished = true;
      video_future.get();
      spdlog::debug("video was processed");
      appendAudio(audio_front.value());
      continue;
    }

    const auto video_front = m_video_producer->bufferFront();
    if (!video_front.has_value()) {
      spdlog::debug("video queue is empty (1)");
      std::this_thread::sleep_for(std::chrono::milliseconds(1000));
      continue;
    }
    const auto video_timestamp =
        video_front.value().timestamp * m_timescale_ratio;

    if (video_timestamp <= audio_timestamp) {
      appendVideo(video_front.value());
      continue;
    }
    appendAudio(audio_front.value());
  }

  audio_future.get();
  spdlog::debug("audio was processed");

  if (video_finished) {
    muxFinalize();
    return;
  }

  spdlog::debug("video is processing");
  while (!m_video_producer->isFinished()) {
    const auto video_front = m_video_producer->bufferFront();
    if (!video_front.has_value()) {
      spdlog::debug("video queue is empty (2)");
      std::this_thread::sleep_for(std::chrono::milliseconds(1000));
      continue;
    }

    appendVideo(video_front.value());
  }

  video_future.get();
  spdlog::debug("video was processed");

  muxFinalize();
}

}  // namespace hisui::muxer
