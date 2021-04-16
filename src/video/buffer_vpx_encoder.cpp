#include "video/buffer_vpx_encoder.hpp"

#include <bits/exception.h>
#include <fmt/core.h>
#include <spdlog/fmt/fmt.h>
#include <spdlog/spdlog.h>
#include <vpx/vpx_codec.h>
#include <vpx/vpx_encoder.h>
#include <vpx/vpx_image.h>

#include <algorithm>
#include <stdexcept>

#include <boost/rational.hpp>

#include "frame.hpp"
#include "video/vpx.hpp"

namespace hisui::video {

BufferVPXEncoder::BufferVPXEncoder(std::queue<hisui::Frame>* t_buffer,
                                   const VPXEncoderConfig& config,
                                   const std::uint64_t t_timescale)
    : m_buffer(t_buffer), m_timescale(t_timescale) {
  m_width = config.width;
  m_height = config.height;
  m_fps = config.fps;
  m_fourcc = config.fourcc;
  m_bitrate = config.bitrate;
  if (!::vpx_img_alloc(&m_raw_vpx_image, VPX_IMG_FMT_I420, m_width, m_height,
                       0)) {
    throw std::runtime_error("vpx_img_alloc() failed");
  }

  create_vpx_codec_ctx_t_for_encoding(&m_codec, &m_cfg, config);
}

void BufferVPXEncoder::outputImage(const std::vector<unsigned char>& yuv) {
  update_vpx_image_by_yuv_data(&m_raw_vpx_image, yuv);
  encodeFrame(&m_codec, &m_raw_vpx_image, m_frame++, 0);
}

void BufferVPXEncoder::flush() {
  while (encodeFrame(&m_codec, nullptr, -1, 0)) {
  }
}

BufferVPXEncoder::~BufferVPXEncoder() {
  if (m_frame > 0) {
    spdlog::debug("VPXEncoder: number of frames: {}", m_frame);
    spdlog::debug("VPXEncoder: final average bitrate (kbps): {}",
                  m_sum_of_bits * m_fps.numerator() / m_fps.denominator() /
                      static_cast<std::uint64_t>(m_frame) / 1024);
  }
  ::vpx_img_free(&m_raw_vpx_image);
  ::vpx_codec_destroy(&m_codec);
}

bool BufferVPXEncoder::encodeFrame(::vpx_codec_ctx_t* codec,
                                   ::vpx_image_t* img,
                                   const int frame_index,
                                   const int flags) {
  const ::vpx_codec_err_t ret =
      ::vpx_codec_encode(codec, img, frame_index, 1, flags, VPX_DL_REALTIME);
  if (ret != VPX_CODEC_OK) {
    throw std::runtime_error(fmt::format("Failed to encode frame: error='{}'",
                                         ::vpx_codec_err_to_string(ret)));
  }

  ::vpx_codec_iter_t iter = nullptr;
  const ::vpx_codec_cx_pkt_t* pkt = nullptr;
  bool got_pkts = false;
  while ((pkt = ::vpx_codec_get_cx_data(codec, &iter)) != nullptr) {
    got_pkts = true;

    if (pkt->kind == VPX_CODEC_CX_FRAME_PKT) {
      const std::uint64_t pts_ns =
          static_cast<std::uint64_t>(pkt->data.frame.pts) * m_timescale *
          m_fps.denominator() / m_fps.numerator();
      const std::uint8_t* buf = static_cast<std::uint8_t*>(pkt->data.frame.buf);
      std::uint8_t* data = new std::uint8_t[pkt->data.frame.sz];
      std::copy_n(buf, pkt->data.frame.sz, data);
      m_buffer->push(hisui::Frame{
          .timestamp = pts_ns,
          .data = data,
          .data_size = pkt->data.frame.sz,
          .is_key = (pkt->data.frame.flags & VPX_FRAME_IS_KEY) != 0});

      m_sum_of_bits += pkt->data.frame.sz * 8;

      if (m_frame > 0 && m_frame % 100 == 0 && frame_index > 0) {
        spdlog::trace("VPXEncoder: frame index: {}", m_frame);
        spdlog::trace("VPXEncoder: average bitrate (kbps): {}",
                      m_sum_of_bits * m_fps.numerator() / m_fps.denominator() /
                          static_cast<std::uint64_t>(m_frame) / 1024);
      }
    }
  }

  return got_pkts;
}

std::uint32_t BufferVPXEncoder::getFourcc() const {
  return m_fourcc;
}

void BufferVPXEncoder::setResolutionAndBitrate(const std::uint32_t width,
                                               const std::uint32_t height,
                                               const std::uint32_t bitrate) {
  if (m_width == width && m_height == height && m_bitrate == bitrate) {
    return;
  }
  flush();
  spdlog::debug("width: {}, height: {}", width, height);
  m_width = width;
  m_height = height;
  m_cfg.g_w = width;
  m_cfg.g_h = height;
  m_cfg.rc_target_bitrate = bitrate;
  m_cfg.g_lag_in_frames = 0;
  auto res = ::vpx_codec_enc_config_set(&m_codec, &m_cfg);
  if (res != VPX_CODEC_OK) {
    throw std::runtime_error(
        fmt::format("vpx_codec_enc_config_set() failed: {}",
                    ::vpx_codec_err_to_string(res)));
  }

  ::vpx_img_free(&m_raw_vpx_image);
  if (!::vpx_img_alloc(&m_raw_vpx_image, VPX_IMG_FMT_I420, m_width, m_height,
                       0)) {
    throw std::runtime_error("vpx_img_alloc() failed");
  }
}

}  // namespace hisui::video

