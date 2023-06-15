#include "video/buffer_av1_encoder.hpp"

#include <EbSvtAv1Enc.h>

#include <bits/exception.h>
#include <fmt/core.h>
#include <spdlog/fmt/fmt.h>
#include <spdlog/spdlog.h>

#include <algorithm>
#include <stdexcept>

#include <boost/rational.hpp>

#include "config.hpp"
#include "frame.hpp"

namespace hisui::video {

AV1EncoderConfig::AV1EncoderConfig(const std::uint32_t t_width,
                                   const std::uint32_t t_height,
                                   const hisui::Config& config)
    : width(t_width),
      height(t_height),
      fps(config.out_video_frame_rate),
      fourcc(config.out_video_codec),
      bitrate(config.out_video_bit_rate) {}

BufferAV1Encoder::BufferAV1Encoder(std::queue<hisui::Frame>* t_buffer,
                                   const AV1EncoderConfig& config,
                                   const std::uint64_t t_timescale)
    : m_buffer(t_buffer), m_timescale(t_timescale) {
  m_width = config.width;
  m_height = config.height;
  m_fps = config.fps;
  m_fourcc = config.fourcc;
  m_bitrate = config.bitrate;

  void* app_data = nullptr;
  if (auto err =
          ::svt_av1_enc_init_handle(&m_handle, app_data, &m_av1_enc_config);
      err != ::EB_ErrorNone) {
    throw std::runtime_error(
        fmt::format("::svt_av1_enc_init_handle() failed: {}",
                    static_cast<std::uint32_t>(err)));
  }
  m_av1_enc_config.rate_control_mode = ::SVT_AV1_RC_MODE_CBR;
  m_av1_enc_config.target_bit_rate = m_bitrate * 1000;
  m_av1_enc_config.force_key_frames = false;
  m_av1_enc_config.source_width = m_width;
  m_av1_enc_config.source_height = m_height;
  m_av1_enc_config.frame_rate_numerator =
      static_cast<std::uint32_t>(m_fps.numerator());
  m_av1_enc_config.frame_rate_denominator =
      static_cast<std::uint32_t>(m_fps.denominator());

  if (auto err = ::svt_av1_enc_set_parameter(m_handle, &m_av1_enc_config);
      err != ::EB_ErrorNone) {
    throw std::runtime_error(
        fmt::format("::svt_av1_enc_set_parameter() failed: {}",
                    static_cast<std::uint32_t>(err)));
  }

  if (auto err = ::svt_av1_enc_init(m_handle); err != ::EB_ErrorNone) {
    throw std::runtime_error(fmt::format("svt_av1_enc_init() failed: {}",
                                         static_cast<std::uint32_t>(err)));
  }

  m_input_buffer = new ::EbBufferHeaderType();
  m_input_buffer->p_buffer =
      reinterpret_cast<std::uint8_t*>(::malloc(sizeof(::EbSvtIOFormat)));
  m_input_buffer->size = sizeof(::EbBufferHeaderType);
  m_input_buffer->p_app_private = nullptr;
  m_input_buffer->pic_type = ::EB_AV1_INVALID_PICTURE;
  m_input_buffer->metadata = nullptr;

  ::EbSvtIOFormat* buffer =
      reinterpret_cast<::EbSvtIOFormat*>(m_input_buffer->p_buffer);
  auto luma_size = m_width * m_height;
  buffer->luma = new std::uint8_t[luma_size];
  buffer->cb = new std::uint8_t[luma_size >> 2];
  buffer->cr = new std::uint8_t[luma_size >> 2];

  ::EbBufferHeaderType* stream_header = nullptr;
  if (auto err = ::svt_av1_enc_stream_header(m_handle, &stream_header);
      err != ::EB_ErrorNone) {
    throw std::runtime_error(
        fmt::format("svt_av1_enc_stream_header() failed: {}",
                    static_cast<std::uint32_t>(err)));
  }

  std::copy_n(stream_header->p_buffer, stream_header->n_filled_len,
              std::back_inserter(m_extra_data));

  ::svt_av1_enc_stream_header_release(stream_header);

  spdlog::debug("AV1 extra_data: [{:02x}]", fmt::join(m_extra_data, ", "));
}

void BufferAV1Encoder::outputImage(const std::vector<unsigned char>& yuv) {
  ::EbSvtIOFormat* buffer =
      reinterpret_cast<::EbSvtIOFormat*>(m_input_buffer->p_buffer);
  const auto luma_size = m_width * m_height;
  std::copy_n(std::begin(yuv), luma_size, buffer->luma);
  std::copy_n(std::begin(yuv) + luma_size, luma_size >> 2, buffer->cb);
  std::copy_n(std::begin(yuv) + luma_size + (luma_size >> 2), luma_size >> 2,
              buffer->cr);
  m_input_buffer->flags = 0;
  m_input_buffer->p_app_private = nullptr;
  m_input_buffer->pts = m_frame;
  m_input_buffer->pic_type = ::EB_AV1_INVALID_PICTURE;
  m_input_buffer->metadata = nullptr;
  buffer->y_stride = m_width;
  buffer->cb_stride = m_width >> 1;
  buffer->cr_stride = m_width >> 1;
  buffer->width = m_width;
  buffer->height = m_height;

  if (auto err = ::svt_av1_enc_send_picture(m_handle, m_input_buffer);
      err != ::EB_ErrorNone) {
    throw std::runtime_error(
        fmt::format("::svt_av1_enc_send_picture() failed: {}",
                    static_cast<std::uint32_t>(err)));
  }

  outputFrame(m_frame++, 0);
}

void BufferAV1Encoder::outputFrame(const std::int64_t frame_index,
                                   const std::uint8_t done_sending_pics) {
  ::EbBufferHeaderType* output_buf = nullptr;

  while (true) {
    auto status =
        ::svt_av1_enc_get_packet(m_handle, &output_buf, done_sending_pics);
    if (status == ::EB_ErrorMax) {
      throw std::runtime_error(
          fmt::format("::svt_av1_enc_send_picture() failed: {}",
                      static_cast<std::uint32_t>(status)));
    } else if (status == ::EB_NoErrorEmptyQueue) {
      return;
    }
    const std::uint64_t timestamp =
        static_cast<std::uint64_t>(output_buf->pts) * m_timescale *
        m_fps.denominator() / m_fps.numerator();
    std::uint8_t* data = new std::uint8_t[output_buf->n_filled_len];
    std::copy_n(output_buf->p_buffer, output_buf->n_filled_len, data);
    m_buffer->push(hisui::Frame{
        .timestamp = timestamp,
        .data = data,
        .data_size = output_buf->n_filled_len,
        .is_key = output_buf->pic_type == ::EB_AV1_KEY_PICTURE ||
                  output_buf->pic_type == ::EB_AV1_INTRA_ONLY_PICTURE});

    m_sum_of_bits += output_buf->n_filled_len * 8;

    ::svt_av1_enc_release_out_buffer(&output_buf);

    if (m_frame > 0 && m_frame % 100 == 0 && frame_index > 0) {
      spdlog::trace("AV1: frame index: {}", m_frame);
      spdlog::trace("AV1: average bitrate (kbps): {}",
                    m_sum_of_bits * m_fps.numerator() / m_fps.denominator() /
                        static_cast<std::uint64_t>(m_frame) / 1024);
    }
  }
}

void BufferAV1Encoder::flush() {
  ::EbBufferHeaderType input_buffer;
  input_buffer.n_alloc_len = 0;
  input_buffer.n_filled_len = 0;
  input_buffer.n_tick_count = 0;
  input_buffer.p_app_private = nullptr;
  input_buffer.flags = EB_BUFFERFLAG_EOS;
  input_buffer.p_buffer = nullptr;
  input_buffer.metadata = nullptr;

  if (auto err = ::svt_av1_enc_send_picture(m_handle, &input_buffer);
      err != ::EB_ErrorNone) {
    throw std::runtime_error(
        fmt::format("::svt_av1_enc_send_picture() failed: {}",
                    static_cast<std::uint32_t>(err)));
  }

  outputFrame(m_frame, 1);
}

BufferAV1Encoder::~BufferAV1Encoder() {
  if (m_frame > 0) {
    spdlog::debug("AV1Encoder: number of frames: {}", m_frame);
    spdlog::debug("AV1Encoder: final average bitrate (kbps): {}",
                  m_sum_of_bits * m_fps.numerator() / m_fps.denominator() /
                      static_cast<std::uint64_t>(m_frame) / 1024);
  }
  if (m_input_buffer) {
    if (m_input_buffer->p_buffer) {
      ::EbSvtIOFormat* buffer =
          reinterpret_cast<::EbSvtIOFormat*>(m_input_buffer->p_buffer);
      if (buffer->luma) {
        delete[] buffer->luma;
      }
      if (buffer->cb) {
        delete[] buffer->cb;
      }
      if (buffer->cr) {
        delete[] buffer->cr;
      }
      ::free(m_input_buffer->p_buffer);
    }
    delete m_input_buffer;
  }

  if (auto err = ::svt_av1_enc_deinit(m_handle); err != ::EB_ErrorNone) {
    spdlog::error("::svt_av1_enc_deinit() failed: {}",
                  static_cast<std::uint32_t>(err));
  }
  if (auto err = ::svt_av1_enc_deinit_handle(m_handle); err != ::EB_ErrorNone) {
    spdlog::error("::svt_av1_enc_deinit_handle() failed: {}",
                  static_cast<std::uint32_t>(err));
  }
}

std::uint32_t BufferAV1Encoder::getFourcc() const {
  return m_fourcc;
}

void BufferAV1Encoder::setResolutionAndBitrate(const std::uint32_t width,
                                               const std::uint32_t height,
                                               const std::uint32_t bitrate) {
  if (m_width == width && m_height == height && m_bitrate == bitrate) {
    return;
  }
  spdlog::debug("width: {}, height: {}", width, height);
  flush();
  m_width = width;
  m_height = height;
  m_bitrate = bitrate;

  m_av1_enc_config.target_bit_rate = m_bitrate;
  m_av1_enc_config.source_width = m_width;
  m_av1_enc_config.source_height = m_height;

  if (auto err = ::svt_av1_enc_set_parameter(m_handle, &m_av1_enc_config);
      err != ::EB_ErrorNone) {
    throw std::runtime_error(
        fmt::format("::svt_av1_enc_set_parameter() failed: {}",
                    static_cast<std::uint32_t>(err)));
  }

  ::EbSvtIOFormat* buffer =
      reinterpret_cast<::EbSvtIOFormat*>(m_input_buffer->p_buffer);
  if (buffer->luma) {
    delete[] buffer->luma;
  }
  if (buffer->cb) {
    delete[] buffer->cb;
  }
  if (buffer->cr) {
    delete[] buffer->cr;
  }
  auto luma_size = sizeof(std::uint8_t) * m_width * m_height;
  buffer->luma = new std::uint8_t[luma_size];
  buffer->cb = new std::uint8_t[luma_size >> 2];
  buffer->cr = new std::uint8_t[luma_size >> 2];
}

}  // namespace hisui::video
