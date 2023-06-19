#include "video/av1_decoder.hpp"

#include <EbSvtAv1Dec.h>
#include <bits/exception.h>
#include <fmt/core.h>
#include <spdlog/fmt/fmt.h>
#include <spdlog/spdlog.h>

#include <limits>
#include <memory>
#include <stdexcept>

#include "constants.hpp"
#include "report/reporter.hpp"
#include "video/yuv.hpp"
#include "webm/input/video_context.hpp"

namespace hisui::video {

void set_config(::EbSvtAv1DecConfiguration* config,
                std::uint32_t width,
                std::uint32_t height) {
  config->operating_point = -1;
  config->output_all_layers = 0;
  config->skip_film_grain = 0;
  config->skip_frames = 0;
  config->frames_to_be_decoded = 0;
  config->compressed_ten_bit_format = 0;
  config->eight_bit_output = 1;

  config->max_picture_width = width;
  config->max_picture_height = height;
  config->max_bit_depth = ::EB_EIGHT_BIT;
  config->is_16bit_pipeline = 0;
  config->max_color_format = ::EB_YUV420;

  config->channel_id = 0;
  config->active_channel_count = 1;
  config->stat_report = 0;

  config->threads = 1;
  config->num_p_frames = 1;
}

int set_picture_buffer(::EbSvtIOFormat* pic_buffer,
                       ::EbSvtAv1DecConfiguration* config,
                       std::uint32_t width,
                       std::uint32_t height) {
  auto luma_size = width * height;
  pic_buffer->luma = new std::uint8_t[luma_size];
  pic_buffer->cb = new std::uint8_t[luma_size >> 2];
  pic_buffer->cr = new std::uint8_t[luma_size >> 2];

  pic_buffer->y_stride = width;
  pic_buffer->cb_stride = (width + 1) >> 1;
  pic_buffer->cr_stride = (width + 1) >> 1;
  pic_buffer->width = width;
  pic_buffer->height = height;

  pic_buffer->org_x = 0;
  pic_buffer->org_y = 0;
  pic_buffer->bit_depth = config->max_bit_depth;
  return 0;
}

void update_yuv_image_by_av1_buffer(std::shared_ptr<YUVImage> yuv_image,
                                    const ::EbSvtIOFormat* buffer) {
  const auto bytes_per_sample = (buffer->bit_depth == ::EB_EIGHT_BIT) ? 1 : 2;
  if (bytes_per_sample == 2) {
    throw std::runtime_error("bytes_per_sample == 2 is not suppoted");
  }
  if (buffer->color_fmt != EB_YUV420) {
    throw std::runtime_error(
        fmt::format("only EB_YUV420 format is suppoted: {}",
                    static_cast<std::int32_t>(buffer->color_fmt)));
  }
  auto* buf = buffer->luma;
  auto s = buffer->y_stride;
  auto w = buffer->width;
  auto h = buffer->height;

  yuv_image->setWidthAndHeight(w, h);

  for (std::uint32_t y = 0; y < h; ++y) {
    std::copy_n(buf + y * s, w, yuv_image->yuv[0] + y * w);
  }

  buf = buffer->cb;
  s = buffer->cb_stride;
  w = (w + 1) >> 1;
  h = (h + 1) >> 1;
  for (std::uint32_t y = 0; y < h; ++y) {
    std::copy_n(buf + y * s, w, yuv_image->yuv[1] + y * w);
  }

  buf = buffer->cr;
  s = buffer->cr_stride;
  for (std::uint32_t y = 0; y < h; ++y) {
    std::copy_n(buf + y * s, w, yuv_image->yuv[2] + y * w);
  }
}

AV1Decoder::AV1Decoder(std::shared_ptr<hisui::webm::input::VideoContext> t_webm)
    : Decoder(t_webm) {
  ::EbSvtAv1DecConfiguration config;
  void* app_data = nullptr;
  if (auto err = svt_av1_dec_init_handle(&m_handle, app_data, &config);
      err != ::EB_ErrorNone) {
    throw std::runtime_error(fmt::format("svt_av1_dec_init_handle() failed: {}",
                                         static_cast<std::uint32_t>(err)));
  }
  set_config(&config, m_width, m_height);

  if (auto err = svt_av1_dec_set_parameter(m_handle, &config);
      err != ::EB_ErrorNone) {
    throw std::runtime_error(
        fmt::format("svt_av1_dec_set_parameter() failed: {}",
                    static_cast<std::uint32_t>(err)));
  }
  if (auto err = svt_av1_dec_init(m_handle); err != ::EB_ErrorNone) {
    if (auto err_deinit = svt_av1_dec_deinit_handle(m_handle);
        err_deinit != ::EB_ErrorNone) {
      spdlog::error("svt_av1_dec_deinit_handle() failed: {}",
                    static_cast<std::uint32_t>(err_deinit));
    }
    throw std::runtime_error(fmt::format("svt_av1_dec_init() failed: {}",
                                         static_cast<std::uint32_t>(err)));
  }

  m_recon_buffer = new ::EbBufferHeaderType();
  m_recon_buffer->p_buffer =
      reinterpret_cast<std::uint8_t*>(::malloc(sizeof(::EbSvtIOFormat)));
  ::EbSvtIOFormat* buffer =
      reinterpret_cast<::EbSvtIOFormat*>(m_recon_buffer->p_buffer);
  set_picture_buffer(buffer, &config, m_width, m_height);

  m_stream_info = new ::EbAV1StreamInfo();
  m_frame_info = new ::EbAV1FrameInfo();

  m_current_yuv_image = std::make_shared<YUVImage>(m_width, m_height);

  if (hisui::report::Reporter::hasInstance()) {
    m_report_enabled = true;

    hisui::report::Reporter::getInstance().registerVideoDecoder(
        m_webm->getFilePath(),
        {.codec = "av1", .duration = m_webm->getDuration()});

    hisui::report::Reporter::getInstance().registerResolutionChange(
        m_webm->getFilePath(),
        {.timestamp = 0, .width = m_width, .height = m_height});
  }

  updateAV1ImageByTimestamp(0);
}

AV1Decoder::~AV1Decoder() {
  // 作法的には実施すべきだが, segmentation fault してしまうので実施しない
  // https://gitlab.com/AOMediaCodec/SVT-AV1/-/issues/2005#note_1181213012
  // if (m_handle) {
  //   if (auto err = svt_av1_dec_deinit(m_handle); err != ::EB_ErrorNone) {
  //     spdlog::error("svt_av1_dec_deinit() failed: {}",
  //                   static_cast<std::uint32_t>(err));
  //   }
  // }

  if (m_stream_info) {
    delete m_stream_info;
  }

  if (m_frame_info) {
    delete m_frame_info;
  }

  if (m_recon_buffer) {
    if (m_recon_buffer->p_buffer) {
      ::EbSvtIOFormat* buffer =
          reinterpret_cast<::EbSvtIOFormat*>(m_recon_buffer->p_buffer);
      if (buffer->luma) {
        delete[] buffer->luma;
      }
      if (buffer->cb) {
        delete[] buffer->cb;
      }
      if (buffer->cr) {
        delete[] buffer->cr;
      }
      ::free(m_recon_buffer->p_buffer);
    }
    delete m_recon_buffer;
  }

  if (m_handle) {
    if (auto err = svt_av1_dec_deinit_handle(m_handle); err != ::EB_ErrorNone) {
      spdlog::error("svt_av1_dec_deinit_handle() failed: {}",
                    static_cast<std::uint32_t>(err));
    }
  }
}

const std::shared_ptr<YUVImage> AV1Decoder::getImage(
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

  updateAV1Image(timestamp);
  return m_current_yuv_image;
}

void AV1Decoder::updateAV1Image(const std::uint64_t timestamp) {
  // 次のブロックに逹していない
  if (timestamp < m_next_timestamp) {
    return;
  }
  // 次以降のブロックに逹した
  updateAV1ImageByTimestamp(timestamp);
}

void AV1Decoder::updateAV1ImageByTimestamp(const std::uint64_t timestamp) {
  if (m_finished_webm) {
    return;
  }

  do {
    m_current_timestamp = m_next_timestamp;
    if (m_webm->readFrame()) {
      spdlog::trace("webm->getBufferSize(): {}", m_webm->getBufferSize());
      if (auto err = svt_av1_dec_frame(m_handle, m_webm->getBuffer(),
                                       m_webm->getBufferSize(), 0);
          err != ::EB_ErrorNone) {
        throw std::runtime_error(fmt::format("svt_av1_dec_frame() failed: {}",
                                             static_cast<std::uint32_t>(err)));
      }
      if (svt_av1_dec_get_picture(m_handle, m_recon_buffer, m_stream_info,
                                  m_frame_info) != ::EB_DecNoOutputPicture) {
        ::EbSvtIOFormat* buffer =
            reinterpret_cast<::EbSvtIOFormat*>(m_recon_buffer->p_buffer);

        if (m_report_enabled) {
          if (m_current_yuv_image->getWidth(0) != buffer->width ||
              m_current_yuv_image->getHeight(0) != buffer->height) {
            hisui::report::Reporter::getInstance().registerResolutionChange(
                m_webm->getFilePath(), {.timestamp = m_next_timestamp,
                                        .width = buffer->width,
                                        .height = buffer->height});
          }
        }

        update_yuv_image_by_av1_buffer(m_current_yuv_image, buffer);
      }
      m_next_timestamp = static_cast<std::uint64_t>(m_webm->getTimestamp());
    } else {
      m_finished_webm = true;
      m_next_timestamp = std::numeric_limits<std::uint64_t>::max();
      return;
    }
  } while (timestamp >= m_next_timestamp);
}

}  // namespace hisui::video
