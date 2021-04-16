#pragma once

#include <cstdint>

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

class MultiChannelVPXVideoProducer : public VideoProducer {
 public:
  MultiChannelVPXVideoProducer(
      const hisui::Config&,
      const hisui::MetadataSet&,
      const std::uint64_t timescale = hisui::Constants::NANO_SECOND);
  ~MultiChannelVPXVideoProducer();
  void produce() override;

 private:
  hisui::video::Composer* m_normal_channel_composer = nullptr;
  hisui::video::Composer* m_preferred_channel_composer = nullptr;

  const std::uint32_t m_normal_bit_rate;
  const std::uint32_t m_preferred_bit_rate;
};

}  // namespace hisui::muxer
