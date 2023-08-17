#pragma once

#include <vpl/mfxdefs.h>
#include <vpl/mfxvideo++.h>

#include <cstdint>
#include <memory>
#include <vector>

#include <boost/rational.hpp>

#include "constants.hpp"
#include "video/encoder.hpp"
#include "video/vpl_session.hpp"

namespace hisui::video {

class VPLEncoder /* : public Encoder */ {
 public:
  static bool isSupported(const std::uint32_t fourcc);

 private:
  static std::unique_ptr<MFXVideoENCODE> createEncoder(
      const ::mfxU32 codec,
      const std::uint32_t width,
      const std::uint32_t height,
      const boost::rational<std::uint64_t> frame_rate,
      const std::uint32_t target_bit_rate,
      const std::uint32_t max_bit_rate,
      const bool init);
};
}  // namespace hisui::video
