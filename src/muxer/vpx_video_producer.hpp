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

struct VPXVideoProducerParameters {
  const std::vector<hisui::ArchiveItem>& archives;
  const double duration;
  const std::uint64_t timescale = hisui::Constants::NANO_SECOND;
};

class VPXVideoProducer : public VideoProducer {
 public:
  VPXVideoProducer(const hisui::Config&, const VPXVideoProducerParameters&);
};

}  // namespace hisui::muxer
