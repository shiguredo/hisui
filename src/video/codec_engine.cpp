#include "video/codec_engine.hpp"

#include <fmt/core.h>

#include <iostream>
#include <string>

#include "constants.hpp"
#include "video/openh264_handler.hpp"

#ifdef USE_ONEVPL
#include "video/vpl_encoder.hpp"
#include "video/vpl_session.hpp"
#endif

namespace hisui::video {

void printEngine(const std::string& name,
                 const std::string& type,
                 const bool is_default) {
  std::cout << fmt::format("    - {} [{}]", name, type);
  if (is_default) {
    std::cout << " (default)";
  }
  std::cout << std::endl;
}

void showCodecEngines() {
  std::cout << "VP8:" << std::endl;
  std::cout << "  Encoder:" << std::endl;
  {
    bool is_default = true;
    printEngine("libvpx", "software", is_default);
  }
  std::cout << "  Decoder:" << std::endl;
  {
    bool is_default = true;
    printEngine("libvpx", "software", is_default);
  }

  std::cout << "VP9:" << std::endl;
  std::cout << "  Encoder:" << std::endl;
  {
    bool is_default = true;
    printEngine("libvpx", "software", is_default);
  }
  std::cout << "  Decoder:" << std::endl;
  {
    bool is_default = true;
    printEngine("libvpx", "software", is_default);
  }

  std::cout << "AV1:" << std::endl;
  std::cout << "  Encoder:" << std::endl;
  {
    bool is_default = true;
    printEngine("SVT-AV1", "software", is_default);
  }
  std::cout << "  Decoder:" << std::endl;
  {
    bool is_default = true;
    printEngine("SVT-AV1", "software", is_default);
  }

  std::cout << "H264:" << std::endl;
  std::cout << "  Encoder:" << std::endl;
  {
    bool is_default = true;
#ifdef USE_ONEVPL
    if (VPLSession::hasInstance() &&
        VPLEncoder::isSupported(hisui::Constants::H264_FOURCC)) {
      printEngine("Intel oneVPL", "intel", is_default);
      is_default = false;
    }
#endif
    if (OpenH264Handler::hasInstance()) {
      printEngine("OpenH264", "software", is_default);
    }
  }
  std::cout << "  Decoder:" << std::endl;
  {
    bool is_default = true;
    if (OpenH264Handler::hasInstance()) {
      printEngine("OpenH264", "software", is_default);
    }
  }
}

}  // namespace hisui::video
