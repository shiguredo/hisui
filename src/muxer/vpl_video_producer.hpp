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

struct VPLVideoProducerParameters {
  const std::vector<hisui::ArchiveItem>& archives;
  const double duration;
  const std::uint64_t timescale = hisui::Constants::NANO_SECOND;
};

class VPLVideoProducer : public VideoProducer {
 public:
  VPLVideoProducer(const hisui::Config&,
                   const VPLVideoProducerParameters&,
                   const std::uint32_t);
};

}  // namespace hisui::muxer
