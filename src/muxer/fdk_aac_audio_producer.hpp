#pragma once

#include <vector>

#include "muxer/audio_producer.hpp"

namespace hisui {

class Config;
class MetadataSet;

}  // namespace hisui

namespace hisui::muxer {

struct FDKAACAudioProducerParameters {
  const std::vector<hisui::ArchiveItem>& archives;
  const double duration;
};

class FDKAACAudioProducer : public AudioProducer {
 public:
  FDKAACAudioProducer(const hisui::Config&,
                      const FDKAACAudioProducerParameters&);
};

}  // namespace hisui::muxer
