#pragma once

#include <vpl/mfxdefs.h>

#include <cstdint>
#include <memory>

#include <vpl/mfxdispatcher.h>
#include <vpl/mfxvideo.h>

#include "constants.hpp"
#include "video/decoder.hpp"

#include "video/vaapi_utils_drm.h"

namespace hisui::video {

class VplSession {
 public:
  VplSession(const VplSession&) = delete;
  VplSession& operator=(const VplSession&) = delete;
  VplSession(VplSession&&) = delete;
  VplSession& operator=(VplSession&&) = delete;
  static VplSession& getInstance();
  static bool hasInstance();
  static void open();
  static void close();
  ::mfxSession getSession();

 private:
  inline static VplSession* m_instance = nullptr;

  VplSession();
  ~VplSession();

  ::mfxLoader m_loader = nullptr;
  ::mfxSession m_session = nullptr;
  std::unique_ptr<DRMLibVA> m_libva;
};

}  // namespace hisui::video
