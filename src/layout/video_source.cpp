#include "layout/video_source.hpp"

#include <spdlog/spdlog.h>

#include <filesystem>
#include <string>

#include "util/interval.hpp"
#include "video/source.hpp"
#include "video/webm_source.hpp"

namespace hisui::video {

class YUVImage;

}

namespace hisui::layout {

VideoSource::VideoSource(const SourceParameters& params) : Source(params) {
  if (params.testing) {
    spdlog::info("VideoSource for testing");
    setEncodingInterval(1);
    return;
  }
  m_source = std::make_shared<hisui::video::WebMSource>(m_file_path.string());
}

const std::shared_ptr<hisui::video::YUVImage> VideoSource::getYUV(
    const std::uint64_t t) {
  return m_source->getYUV(m_encoding_interval.getSubstructLower(t));
}

}  // namespace hisui::layout
