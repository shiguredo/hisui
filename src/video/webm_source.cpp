#include "video/webm_source.hpp"

#include <bits/exception.h>
#include <fmt/core.h>
#include <spdlog/fmt/fmt.h>
#include <spdlog/spdlog.h>

#include <stdexcept>

#include "constants.hpp"
#include "video/decoder.hpp"
#include "video/openh264_decoder.hpp"
#include "video/openh264_handler.hpp"
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

  switch (m_webm->getFourcc()) {
    case hisui::Constants::VP8_FOURCC: /* fall through */
    case hisui::Constants::VP9_FOURCC:
      m_decoder = std::make_shared<VPXDecoder>(m_webm);
      break;
    case hisui::Constants::H264_FOURCC:
      if (OpenH264Handler::hasInstance()) {
        m_decoder = std::make_shared<OpenH264Decoder>(m_webm);
        break;
      }
      throw std::runtime_error("openh264 library is not loaded");
    default:
      const auto fourcc = m_webm->getFourcc();
      m_webm = nullptr;
      throw std::runtime_error(fmt::format("unknown fourcc: {}", fourcc));
  }
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
