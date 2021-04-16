#pragma once

#include <cstdint>
#include <string>

#include "video/source.hpp"

namespace hisui::webm::input {

class VideoContext;

}

namespace hisui::video {

class Decoder;
class YUVImage;

class WebMSource : public Source {
 public:
  explicit WebMSource(const std::string&);
  ~WebMSource();
  const YUVImage* getYUV(const std::uint64_t);
  std::uint32_t getWidth() const;
  std::uint32_t getHeight() const;

 private:
  hisui::webm::input::VideoContext* m_webm;
  hisui::video::Decoder* m_decoder = nullptr;
  YUVImage* m_black_yuv_image = nullptr;
  std::uint32_t m_width;
  std::uint32_t m_height;
  std::uint64_t m_duration;

  void readFrame();
};

}  // namespace hisui::video
