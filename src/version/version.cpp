#include "version/version.hpp"

#include <EbSvtAv1.h>
#include <codec/api/wels/codec_app_def.h>
#include <fmt/core.h>
#include <mkvparser/mkvparser.h>
#include <vpx/vpx_codec.h>

#ifdef USE_FDK_AAC
#include <fdk-aac/FDK_audio.h>
#include <fdk-aac/aacenc_lib.h>
#endif

#include <string>

#include "shiguredo/mp4/version.hpp"
#include "video/openh264_handler.hpp"

namespace hisui::version {

std::string get_libvpx_version() {
  return fmt::format("{}", ::vpx_codec_version_str());
}

std::string get_libwebm_version() {
  int maj, min, build, rev;
  mkvparser::GetVersion(maj, min, build, rev);
  return fmt::format("{}.{}.{}.{}", maj, min, build, rev);
}

std::string get_openh264_version() {
  if (!hisui::video::OpenH264Handler::hasInstance()) {
    return "";
  }
  auto version = hisui::video::OpenH264Handler::getInstance().getCodecVersion();
  return fmt::format("{}.{}.{}", version.uMajor, version.uMinor,
                     version.uRevision);
}

std::string get_svt_av1_version() {
  return fmt::format("{}.{}.{}", SVT_AV1_VERSION_MAJOR, SVT_AV1_VERSION_MINOR,
                     SVT_AV1_VERSION_PATCHLEVEL);
}

#ifdef USE_FDK_AAC
std::string get_fdkaac_aacenc_version() {
  ::LIB_INFO info[FDK_MODULE_LAST];
  ::FDKinitLibInfo(info);
  ::aacEncGetLibInfo(info);
  return fmt::format("{}.{}.{}", (info[FDK_AACENC].version >> 24) & 0xff,
                     (info[FDK_AACENC].version >> 16) & 0xff,
                     (info[FDK_AACENC].version >> 8) & 0xff);
}
#endif

std::string get_hisui_version() {
  return "2023.1.1";
}

std::string get_cppmp4_version() {
  return shiguredo::mp4::get_version_string();
}

}  // namespace hisui::version
