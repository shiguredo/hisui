#pragma once

#include <codec/api/svc/codec_app_def.h>

#include <queue>
#include <vector>

#include <boost/cstdint.hpp>
#include <boost/rational.hpp>

#include "constants.hpp"
#include "video/encoder.hpp"
#include "video/openh264.hpp"

class ISVCEncoder;

namespace hisui {

struct Frame;

}

namespace hisui::video {

class BufferOpenH264Encoder : public Encoder {
 public:
  BufferOpenH264Encoder(
      std::queue<hisui::Frame>*,
      const OpenH264EncoderConfig&,
      const std::uint64_t timescale = hisui::Constants::NANO_SECOND);
  ~BufferOpenH264Encoder();

  void outputImage(const std::vector<unsigned char>&);
  void flush();
  std::uint32_t getFourcc() const;
  void setResolutionAndBitrate(const std::uint32_t,
                               const std::uint32_t,
                               const std::uint32_t);

 private:
  ::ISVCEncoder* m_encoder = nullptr;
  std::queue<hisui::Frame>* m_buffer;
  std::uint32_t m_width;
  std::uint32_t m_height;
  std::uint32_t m_bitrate;
  boost::rational<std::uint64_t> m_fps;
  int m_frame = 0;
  std::uint64_t m_sum_of_bits = 0;
  const std::uint64_t m_timescale;
  ::SSourcePicture m_pic = {};

  bool encodeFrame();
};

}  // namespace hisui::video
