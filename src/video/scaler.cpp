#include "video/scaler.hpp"

#include "video/yuv.hpp"

namespace hisui::video {

Scaler::Scaler(const std::uint32_t t_width, const std::uint32_t t_height)
    : m_width(t_width), m_height(t_height) {
  m_scaled = std::make_shared<YUVImage>(m_width, m_height);
}

}  // namespace hisui::video
