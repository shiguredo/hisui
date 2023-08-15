#include "video/vpl_session.hpp"

#include <fmt/core.h>
#include <spdlog/spdlog.h>
#include <vpl/mfxdispatcher.h>
#include <vpl/mfxvideo.h>

#include <iostream>

#include "video/vaapi_utils_drm.h"

namespace hisui::video {

struct VplSessionImpl : VplSession {
  ~VplSessionImpl();

  mfxLoader loader = nullptr;
  mfxSession session = nullptr;

  std::unique_ptr<DRMLibVA> libva;
};

VplSessionImpl::~VplSessionImpl() {
  MFXClose(session);
  MFXUnload(loader);
}

std::shared_ptr<VplSession> VplSession::Create() {
  std::shared_ptr<VplSessionImpl> session(new VplSessionImpl());

  mfxStatus sts = MFX_ERR_NONE;

  session->loader = MFXLoad();
  if (session->loader == nullptr) {
    spdlog::warn("MFXLoad() failed");
    return nullptr;
  }

  MFX_ADD_PROPERTY_U32(session->loader, "mfxImplDescription.Impl",
                       MFX_IMPL_TYPE_HARDWARE);

  sts = MFXCreateSession(session->loader, 0, &session->session);
  if (sts != MFX_ERR_NONE) {
    spdlog::warn("MFXCreateSession() failed");
    return nullptr;
  }

  session->libva = CreateDRMLibVA();
  if (!session->libva) {
    spdlog::warn("CreateDRMLibVA() failed");
    return nullptr;
  }

  sts = MFXVideoCORE_SetHandle(
      session->session, static_cast<mfxHandleType>(MFX_HANDLE_VA_DISPLAY),
      session->libva->GetVADisplay());
  if (sts != MFX_ERR_NONE) {
    throw std::runtime_error(fmt::format("MFXVideoCORE_SetHandle() failed: {}",
                                         static_cast<std::int32_t>(sts)));
  }

  // Query selected implementation and version
  mfxIMPL impl;
  sts = MFXQueryIMPL(session->session, &impl);
  if (sts != MFX_ERR_NONE) {
    throw std::runtime_error(fmt::format("MFXQueryIMPL() failed: {}",
                                         static_cast<std::int32_t>(sts)));
  }

  mfxVersion ver;
  sts = MFXQueryVersion(session->session, &ver);
  if (sts != MFX_ERR_NONE) {
    throw std::runtime_error(fmt::format("MFXQueryVersion() failed: {}",
                                         static_cast<std::int32_t>(sts)));
  }

  return session;
}

mfxSession GetVplSession(std::shared_ptr<VplSession> session) {
  return std::static_pointer_cast<VplSessionImpl>(session)->session;
}

}  // namespace hisui::video
