#pragma once

#include <vpl/mfxdefs.h>
#include <vpl/mfxdispatcher.h>
#include <vpl/mfxvideo.h>

#include <cstdint>
#include <memory>

#include "constants.hpp"
#include "video/decoder.hpp"

#include "video/vaapi_utils_drm.h"

namespace hisui::video {

class VPLSession {
 public:
  VPLSession(const VPLSession&) = delete;
  VPLSession& operator=(const VPLSession&) = delete;
  VPLSession(VPLSession&&) = delete;
  VPLSession& operator=(VPLSession&&) = delete;
  static VPLSession& getInstance();
  static bool hasInstance();
  static void open();
  static void close();
  ::mfxSession getSession() const;

 private:
  inline static VPLSession* m_instance = nullptr;

  VPLSession();
  ~VPLSession();

  ::mfxLoader m_loader = nullptr;
  ::mfxSession m_session = nullptr;
  std::unique_ptr<DRMLibVA> m_libva;
};

}  // namespace hisui::video
