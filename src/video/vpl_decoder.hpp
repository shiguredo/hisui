#pragma once

#include <vpl/mfxdefs.h>

#include <cstdint>
#include <memory>

#include "constants.hpp"
#include "video/decoder.hpp"

// namespace hisui::webm::input {
//
// class VideoContext;
//
// }

namespace hisui::video {

static mfxU32 ToMfxCodec(const std::uint32_t fourcc);

// class YUVImage;
//
// class VPLDecoder : public Decoder {
//  public:
//   explicit VPLDecoder(std::shared_ptr<hisui::webm::input::VideoContext>);
//
//   ~VPLDecoder();
//
//   const std::shared_ptr<YUVImage> getImage(const std::uint64_t) override;
//
//  private:
//   std::uint64_t m_current_timestamp = 0;
//   std::uint64_t m_next_timestamp = 0;
//   std::shared_ptr<YUVImage> m_current_yuv_image = nullptr;
//   bool m_report_enabled = false;
//
//   static bool IsSupported(std::shared_ptr<VplSession> session, codec){};
//
}  // namespace hisui::video
