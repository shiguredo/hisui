#include "video/webm_source.hpp"

#include <bits/exception.h>
#include <fmt/core.h>
#include <spdlog/fmt/fmt.h>
#include <spdlog/spdlog.h>

#include <stdexcept>

#include "constants.hpp"
#include "video/av1_decoder.hpp"
#include "video/decoder.hpp"
#include "video/decoder_factory.hpp"
#include "video/openh264_decoder.hpp"
#include "video/openh264_handler.hpp"
#include "video/vpl_decoder.hpp"
#include "video/vpl_session.hpp"
#include "video/vpx_decoder.hpp"
#include "video/yuv.hpp"
#include "webm/input/video_context.hpp"

namespace hisui::video {

WebMSource::WebMSource(const std::string& t_file_path) {
  m_webm = std::make_shared<hisui::webm::input::VideoContext>(t_file_path);
  if (!m_webm->init()) {
    spdlog::info(
        "VideoContext initialization failed. no video track, invalid video "
        "track or unsupported codec: file_path={}",
        t_file_path);

    m_webm = nullptr;
    m_width = 320;
    m_height = 240;
    m_black_yuv_image = create_black_yuv_image(m_width, m_height);
    return;
  }

  m_width = m_webm->getWidth();
  m_height = m_webm->getHeight();

  spdlog::trace("WebMSource: file_path={}, width={}, height={}", t_file_path,
                m_width, m_height);

  m_duration = static_cast<std::uint64_t>(m_webm->getDuration());
  m_black_yuv_image = create_black_yuv_image(m_width, m_height);

  m_decoder = hisui::video::DecoderFactory::create(m_webm);
}

const std::shared_ptr<YUVImage> WebMSource::getYUV(
    const std::uint64_t timestamp) {
  if (!m_decoder) {
    return m_black_yuv_image;
  }
  return m_decoder->getImage(timestamp);
}

std::uint32_t WebMSource::getWidth() const {
  return m_width;
}
std::uint32_t WebMSource::getHeight() const {
  return m_height;
}

}  // namespace hisui::video
