#pragma once

#include "muxer/audio_producer.hpp"

namespace hisui {

class Config;
class MetadataSet;

}  // namespace hisui

namespace hisui::muxer {

class FDKAACAudioProducer : public AudioProducer {
 public:
  FDKAACAudioProducer(const hisui::Config&, const hisui::MetadataSet&);
};

}  // namespace hisui::muxer
