#include "webm/input/audio_context.hpp"

#include <bits/exception.h>
#include <mkvparser/mkvparser.h>
#include <spdlog/fmt/fmt.h>
#include <spdlog/spdlog.h>

#include <cstddef>
#include <cstdio>
#include <cstring>
#include <stdexcept>

#include "webm/input/context.hpp"

namespace hisui::webm::input {

AudioContext::AudioContext(const std::string& t_file_path)
    : Context(t_file_path) {}

AudioContext::~AudioContext() {
  reset();
}

void AudioContext::reset() {
  Context::reset();
  m_channels = 0;
  m_bit_depth = 0;
  m_sampling_rate = 0.0;
  m_codec = AudioCodec::None;
}

bool AudioContext::init() {
  m_file = std::fopen(m_file_path.c_str(), "rb");
  if (m_file == nullptr) {
    throw std::runtime_error("Unable to open: " + m_file_path);
  }

  initReaderAndSegment(m_file);

  const mkvparser::Tracks* const tracks = m_segment->GetTracks();
  const mkvparser::AudioTrack* audio_track = nullptr;
  for (std::uint64_t i = 0, m = tracks->GetTracksCount(); i < m; ++i) {
    const mkvparser::Track* const track = tracks->GetTrackByIndex(i);
    if (track != nullptr && track->GetType() == mkvparser::Track::kAudio) {
      audio_track = static_cast<const mkvparser::AudioTrack*>(track);
      m_track_index = static_cast<int>(track->GetNumber());
      break;
    }
  }

  if (audio_track == nullptr || audio_track->GetCodecId() == nullptr) {
    spdlog::info("audio track not found");
    return false;
  }

  if (!std::strncmp(audio_track->GetCodecId(), "A_OPUS", 6)) {
    m_codec = AudioCodec::Opus;
    // WebM 側に Channels が入っていない場合があるので, Opus のヘッダーから
    // Channels を取得する
    std::size_t private_size;
    const unsigned char* const private_data =
        audio_track->GetCodecPrivate(private_size);
    if (private_size >= 10 &&
        !std::strncmp(reinterpret_cast<const char*>(private_data), "OpusHead",
                      8)) {
      m_channels = private_data[9];
    } else {
      m_channels = static_cast<int>(audio_track->GetChannels());
    }
  } else {
    spdlog::info("unsuppoted codec: codec_id={}", audio_track->GetCodecId());
    return false;
  }

  m_bit_depth = static_cast<std::uint64_t>(audio_track->GetBitDepth());
  m_sampling_rate = audio_track->GetSamplingRate();

  m_cluster = m_segment->GetFirst();

  return true;
}

int AudioContext::getChannels() const {
  return m_channels;
}

std::uint64_t AudioContext::getBitDepth() const {
  return m_bit_depth;
}

double AudioContext::getSamplingRate() const {
  return m_sampling_rate;
}

AudioCodec AudioContext::getCodec() const {
  return m_codec;
}

}  // namespace hisui::webm::input
