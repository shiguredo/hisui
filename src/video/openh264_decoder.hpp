#pragma once

#include <cstdint>
#include <memory>

#include "video/decoder.hpp"

class ISVCDecoder;

namespace hisui::webm::input {

class VideoContext;

}

namespace hisui::video {

class YUVImage;

class OpenH264Decoder : public Decoder {
 public:
  explicit OpenH264Decoder(std::shared_ptr<hisui::webm::input::VideoContext>);
  ~OpenH264Decoder();

  const std::shared_ptr<YUVImage> getImage(const std::uint64_t);

 private:
  ::ISVCDecoder* m_decoder = nullptr;
  std::uint64_t m_current_timestamp = 0;
  std::uint64_t m_next_timestamp = 0;
  std::shared_ptr<YUVImage> m_current_yuv_image = nullptr;
  std::shared_ptr<YUVImage> m_next_yuv_image = nullptr;
  std::uint8_t* m_tmp_yuv[3];
  bool m_report_enabled = false;

  void updateImage(const std::uint64_t);

  void updateImageByTimestamp(const std::uint64_t);
};

}  // namespace hisui::video
