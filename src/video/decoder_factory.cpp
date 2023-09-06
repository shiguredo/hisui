#include "video/decoder_factory.hpp"

#include <cstdint>
#include <memory>

#include "config.hpp"
#include "constants.hpp"
#include "video/av1_decoder.hpp"
#include "video/openh264_decoder.hpp"
#include "video/openh264_handler.hpp"
#include "video/vpx_decoder.hpp"
#include "webm/input/video_context.hpp"

#ifdef USE_ONEVPL
#include "video/vpl_session.hpp"
#endif

namespace hisui::video {

void DecoderFactory::setup(const hisui::Config& config) {
  auto factory = new DecoderFactory(config);
  m_instance = std::unique_ptr<DecoderFactory>(factory);
}

DecoderFactory::DecoderFactory(const hisui::Config& t_config)
    : m_config(t_config) {}

std::shared_ptr<hisui::video::Decoder> DecoderFactory::create(
    std::shared_ptr<hisui::webm::input::VideoContext> webm) {
  auto fourcc = webm->getFourcc();
  switch (fourcc) {
    case hisui::Constants::VP8_FOURCC: /* fall through */
    case hisui::Constants::VP9_FOURCC:
      return std::make_shared<VPXDecoder>(webm);
    case hisui::Constants::AV1_FOURCC:
      return std::make_shared<AV1Decoder>(webm);
    case hisui::Constants::H264_FOURCC:
      if (OpenH264Handler::hasInstance()) {
        return std::make_shared<OpenH264Decoder>(webm);
      }
      throw std::runtime_error("H.264 decoder is unavailable");
    default:
      throw std::runtime_error(fmt::format("unknown fourcc: {}", fourcc));
  }
}

}  // namespace hisui::video
