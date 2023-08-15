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

class VplDecoder /* : public Decoder */ {
 public:
  // explicit VplDecoder(std::shared_ptr<hisui::webm::input::VideoContext>);
  // ~VplDecoder();
  //
  // const std::shared_ptr<YUVImage> getImage(const std::uint64_t) override;

  static bool IsSupported(const std::shared_ptr<VplSession> session,
                          const std::uint32_t fourcc);

 private:
  std::shared_ptr<VplSession> m_session;
  std::unique_ptr<MFXVideoDECODE> m_decoder;
  std::uint64_t m_current_timestamp = 0;
  std::uint64_t m_next_timestamp = 0;
  std::shared_ptr<YUVImage> m_current_yuv_image = nullptr;
  bool m_report_enabled = false;

  static std::unique_ptr<MFXVideoDECODE> CreateDecoder(
      const std::shared_ptr<VplSession> session,
      const mfxU32 codec,
      const std::vector<std::pair<std::uint32_t, std::uint32_t>> sizes);

  static std::unique_ptr<MFXVideoDECODE> CreateDecoderInternal(
      const std::shared_ptr<VplSession> session,
      const mfxU32 codec,
      const std::uint32_t width,
      const std::uint32_t height);
};

}  // namespace hisui::video
