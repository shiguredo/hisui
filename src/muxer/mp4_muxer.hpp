#pragma once

#include <cstdint>
#include <fstream>
#include <vector>

#include "frame.hpp"
#include "muxer/muxer.hpp"

namespace hisui {

class Config;
class MetadataSet;

}  // namespace hisui

namespace shiguredo::mp4::track {

class SounTrack;
class VideTrack;

}  // namespace shiguredo::mp4::track

namespace shiguredo::mp4::writer {

class Writer;

}

namespace hisui::muxer {

class MP4Muxer : public Muxer {
 public:
  ~MP4Muxer();

 protected:
  std::ofstream m_ofs;
  shiguredo::mp4::writer::Writer* m_writer;
  shiguredo::mp4::track::VideTrack* m_vide_track = nullptr;
  shiguredo::mp4::track::SounTrack* m_soun_track;
  std::uint64_t m_chunk_interval;

  std::uint64_t m_chunk_start = 0;
  std::vector<hisui::Frame> m_audio_buffer;
  std::vector<hisui::Frame> m_video_buffer;

  void muxFinalize() override;
  void appendAudio(hisui::Frame) override;
  void appendVideo(hisui::Frame) override;

  void writeTrackData();
  void initialize(const hisui::Config&,
                  const hisui::MetadataSet&,
                  shiguredo::mp4::writer::Writer*,
                  const float);
};

}  // namespace hisui::muxer
