#include "muxer/no_video_producer.hpp"

#include <cstdint>

#include <boost/rational.hpp>

#include "config.hpp"
#include "metadata.hpp"
#include "video/basic_sequencer.hpp"
#include "video/buffer_vpx_encoder.hpp"
#include "video/composer.hpp"
#include "video/grid_composer.hpp"
#include "video/parallel_grid_composer.hpp"
#include "video/sequencer.hpp"
#include "video/vpx.hpp"

namespace hisui::muxer {

NoVideoProducer::NoVideoProducer() {
  m_is_finished = true;
}

}  // namespace hisui::muxer
