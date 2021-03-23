#pragma once

#include "muxer/video_producer.hpp"

namespace hisui::muxer {

class NoVideoProducer : public VideoProducer {
 public:
  NoVideoProducer();
};

}  // namespace hisui::muxer
