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

struct OpenH264VideoProducerParameters {
  const std::vector<hisui::ArchiveItem>& archives;
  const double duration;
  const std::uint64_t timescale = hisui::Constants::NANO_SECOND;
};

class OpenH264VideoProducer : public VideoProducer {
 public:
  OpenH264VideoProducer(const hisui::Config&,
                        const OpenH264VideoProducerParameters&);
};

}  // namespace hisui::muxer
