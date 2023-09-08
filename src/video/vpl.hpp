#pragma once

#include <vpl/mfxdefs.h>
#include <vpl/mfxvideo++.h>

#include <cstdint>
#include <memory>
#include <vector>

#include "constants.hpp"
#include "video/decoder.hpp"
#include "video/vpl_session.hpp"

namespace hisui::video {

mfxU32 ToMfxCodec(const std::uint32_t fourcc);

}
