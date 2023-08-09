#pragma once

#include <vpl/mfxdefs.h>

#include <cstdint>
#include <memory>

#include <vpl/mfxvideo.h>

#include "constants.hpp"
#include "video/decoder.hpp"

namespace hisui::video {

struct VplSession {
  static std::shared_ptr<VplSession> Create();
};

mfxSession GetVplSession(std::shared_ptr<VplSession> session);

}  // namespace hisui::video
