#include "video/vpl_decoder.hpp"

#include <fmt/core.h>
#include <vpl/mfxdefs.h>
#include <vpl/mfxstructures.h>
#include <vpl/mfxvp8.h>

namespace hisui::video {

static mfxU32 ToMfxCodec(const std::uint32_t fourcc) {
  switch (fourcc) {
    case hisui::Constants::VP8_FOURCC:
      return MFX_CODEC_VP8;
    case hisui::Constants::VP9_FOURCC:
      return MFX_CODEC_VP9;
    case hisui::Constants::H264_FOURCC:
      return MFX_CODEC_AVC;
    case hisui::Constants::AV1_FOURCC:
      return MFX_CODEC_AV1;
    default:
      throw std::runtime_error(fmt::format("unknown fourcc: {:x}", fourcc));
  }
}
}  // namespace hisui::video
