#include "muxer/no_video_producer.hpp"

namespace hisui::muxer {

NoVideoProducer::NoVideoProducer() : VideoProducer({.is_finished = true}) {}

}  // namespace hisui::muxer
