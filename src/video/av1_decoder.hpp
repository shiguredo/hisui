#pragma once

// EbSvtAv1Dec.h の前に必要
#include <cstdio>

#include <EbSvtAv1Dec.h>
#include <cstdint>
#include <memory>

#include "video/decoder.hpp"

namespace hisui::webm::input {

class VideoContext;

}

namespace hisui::video {

class YUVImage;

class AV1Decoder : public Decoder {
 public:
  explicit AV1Decoder(std::shared_ptr<hisui::webm::input::VideoContext>);

  ~AV1Decoder();

  const std::shared_ptr<YUVImage> getImage(const std::uint64_t);

 private:
  std::uint64_t m_current_timestamp = 0;
  std::uint64_t m_next_timestamp = 0;
  std::shared_ptr<YUVImage> m_current_yuv_image = nullptr;
  bool m_report_enabled = false;
  ::EbComponentType* m_handle;
  ::EbBufferHeaderType* m_recon_buffer;
  ::EbAV1StreamInfo* m_stream_info;
  ::EbAV1FrameInfo* m_frame_info;

  void updateAV1Image(const std::uint64_t);

  void updateAV1ImageByTimestamp(const std::uint64_t);
};

}  // namespace hisui::video
