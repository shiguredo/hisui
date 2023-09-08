#include "webm/output/context.hpp"

#include <stdexcept>
#include <string>

#include "fmt/core.h"
#include "mkvmuxer/mkvmuxer.h"
#include "mkvmuxer/mkvwriter.h"

#include "constants.hpp"

namespace hisui::webm::output {

Context::Context(const std::string& t_file_path) : m_file_path(t_file_path) {}

void Context::init() {
  m_file = std::fopen(m_file_path.c_str(), "wb");
  if (m_file == nullptr) {
    throw std::runtime_error("Unable to open: " + m_file_path);
  }

  m_writer = new mkvmuxer::MkvWriter(m_file);
  m_segment = new mkvmuxer::Segment();
  m_segment->Init(m_writer);
  m_segment->set_mode(mkvmuxer::Segment::kFile);
  m_segment->OutputCues(true);
  mkvmuxer::SegmentInfo* const info = m_segment->GetSegmentInfo();
  info->set_timecode_scale(1000000);
  info->set_writing_app(hisui::Constants::HISUI_APPLICATION_NAME.c_str());
}

Context::~Context() {
  if (m_segment) {
    m_segment->Finalize();
    delete m_segment;
  }
  if (m_writer) {
    delete m_writer;
  }
  if (m_file) {
    std::fclose(m_file);
  }
}

void Context::setAudioTrack(const std::uint64_t codec_delay,
                            const std::uint8_t* private_data,
                            const std::size_t private_data_size) {
  std::uint64_t audio_track_id =
      m_segment->AddAudioTrack(hisui::Constants::PCM_SAMPLE_RATE, 2,
                               static_cast<int>(m_audio_track_number));
  mkvmuxer::AudioTrack* const audio_track = static_cast<mkvmuxer::AudioTrack*>(
      m_segment->GetTrackByNumber(audio_track_id));

  if (audio_track == nullptr) {
    throw std::runtime_error("m_segment->GetTrackByNumber() failed");
  }

  audio_track->set_codec_id("A_OPUS");
  audio_track->set_seek_pre_roll(80000000);
  audio_track->set_codec_delay(codec_delay);
  audio_track->SetCodecPrivate(private_data, private_data_size);
}

void Context::setVideoTrack(const std::uint32_t width,
                            const std::uint32_t height,
                            const std::uint32_t fourcc,
                            const std::uint8_t* private_data,
                            const std::size_t private_data_size) {
  const std::uint64_t video_track_id = m_segment->AddVideoTrack(
      static_cast<int>(width), static_cast<int>(height),
      static_cast<int>(m_video_track_number));
  mkvmuxer::VideoTrack* const video_track = static_cast<mkvmuxer::VideoTrack*>(
      m_segment->GetTrackByNumber(video_track_id));

  if (video_track == nullptr) {
    throw std::runtime_error("m_segment->GetTrackByNumber() failed");
  }

  video_track->SetStereoMode(0);
  const char* codec_id;
  switch (fourcc) {
    case hisui::Constants::VP8_FOURCC:
      codec_id = "V_VP8";
      break;
    case hisui::Constants::VP9_FOURCC:
      codec_id = "V_VP9";
      break;
    case hisui::Constants::H264_FOURCC:
      codec_id = "V_MPEG4/ISO/AVC";
      break;
    case hisui::Constants::AV1_FOURCC:
      codec_id = "V_AV1";
      break;
    default:
      throw std::runtime_error(fmt::format("unknown fourcc: {:x}", fourcc));
  }
  video_track->set_codec_id(codec_id);

  if (private_data_size > 0) {
    video_track->SetCodecPrivate(private_data, private_data_size);
  }
}

void Context::addVideoFrame(const std::uint8_t* content,
                            const std::uint64_t length,
                            const std::uint64_t pts_ns,
                            bool is_key_frame) {
  if (!m_segment->AddFrame(content, length, m_video_track_number, pts_ns,
                           is_key_frame)) {
    throw std::runtime_error("writeVideoFrame(): AddFrame() failed");
  }
}

void Context::addAudioFrame(const std::uint8_t* content,
                            const std::uint64_t length,
                            const std::uint64_t pts_ns) {
  if (!m_segment->AddFrame(content, length, m_audio_track_number, pts_ns,
                           true)) {
    throw std::runtime_error("writeAudioFrame(): AddFrame() failed");
  }
}

}  // namespace hisui::webm::output
