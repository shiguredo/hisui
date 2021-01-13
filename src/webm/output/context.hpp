#pragma once

#include <cstddef>
#include <cstdint>
#include <cstdio>

namespace mkvmuxer {

class MkvWriter;
class Segment;

}  // namespace mkvmuxer

namespace hisui::webm::output {

class Context {
 public:
  explicit Context(std::FILE* file);
  ~Context();

  void setAudioTrack(const std::uint64_t codec_delay,
                     const std::uint8_t* private_data,
                     const std::size_t private_data_size);
  void setVideoTrack(const std::uint32_t width,
                     const std::uint32_t height,
                     const std::uint32_t fourcc);

  void addVideoFrame(const uint8_t* content,
                     const std::uint64_t length,
                     const std::uint64_t pts_ns,
                     bool is_key_frame);
  void addAudioFrame(const uint8_t* content,
                     const std::uint64_t length,
                     const std::uint64_t pts_ns);

 private:
  mkvmuxer::MkvWriter* m_writer;
  mkvmuxer::Segment* m_segment;
  const std::uint64_t m_video_track_number = 1;
  const std::uint64_t m_audio_track_number = 2;
};

}  // namespace hisui::webm::output
