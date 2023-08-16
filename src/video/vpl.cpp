#include "video/vpl.hpp"

#include <fmt/core.h>
#include <vpl/mfxdefs.h>
#include <vpl/mfxvp8.h>

#include <cstdint>
#include <stdexcept>

#include "constants.hpp"

namespace hisui::video {

mfxU32 ToMfxCodec(const std::uint32_t fourcc) {
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
      throw std::invalid_argument(fmt::format("unknown fourcc: {:x}", fourcc));
  }
}

}  // namespace hisui::video
