#pragma once

#include <cstdint>
#include <fstream>
#include <memory>
#include <vector>

#include "archive_item.hpp"
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

struct MP4MuxerParameters {
  const std::vector<hisui::ArchiveItem>& audio_archive_items;
  const std::vector<hisui::ArchiveItem>& normal_archives;
  const std::vector<hisui::ArchiveItem>& preferred_archives;
  const double duration;
};

struct MP4MuxerParametersForLayout {
  const std::vector<hisui::ArchiveItem>& audio_archive_items;
  const std::shared_ptr<VideoProducer>& video_producer;
  const double duration;
};

class MP4Muxer : public Muxer {
 public:
  explicit MP4Muxer(const MP4MuxerParameters&);
  explicit MP4Muxer(const MP4MuxerParametersForLayout&);
  virtual ~MP4Muxer();

 protected:
  std::ofstream m_ofs;
  std::shared_ptr<shiguredo::mp4::writer::Writer> m_writer;
  std::shared_ptr<shiguredo::mp4::track::VideTrack> m_vide_track;
  std::shared_ptr<shiguredo::mp4::track::SounTrack> m_soun_track;
  std::uint64_t m_chunk_interval;

  std::uint64_t m_chunk_start = 0;
  std::vector<hisui::Frame> m_audio_buffer;
  std::vector<hisui::Frame> m_video_buffer;

  void muxFinalize() override;
  void appendAudio(hisui::Frame) override;
  void appendVideo(hisui::Frame) override;

  void writeTrackData();
  void initialize(const hisui::Config&,
                  std::shared_ptr<shiguredo::mp4::writer::Writer>);
  double m_duration;

 private:
  std::vector<hisui::ArchiveItem> m_audio_archives;
  std::vector<hisui::ArchiveItem> m_normal_archives;
  std::vector<hisui::ArchiveItem> m_preferred_archives;
};

}  // namespace hisui::muxer
