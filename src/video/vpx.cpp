#include "video/vpx.hpp"

#include <bits/exception.h>
#include <fmt/core.h>
#include <spdlog/spdlog.h>
#include <vpx/vp8cx.h>
#include <vpx/vp8dx.h>
#include <vpx/vpx_codec.h>
#include <vpx/vpx_decoder.h>
#include <vpx/vpx_encoder.h>
#include <vpx/vpx_image.h>

#include <algorithm>
#include <array>
#include <cstddef>
#include <iterator>
#include <stdexcept>
#include <vector>

#include <boost/rational.hpp>

#include "config.hpp"
#include "constants.hpp"
#include "video/yuv.hpp"

namespace hisui::video {

VPXEncoderConfig::VPXEncoderConfig(const std::uint32_t t_width,
                                   const std::uint32_t t_height,
                                   const hisui::Config& config)
    : width(t_width),
      height(t_height),
      fps(config.out_video_frame_rate),
      fourcc(config.out_video_codec),
      bitrate(config.out_video_bit_rate),
      cq_level(config.libvpx_cq_level),
      min_q(config.libvpx_min_q),
      max_q(config.libvpx_max_q),
      threads(config.libvpx_threads),
      frame_parallel(config.libvp9_frame_parallel),
      cpu_used(config.libvpx_cpu_used),
      tile_columns(config.libvp9_tile_columns) {}

void update_yuv_image_by_vpx_image(YUVImage* yuv_image,
                                   const vpx_image_t* vpx_image) {
  const std::array<int, 3> PLANES_YUV = {VPX_PLANE_Y, VPX_PLANE_U, VPX_PLANE_V};

  const std::uint32_t new_width =
      get_vpx_image_plane_width(vpx_image, VPX_PLANE_Y);
  const std::uint32_t new_height =
      get_vpx_image_plane_height(vpx_image, VPX_PLANE_Y);

  yuv_image->setWidthAndHeight(new_width, new_height);

  for (std::size_t i = 0; i < 3; ++i) {
    const int plane = PLANES_YUV[i];
    const std::uint32_t w = get_vpx_image_plane_width(vpx_image, plane);
    const std::uint32_t h = get_vpx_image_plane_height(vpx_image, plane);
    const std::uint32_t s =
        static_cast<std::uint32_t>(vpx_image->stride[plane]);

    for (std::uint32_t y = 0; y < h; ++y) {
      std::copy_n(vpx_image->planes[plane] + y * s, w,
                  yuv_image->yuv[i] + y * w);
    }
  }
}

::vpx_codec_iface_t* get_vpx_decode_codec_iface_by_fourcc(
    const std::uint32_t fourcc) {
  switch (fourcc) {
    case hisui::Constants::VP8_FOURCC:
      return &::vpx_codec_vp8_dx_algo;
    case hisui::Constants::VP9_FOURCC:
      return &::vpx_codec_vp9_dx_algo;
  }
  return nullptr;
}

::vpx_codec_iface_t* get_vpx_encode_codec_iface_by_fourcc(
    const std::uint32_t fourcc) {
  switch (fourcc) {
    case hisui::Constants::VP8_FOURCC:
      return &::vpx_codec_vp8_cx_algo;
    case hisui::Constants::VP9_FOURCC:
      return &::vpx_codec_vp9_cx_algo;
  }
  return nullptr;
}

std::uint32_t get_vpx_image_plane_width(const ::vpx_image_t* img,
                                        const int plane) {
  if (plane > 0 && img->x_chroma_shift > 0)
    return (img->d_w + 1) >> img->x_chroma_shift;
  else
    return img->d_w;
}

std::uint32_t get_vpx_image_plane_height(const ::vpx_image_t* img,
                                         const int plane) {
  if (plane > 0 && img->y_chroma_shift > 0)
    return (img->d_h + 1) >> img->y_chroma_shift;
  else
    return img->d_h;
}

::vpx_image_t* create_black_vpx_image(const std::uint32_t width,
                                      const std::uint32_t height) {
  const std::array<int, 3> PLANES_YUV = {VPX_PLANE_Y, VPX_PLANE_U, VPX_PLANE_V};
  const auto img = ::vpx_img_alloc(nullptr, VPX_IMG_FMT_I420, width, height, 0);
  std::fill(img->planes[PLANES_YUV[0]],
            img->planes[PLANES_YUV[0]] + width * height, 0);
  for (std::size_t i = 1; i < 3; ++i) {
    const int plane = PLANES_YUV[i];
    const std::uint32_t w = get_vpx_image_plane_width(img, plane);
    const std::uint32_t h = get_vpx_image_plane_height(img, plane);
    std::fill(img->planes[plane], img->planes[plane] + w * h, 128);
  }
  return img;
}

void update_vpx_image_by_yuv_data(::vpx_image_t* img,
                                  const std::vector<std::uint8_t>& v) {
  auto base = 0;
  for (auto plane = 0; plane < 3; ++plane) {
    unsigned char* buf = img->planes[plane];
    const std::uint32_t stride = static_cast<std::uint32_t>(img->stride[plane]);
    const std::uint32_t w = get_vpx_image_plane_width(img, plane) *
                            ((img->fmt & VPX_IMG_FMT_HIGHBITDEPTH) ? 2 : 1);
    const std::uint32_t h = get_vpx_image_plane_height(img, plane);

    for (std::uint32_t y = 0; y < h; ++y) {
      std::copy_n(std::begin(v) + base + y * w, w, buf + y * stride);
    }
    base += h * w;
  }
}

void create_vpx_codec_ctx_t_for_encoding(::vpx_codec_ctx_t* codec,
                                         const VPXEncoderConfig& config) {
  const auto dx_algo = get_vpx_encode_codec_iface_by_fourcc(config.fourcc);
  if (!dx_algo) {
    throw std::runtime_error("get_vpx_encode_codec_iface_by_fourcc() failed");
  }

  ::vpx_codec_enc_cfg_t cfg;
  const auto ret = ::vpx_codec_enc_config_default(dx_algo, &cfg, 0);
  if (ret) {
    throw std::runtime_error(
        fmt::format("vpx_codec_enc_config_default() failed: error_code={}",
                    ::vpx_codec_err_to_string(ret)));
  }

  spdlog::debug("VPX fourcc={:x} codec_iface_name={}", config.fourcc,
                vpx_codec_iface_name(dx_algo));
  spdlog::debug("target_bitrate={} cq_level={} min_q={}, max_q={}",
                config.bitrate, config.cq_level, config.min_q, config.max_q);

  cfg.g_w = config.width;
  cfg.g_h = config.height;
  cfg.g_timebase.num = static_cast<int>(config.fps.denominator());
  cfg.g_timebase.den = static_cast<int>(config.fps.numerator());
  cfg.rc_target_bitrate = config.bitrate;
  cfg.rc_end_usage = VPX_CQ;
  cfg.rc_min_quantizer = config.min_q;
  cfg.rc_max_quantizer = config.max_q;

  if (config.threads > 0) {
    cfg.g_threads = config.threads;
  }

  if (::vpx_codec_enc_init(codec, dx_algo, &cfg, 0)) {
    throw std::runtime_error("vpx_codec_enc_init() failed");
  }

  ::vpx_codec_control(codec, VP8E_SET_CQ_LEVEL, config.cq_level);
  ::vpx_codec_control(codec, VP8E_SET_CPUUSED, config.cpu_used);
  if (config.fourcc == hisui::config::OutVideoCodec::VP9) {
    ::vpx_codec_control(codec, VP9E_SET_FRAME_PARALLEL_DECODING,
                        config.frame_parallel);
    ::vpx_codec_control(codec, VP9E_SET_TILE_COLUMNS,
                        static_cast<int>(config.tile_columns));
  }
}

void create_vpx_codec_ctx_t_for_decoding(::vpx_codec_ctx_t* codec,
                                         const std::uint32_t fourcc) {
  const auto dx_algo = get_vpx_decode_codec_iface_by_fourcc(fourcc);

  if (!dx_algo) {
    throw std::runtime_error("get_vpx_decode_codec_iface_by_fourcc() failed");
  }

  if (::vpx_codec_dec_init(codec, dx_algo, nullptr, 0)) {
    throw std::runtime_error("vpx_codec_dec_init() failed");
  }
}

}  // namespace hisui::video
