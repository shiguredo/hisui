#include "webm/input/video_context.hpp"

#include <bits/exception.h>
#include <mkvparser/mkvparser.h>
#include <spdlog/fmt/fmt.h>
#include <spdlog/spdlog.h>

#include <cstdio>
#include <cstring>
#include <stdexcept>
#include <string>

#include "constants.hpp"
#include "webm/input/context.hpp"

namespace hisui::webm::input {

VideoContext::VideoContext(const std::string& t_file_path)
    : Context(t_file_path) {}

VideoContext::~VideoContext() {
  reset();
}

void VideoContext::reset() {
  Context::reset();
  m_fourcc = 0;
}

bool VideoContext::init() {
  m_file = std::fopen(m_file_path.c_str(), "rb");
  if (m_file == nullptr) {
    throw std::runtime_error("Unable to open: " + m_file_path);
  }
  initReaderAndSegment(m_file);

  const mkvparser::Tracks* const tracks = m_segment->GetTracks();
  const mkvparser::VideoTrack* video_track = nullptr;
  for (std::uint64_t i = 0, m = tracks->GetTracksCount(); i < m; ++i) {
    const mkvparser::Track* const track = tracks->GetTrackByIndex(i);
    if (track != nullptr && track->GetType() == mkvparser::Track::kVideo) {
      video_track = static_cast<const mkvparser::VideoTrack*>(track);
      m_track_index = static_cast<int>(track->GetNumber());
      break;
    }
  }

  if (video_track == nullptr || video_track->GetCodecId() == nullptr) {
    spdlog::info("video track not found");
    return false;
  }

  if (video_track->GetWidth() == 0 || video_track->GetHeight() == 0) {
    spdlog::info("invalid video track");
    return false;
  }

  const auto codec_id = video_track->GetCodecId();

  if (!std::strncmp(codec_id, "V_VP8", 5)) {
    m_fourcc = hisui::Constants::VP8_FOURCC;
  } else if (!std::strncmp(codec_id, "V_VP9", 5)) {
    m_fourcc = hisui::Constants::VP9_FOURCC;
  } else if (!std::strncmp(codec_id, "V_MPEG4/ISO/AVC", 15)) {
    const auto codec_name_as_utf8 = video_track->GetCodecNameAsUTF8();
    if (codec_name_as_utf8 == nullptr) {
      spdlog::info("V_MPEG4/ISO/AVC: codec_name_as_utf8 is null");
      return false;
    }
    if (!std::strncmp(codec_name_as_utf8, "H.264", 5)) {
      m_fourcc = hisui::Constants::H264_FOURCC;
    } else {
      spdlog::info("V_MPEG4/ISO/AVC: unknown codec_name_as_utf8: {}",
                   codec_name_as_utf8);
      return false;
    }
  } else {
    if (video_track->GetCodecNameAsUTF8() == nullptr) {
      spdlog::info("unsuppoted codec: codec_id={}", video_track->GetCodecId());
    } else {
      spdlog::info("unsuppoted codec: codec_id={}, codec_name={}",
                   video_track->GetCodecId(),
                   video_track->GetCodecNameAsUTF8());
    }
    return false;
  }

  m_width = static_cast<std::uint32_t>(video_track->GetWidth());
  m_height = static_cast<std::uint32_t>(video_track->GetHeight());

  m_cluster = m_segment->GetFirst();

  return true;
}

std::uint32_t VideoContext::getFourcc() const {
  return m_fourcc;
}

std::uint32_t VideoContext::getWidth() const {
  return m_width;
}

std::uint32_t VideoContext::getHeight() const {
  return m_height;
}

}  // namespace hisui::webm::input
