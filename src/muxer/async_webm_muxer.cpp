#include "muxer/async_webm_muxer.hpp"

#include <array>
#include <cstdint>
#include <filesystem>
#include <iterator>
#include <stdexcept>
#include <string>

#include "audio/opus.hpp"
#include "constants.hpp"
#include "frame.hpp"
#include "muxer/audio_producer.hpp"
#include "muxer/opus_audio_producer.hpp"
#include "muxer/video_producer.hpp"
#include "muxer/vpx_video_producer.hpp"
#include "webm/output/context.hpp"

namespace hisui::muxer {

AsyncWebMMuxer::AsyncWebMMuxer(const hisui::Config& t_config,
                               const hisui::Metadata& t_metadata)
    : m_config(t_config), m_metadata(t_metadata) {}

void AsyncWebMMuxer::setUp() {
  if (m_config.out_filename == "") {
    std::filesystem::path metadata_path(m_config.in_metadata_filename);
    m_config.out_filename = metadata_path.replace_extension(".webm");
  }

  m_file = std::fopen(m_config.out_filename.c_str(), "wb");
  if (!m_file) {
    throw std::runtime_error("Unable to open: " + m_config.out_filename);
  }
  m_context = new hisui::webm::output::Context(m_file);

  if (m_config.out_video_bit_rate == 0) {
    m_config.out_video_bit_rate =
        static_cast<std::uint32_t>(std::size(m_metadata.getArchives())) *
        hisui::Constants::VIDEO_VPX_BIT_RATE_PER_FILE;
  }

  m_video_producer = new VPXVideoProducer(m_config, m_metadata);
  m_context->setVideoTrack(m_video_producer->getWidth(),
                           m_video_producer->getHeight(),
                           m_video_producer->getFourcc());
  OpusAudioProducer* audio_producer =
      new OpusAudioProducer(m_config, m_metadata);
  const auto skip = audio_producer->getSkip();
  m_audio_producer = audio_producer;

  const auto private_data =
      hisui::audio::create_opus_private_data({.skip = skip});

  m_context->setAudioTrack(static_cast<std::uint64_t>(skip) *
                               hisui::Constants::NANO_SECOND /
                               hisui::Constants::PCM_SAMPLE_RATE,
                           private_data.data(), std::size(private_data));
}

AsyncWebMMuxer::~AsyncWebMMuxer() {
  delete m_context;
  std::fclose(m_file);

  delete m_video_producer;
  delete m_audio_producer;
}

void AsyncWebMMuxer::appendAudio(hisui::Frame frame) {
  m_context->addAudioFrame(frame.data, frame.data_size, frame.timestamp);
  delete[] frame.data;
  m_audio_producer->bufferPop();
}

void AsyncWebMMuxer::appendVideo(hisui::Frame frame) {
  m_context->addVideoFrame(frame.data, frame.data_size, frame.timestamp,
                           frame.is_key);
  delete[] frame.data;
  m_video_producer->bufferPop();
}

void AsyncWebMMuxer::run() {
  mux();
}

void AsyncWebMMuxer::cleanUp() {}

void AsyncWebMMuxer::muxFinalize() {}

}  // namespace hisui::muxer
