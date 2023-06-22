#pragma once

#include <EbSvtAv1Enc.h>

#include <vpx/vpx_codec.h>
#include <vpx/vpx_encoder.h>
#include <vpx/vpx_image.h>

#include <cstdint>
#include <queue>
#include <vector>

#include <boost/cstdint.hpp>
#include <boost/rational.hpp>

#include "constants.hpp"
#include "video/encoder.hpp"

namespace hisui {

class Config;
struct Frame;

}  // namespace hisui

namespace hisui::video {

class AV1EncoderConfig {
 public:
  AV1EncoderConfig(const std::uint32_t,
                   const std::uint32_t,
                   const hisui::Config&);
  const std::uint32_t width;
  const std::uint32_t height;
  const boost::rational<std::uint64_t> fps;
  const std::uint32_t fourcc;
  const std::uint32_t bitrate;
};

class BufferAV1Encoder : public Encoder {
 public:
  BufferAV1Encoder(
      std::queue<hisui::Frame>*,
      const AV1EncoderConfig& config,
      const std::uint64_t timescale = hisui::Constants::NANO_SECOND);
  ~BufferAV1Encoder();

  void outputImage(const std::vector<unsigned char>&);
  void flush();
  std::uint32_t getFourcc() const;
  void setResolutionAndBitrate(const std::uint32_t,
                               const std::uint32_t,
                               const std::uint32_t);

 private:
  std::queue<hisui::Frame>* m_buffer;
  std::uint32_t m_width;
  std::uint32_t m_height;
  std::uint32_t m_bitrate;
  boost::rational<std::uint64_t> m_fps;
  std::uint32_t m_fourcc;
  std::int64_t m_frame = 0;
  std::uint64_t m_sum_of_bits = 0;
  const std::uint64_t m_timescale;
  ::EbComponentType* m_handle;
  ::EbBufferHeaderType* m_input_buffer;
  ::EbSvtAv1EncConfiguration m_av1_enc_config;
  std::vector<std::uint8_t> m_extra_data = {};

  void outputFrame(const std::int64_t, const std::uint8_t);
};

}  // namespace hisui::video
