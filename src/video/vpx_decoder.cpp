#include "video/vpx_decoder.hpp"

#include <bits/exception.h>
#include <fmt/core.h>
#include <spdlog/fmt/fmt.h>
#include <spdlog/spdlog.h>
#include <vpx/vpx_codec.h>
#include <vpx/vpx_decoder.h>
#include <vpx/vpx_image.h>

#include <limits>
#include <memory>
#include <stdexcept>

#include "constants.hpp"
#include "report/reporter.hpp"
#include "video/vpx.hpp"
#include "video/yuv.hpp"
#include "webm/input/video_context.hpp"

namespace hisui::video {

VPXDecoder::VPXDecoder(std::shared_ptr<hisui::webm::input::VideoContext> t_webm)
    : Decoder(t_webm) {
  create_vpx_codec_ctx_t_for_decoding(&m_codec, m_webm->getFourcc());

  m_current_yuv_image = std::make_shared<YUVImage>(m_width, m_height);

  m_next_vpx_image = create_black_vpx_image(m_width, m_height);

  if (hisui::report::Reporter::hasInstance()) {
    m_report_enabled = true;

    hisui::report::Reporter::getInstance().registerVideoDecoder(
        m_webm->getFilePath(),
        {.codec = m_webm->getFourcc() == hisui::Constants::VP9_FOURCC ? "vp9"
                                                                      : "vp8",
         .duration = m_webm->getDuration()});

    hisui::report::Reporter::getInstance().registerResolutionChange(
        m_webm->getFilePath(),
        {.timestamp = 0, .width = m_width, .height = m_height});
  }

  updateVPXImageByTimestamp(0);
}

VPXDecoder::~VPXDecoder() {
  if (m_next_vpx_image && m_current_vpx_image != m_next_vpx_image) {
    ::vpx_img_free(m_next_vpx_image);
  }
  if (m_current_vpx_image) {
    ::vpx_img_free(m_current_vpx_image);
  }
  ::vpx_codec_destroy(&m_codec);
}

const std::shared_ptr<YUVImage> VPXDecoder::getImage(
    const std::uint64_t timestamp) {
  // 非対応 WebM or 時間超過
  if (!m_webm || m_is_time_over) {
    return m_black_yuv_image;
  }
  // 時間超過した
  if (m_duration <= timestamp) {
    m_is_time_over = true;
    return m_black_yuv_image;
  }
  // 極端におおきな fps にはしないと思うので m_current_yuv_image への変換は毎回やってよいと考えている
  updateVPXImage(timestamp);
  update_yuv_image_by_vpx_image(m_current_yuv_image, m_current_vpx_image);
  return m_current_yuv_image;
}

void VPXDecoder::updateVPXImage(const std::uint64_t timestamp) {
  // 次のブロックに逹っしていない
  if (timestamp < m_next_timestamp) {
    return;
  }
  // 次以降のブロックに逹っした
  updateVPXImageByTimestamp(timestamp);
}

void VPXDecoder::updateVPXImageByTimestamp(const std::uint64_t timestamp) {
  if (m_finished_webm) {
    return;
  }

  do {
    ::vpx_codec_iter_t codec_iter = nullptr;
    if (m_current_vpx_image) {
      if (m_report_enabled) {
        if (get_vpx_image_plane_width(m_current_vpx_image, 0) !=
                get_vpx_image_plane_width(m_next_vpx_image, 0) ||
            get_vpx_image_plane_height(m_current_vpx_image, 0) !=
                get_vpx_image_plane_height(m_next_vpx_image, 0)) {
          hisui::report::Reporter::getInstance().registerResolutionChange(
              m_webm->getFilePath(),
              {.timestamp = m_next_timestamp,
               .width = get_vpx_image_plane_width(m_next_vpx_image, 0),
               .height = get_vpx_image_plane_height(m_next_vpx_image, 0)});
        }
      }
      ::vpx_img_free(m_current_vpx_image);
    }
    m_current_vpx_image = m_next_vpx_image;
    m_current_timestamp = m_next_timestamp;
    if (m_webm->readFrame()) {
      spdlog::trace("webm->getBufferSize(): {}", m_webm->getBufferSize());
      const auto ret = ::vpx_codec_decode(
          &m_codec, m_webm->getBuffer(),
          static_cast<unsigned int>(m_webm->getBufferSize()), nullptr, 0);
      if (ret != VPX_CODEC_OK) {
        spdlog::warn("vpx_codec_decode() failed: error_code={}", ret);
        const char* detail = ::vpx_codec_error_detail(&m_codec);
        if (detail != nullptr) {
          spdlog::warn("vpx_codec_decode() error detail: {}", detail);
        }
        throw std::runtime_error(
            fmt::format("vpx_codec_decode() failed: error_code={}", ret));
      }
      m_next_vpx_image = ::vpx_codec_get_frame(&m_codec, &codec_iter);
      if (!m_next_vpx_image) {
        throw std::runtime_error("vpx_codec_get_frame() failed");
      }
      m_next_timestamp = static_cast<std::uint64_t>(m_webm->getTimestamp());
    } else {
      // m_duration までは m_current_image を出すので webm を読み終えても m_current_image を維持する
      m_finished_webm = true;
      if (m_next_vpx_image && m_current_vpx_image != m_next_vpx_image) {
        ::vpx_img_free(m_next_vpx_image);
      }
      m_next_vpx_image = nullptr;
      m_next_timestamp = std::numeric_limits<std::uint64_t>::max();
      return;
    }
  } while (timestamp >= m_next_timestamp);
}

}  // namespace hisui::video
