#include "video/vpl_session.hpp"

#include <fmt/core.h>
#include <spdlog/spdlog.h>
#include <vpl/mfxdispatcher.h>
#include <vpl/mfxvideo.h>

#include <iostream>

#include "video/vaapi_utils_drm.h"

namespace hisui::video {

VPLSession::VPLSession() {}

VPLSession::~VPLSession() {
  ::MFXClose(m_session);
  ::MFXUnload(m_loader);
}

bool VPLSession::hasInstance() {
  return m_instance != nullptr;
}

VPLSession& VPLSession::getInstance() {
  return *m_instance;
}

void VPLSession::close() {
  delete m_instance;
  m_instance = nullptr;
}

void VPLSession::open() {
  auto session = new VPLSession();
  ::mfxStatus sts = MFX_ERR_NONE;

  session->m_loader = ::MFXLoad();
  if (session->m_loader == nullptr) {
    delete session;
    spdlog::warn("::MFXLoad() failed");
    return;
  }

  MFX_ADD_PROPERTY_U32(session->m_loader, "mfxImplDescription.Impl",
                       MFX_IMPL_TYPE_HARDWARE);

  sts = ::MFXCreateSession(session->m_loader, 0, &session->m_session);
  if (sts != MFX_ERR_NONE) {
    delete session;
    spdlog::warn("MFXCreateSession() failed");
    return;
  }

  session->m_libva = CreateDRMLibVA();
  if (!session->m_libva) {
    delete session;
    spdlog::warn("CreateDRMLibVA() failed");
    return;
  }

  sts = ::MFXVideoCORE_SetHandle(
      session->m_session, static_cast<::mfxHandleType>(MFX_HANDLE_VA_DISPLAY),
      session->m_libva->GetVADisplay());
  if (sts != MFX_ERR_NONE) {
    delete session;
    throw std::runtime_error(fmt::format("MFXVideoCORE_SetHandle() failed: {}",
                                         static_cast<std::int32_t>(sts)));
  }

  // Query selected implementation and version
  ::mfxIMPL impl;
  sts = ::MFXQueryIMPL(session->m_session, &impl);
  if (sts != MFX_ERR_NONE) {
    delete session;
    throw std::runtime_error(fmt::format("MFXQueryIMPL() failed: {}",
                                         static_cast<std::int32_t>(sts)));
  }

  ::mfxVersion ver;
  sts = ::MFXQueryVersion(session->m_session, &ver);
  if (sts != MFX_ERR_NONE) {
    delete session;
    throw std::runtime_error(fmt::format("MFXQueryVersion() failed: {}",
                                         static_cast<std::int32_t>(sts)));
  }

  m_instance = session;
}

::mfxSession VPLSession::getSession() {
  return m_session;
}

}  // namespace hisui::video
