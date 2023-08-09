#pragma once

#include <vpl/mfxdefs.h>
#include <vpl/mfxvideo++.h>

#include <cstdint>
#include <memory>
#include <vector>

#include "constants.hpp"
#include "video/decoder.hpp"
#include "video/vpl_session.hpp"

namespace hisui::webm::input {

class VideoContext;

}

namespace hisui::video {

mfxU32 ToMfxCodec(const std::uint32_t fourcc);

class YUVImage;

class VplDecoder {
 public:
  // explicit VplDecoder(std::shared_ptr<hisui::webm::input::VideoContext>);
  //
  // ~VplDecoder();
  //
  // const std::shared_ptr<YUVImage> getImage(const std::uint64_t) override;

  static bool IsSupported(std::shared_ptr<VplSession> session,
                          const std::uint32_t fourcc);

  static std::unique_ptr<MFXVideoDECODE> CreateDecoder(
      std::shared_ptr<VplSession> session,
      mfxU32 codec,
      std::vector<std::pair<int, int>> sizes);

 private:
  //   std::uint64_t m_current_timestamp = 0;
  //   std::uint64_t m_next_timestamp = 0;
  //   std::shared_ptr<YUVImage> m_current_yuv_image = nullptr;
  //   bool m_report_enabled = false;
  static std::unique_ptr<MFXVideoDECODE> CreateDecoderInternal(
      std::shared_ptr<VplSession> session,
      mfxU32 codec,
      int width,
      int height);
};

}  // namespace hisui::video
