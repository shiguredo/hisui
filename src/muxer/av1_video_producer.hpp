#pragma once

#include <cstdint>
#include <vector>

#include "archive_item.hpp"
#include "constants.hpp"
#include "muxer/video_producer.hpp"

namespace hisui {

class Config;
class Metadata;

}  // namespace hisui

namespace hisui::muxer {

struct AV1VideoProducerParameters {
  const std::vector<hisui::ArchiveItem>& archives;
  const double duration;
  const std::uint64_t timescale = hisui::Constants::NANO_SECOND;
};

class AV1VideoProducer : public VideoProducer {
 public:
  AV1VideoProducer(const hisui::Config&, const AV1VideoProducerParameters&);
  const std::vector<std::uint8_t>& getExtraData() const override;
};

}  // namespace hisui::muxer
