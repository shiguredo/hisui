#pragma once

#include <memory>
#include <vector>

#include "config.hpp"
#include "metadata.hpp"
#include "muxer/muxer.hpp"
#include "webm/output/context.hpp"

namespace hisui {

struct Frame;

}

namespace hisui::muxer {

struct AsyncWebMMuxerParameters {
  const std::vector<hisui::ArchiveItem>& audio_archive_items;
  const std::vector<hisui::ArchiveItem>& normal_archives;
  const std::vector<hisui::ArchiveItem>& preferred_archives;
  const double duration;
};

struct AsyncWebMMuxerParametersForLayout {
  const std::vector<hisui::ArchiveItem>& audio_archive_items;
  const std::shared_ptr<VideoProducer>& video_producer;
  const double duration;
};

class AsyncWebMMuxer : public Muxer {
 public:
  AsyncWebMMuxer(const hisui::Config&, const AsyncWebMMuxerParameters&);
  AsyncWebMMuxer(const hisui::Config&,
                 const AsyncWebMMuxerParametersForLayout&);

  void setUp() override;
  void run() override;
  void cleanUp() override;

 private:
  void muxFinalize() override;
  void appendAudio(hisui::Frame) override;
  void appendVideo(hisui::Frame) override;

  std::unique_ptr<hisui::webm::output::Context> m_context;

  bool has_preferred;
  hisui::Config m_config;
  std::vector<hisui::ArchiveItem> m_audio_archives;
  std::vector<hisui::ArchiveItem> m_normal_archives;
  std::vector<hisui::ArchiveItem> m_preferred_archives;
  double m_duration;
  std::size_t m_normal_archive_size;
};

}  // namespace hisui::muxer
