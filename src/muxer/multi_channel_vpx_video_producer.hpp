#pragma once

#include <cstdint>

#include "constants.hpp"
#include "muxer/video_producer.hpp"

namespace hisui {

class Config;
class Metadata;

}  // namespace hisui

namespace hisui::muxer {

class MultiChannelVPXVideoProducer : public VideoProducer {
 public:
  MultiChannelVPXVideoProducer(
      const hisui::Config&,
      const hisui::Metadata&,
      const hisui::Metadata&,
      const std::uint64_t timescale = hisui::Constants::NANO_SECOND);
  ~MultiChannelVPXVideoProducer();
  void produce() override;

 private:
  hisui::video::Composer* m_original_channel_composer = nullptr;
  hisui::video::Composer* m_alternative_channel_composer = nullptr;
};

}  // namespace hisui::muxer
