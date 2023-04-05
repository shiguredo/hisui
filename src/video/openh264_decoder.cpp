#include "video/openh264_decoder.hpp"

#include <bits/exception.h>
#include <codec/api/svc/codec_api.h>
#include <codec/api/svc/codec_app_def.h>
#include <codec/api/svc/codec_def.h>
#include <fmt/core.h>
#include <spdlog/fmt/fmt.h>
#include <spdlog/spdlog.h>

#include <limits>
#include <memory>
#include <stdexcept>

#include "report/reporter.hpp"
#include "video/openh264.hpp"
#include "video/openh264_handler.hpp"
#include "video/yuv.hpp"
#include "webm/input/video_context.hpp"

namespace hisui::video {

OpenH264Decoder::OpenH264Decoder(
    std::shared_ptr<hisui::webm::input::VideoContext> t_webm)
    : Decoder(t_webm) {
  const auto create_decoder_ret =
      OpenH264Handler::getInstance().createDecoder(&m_decoder);
  if (create_decoder_ret != 0 || m_decoder == nullptr) {
    throw std::runtime_error(
        fmt::format("m_h264_handler->createDecoder() failed: error_code={}",
                    create_decoder_ret));
  }
  ::SDecodingParam param;
  param.pFileNameRestructed = nullptr;
  param.uiCpuLoad = 100;  // openh264 のソースをみると利用していないようだ
  param.uiTargetDqLayer = 1;
  param.eEcActiveIdc = ::ERROR_CON_DISABLE;
  param.bParseOnly = false;
  param.sVideoProperty.eVideoBsType = ::VIDEO_BITSTREAM_AVC;
  const auto decoder_initialize_ret = m_decoder->Initialize(&param);
  if (decoder_initialize_ret != 0) {
    throw std::runtime_error(
        fmt::format("m_decoder->Initialize() failed: error_code={}",
                    decoder_initialize_ret));
  }

  m_current_yuv_image =
      std::shared_ptr<YUVImage>(create_black_yuv_image(m_width, m_height));
  m_next_yuv_image =
      std::shared_ptr<YUVImage>(create_black_yuv_image(m_width, m_height));

  if (hisui::report::Reporter::hasInstance()) {
    m_report_enabled = true;

    hisui::report::Reporter::getInstance().registerVideoDecoder(
        m_webm->getFilePath(),
        {.codec = "H.264", .duration = m_webm->getDuration()});

    hisui::report::Reporter::getInstance().registerResolutionChange(
        m_webm->getFilePath(),
        {.timestamp = 0, .width = m_width, .height = m_height});
  }

  m_tmp_yuv[0] = nullptr;
  m_tmp_yuv[1] = nullptr;
  m_tmp_yuv[2] = nullptr;
}

OpenH264Decoder::~OpenH264Decoder() {
  if (m_decoder) {
    m_decoder->Uninitialize();
    OpenH264Handler::getInstance().destroyDecoder(m_decoder);
  }
}

const std::shared_ptr<YUVImage> OpenH264Decoder::getImage(
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
  updateImage(timestamp);
  return m_current_yuv_image;
}

void OpenH264Decoder::updateImage(const std::uint64_t timestamp) {
  // 次のブロックに逹っしていない
  if (timestamp < m_next_timestamp) {
    return;
  }
  // 次以降のブロックに逹っした
  updateImageByTimestamp(timestamp);
}

void OpenH264Decoder::updateImageByTimestamp(const std::uint64_t timestamp) {
  if (m_finished_webm) {
    return;
  }

  do {
    if (m_report_enabled) {
      if (m_current_yuv_image->getWidth(0) != m_next_yuv_image->getWidth(0) ||
          m_current_yuv_image->getHeight(0) != m_next_yuv_image->getHeight(0)) {
        hisui::report::Reporter::getInstance().registerResolutionChange(
            m_webm->getFilePath(), {.timestamp = m_next_timestamp,
                                    .width = m_next_yuv_image->getWidth(0),
                                    .height = m_next_yuv_image->getHeight(0)});
      }
    }
    m_current_yuv_image = m_next_yuv_image;
    m_current_timestamp = m_next_timestamp;
    if (m_webm->readFrame()) {
      ::SBufferInfo buffer_info;
      const auto ret = m_decoder->DecodeFrameNoDelay(
          m_webm->getBuffer(), static_cast<int>(m_webm->getBufferSize()),
          m_tmp_yuv, &buffer_info);
      if (ret != 0) {
        spdlog::error(
            "OpenH264Decoder DecodeFrameNoDelay failed: error_code={}", ret);
        throw std::runtime_error(fmt::format(
            "m_decoder->DecodeFrameNoDelay() failed: error_code={}", ret));
      }
      m_next_timestamp = static_cast<std::uint64_t>(m_webm->getTimestamp());
      if (buffer_info.iBufferStatus == 1) {
        m_next_yuv_image = std::make_shared<YUVImage>(m_width, m_height);
        update_yuv_image_by_openh264_buffer_info(m_next_yuv_image.get(),
                                                 buffer_info);
      }
    } else {
      // m_duration までは m_current_image を出すので webm を読み終えても m_current_image を維持する
      m_finished_webm = true;
      m_next_timestamp = std::numeric_limits<std::uint64_t>::max();
      return;
    }
  } while (timestamp >= m_next_timestamp);
}

}  // namespace hisui::video
