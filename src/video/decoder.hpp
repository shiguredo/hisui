#pragma once

#include <cstdint>
#include <memory>

namespace hisui::webm::input {
class VideoContext;
}

namespace hisui::video {

class YUVImage;

class Decoder {
 public:
  explicit Decoder(std::shared_ptr<hisui::webm::input::VideoContext>);

  virtual ~Decoder() = default;
  virtual const std::shared_ptr<YUVImage> getImage(const std::uint64_t) = 0;

  std::uint32_t getWidth() const;
  std::uint32_t getHeight() const;

 protected:
  std::shared_ptr<hisui::webm::input::VideoContext> m_webm;
  std::uint64_t m_duration;
  bool m_is_time_over = false;
  bool m_finished_webm = false;
  std::uint32_t m_width;
  std::uint32_t m_height;
  std::shared_ptr<YUVImage> m_black_yuv_image;
};

}  // namespace hisui::video
