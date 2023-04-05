#pragma once

#include <cstdint>
#include <memory>
#include <vector>

#include "archive_item.hpp"
#include "constants.hpp"
#include "muxer/video_producer.hpp"

namespace hisui {

class Config;
class MetadataSet;

}  // namespace hisui

namespace hisui::video {

class Composer;

}

namespace hisui::muxer {

struct MultiChannelVPXVideoProducerParameters {
  const std::vector<hisui::ArchiveItem>& normal_archives = {};
  const std::vector<hisui::ArchiveItem>& preferred_archives = {};
  const double duration;
  const std::uint64_t timescale = hisui::Constants::NANO_SECOND;
};

class MultiChannelVPXVideoProducer : public VideoProducer {
 public:
  MultiChannelVPXVideoProducer(const hisui::Config&,
                               const MultiChannelVPXVideoProducerParameters&);

  void produce() override;

 private:
  std::shared_ptr<hisui::video::Composer> m_normal_channel_composer;
  std::shared_ptr<hisui::video::Composer> m_preferred_channel_composer;

  const std::uint32_t m_normal_bit_rate;
  const std::uint32_t m_preferred_bit_rate;
};

}  // namespace hisui::muxer
