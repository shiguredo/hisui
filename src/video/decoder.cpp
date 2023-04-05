#include "video/decoder.hpp"

#include "video/yuv.hpp"
#include "webm/input/video_context.hpp"

namespace hisui::video {

Decoder::Decoder(std::shared_ptr<hisui::webm::input::VideoContext> t_webm)
    : m_webm(t_webm) {
  m_width = m_webm->getWidth();
  m_height = m_webm->getHeight();
  m_duration = static_cast<std::uint64_t>(m_webm->getDuration());
  m_black_yuv_image = create_black_yuv_image(m_width, m_height);
}

std::uint32_t Decoder::getWidth() const {
  return m_width;
}
std::uint32_t Decoder::getHeight() const {
  return m_height;
}

}  // namespace hisui::video
