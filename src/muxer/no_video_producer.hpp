#pragma once

#include <cstdint>

#include "constants.hpp"
#include "muxer/video_producer.hpp"

namespace hisui {

class Config;
class Metadata;

}  // namespace hisui

namespace hisui::muxer {

class NoVideoProducer : public VideoProducer {
 public:
  NoVideoProducer();
};

}  // namespace hisui::muxer
