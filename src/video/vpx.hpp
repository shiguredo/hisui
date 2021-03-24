#pragma once

#include <vpx/vpx_codec.h>
#include <vpx/vpx_image.h>

#include <cstdint>
#include <vector>

#include <boost/cstdint.hpp>
#include <boost/rational.hpp>

namespace hisui {

class Config;

}

namespace hisui::video {

class YUVImage;

class VPXEncoderConfig {
 public:
  VPXEncoderConfig(const std::uint32_t,
                   const std::uint32_t,
                   const hisui::Config&);
  const std::uint32_t width;
  const std::uint32_t height;
  const boost::rational<std::uint64_t> fps;
  const std::uint32_t fourcc;
  const std::uint32_t bitrate;
  const std::uint32_t cq_level;
  const std::uint32_t min_q;
  const std::uint32_t max_q;
  const std::uint32_t threads;
  const std::uint32_t frame_parallel;
  const std::int32_t cpu_used;
  const std::uint32_t tile_columns;
};

void update_yuv_image_by_vpx_image(YUVImage*, const ::vpx_image_t*);

vpx_codec_iface_t* get_vpx_decode_codec_iface_by_fourcc(const std::uint32_t);
vpx_codec_iface_t* get_vpx_encode_codec_iface_by_fourcc(const std::uint32_t);

std::uint32_t get_vpx_image_plane_width(const ::vpx_image_t*, const int);
std::uint32_t get_vpx_image_plane_height(const ::vpx_image_t*, const int);

::vpx_image_t* create_black_vpx_image(const std::uint32_t, const std::uint32_t);

void update_vpx_image_by_yuv_data(::vpx_image_t*,
                                  const std::vector<std::uint8_t>&);

void create_vpx_codec_ctx_t_for_encoding(::vpx_codec_ctx_t*,
                                         const VPXEncoderConfig&);

void create_vpx_codec_ctx_t_for_decoding(::vpx_codec_ctx_t*,
                                         const std::uint32_t);

}  // namespace hisui::video
