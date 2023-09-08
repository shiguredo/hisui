#pragma once

#include <vpl/mfxdefs.h>
#include <vpl/mfxvideo++.h>

#include <cstdint>
#include <memory>
#include <queue>
#include <vector>

#include <boost/rational.hpp>

#include "constants.hpp"
#include "video/encoder.hpp"
#include "video/vpl_session.hpp"

namespace hisui {

class Config;
struct Frame;

}  // namespace hisui

namespace hisui::video {

class VPLEncoderConfig {
 public:
  VPLEncoderConfig(const std::uint32_t,
                   const std::uint32_t,
                   const hisui::Config&);
  const std::uint32_t width;
  const std::uint32_t height;
  const boost::rational<std::uint64_t> fps;
  const std::uint32_t target_bit_rate;
  const std::uint32_t max_bit_rate;
};

class VPLEncoder : public Encoder {
 public:
  VPLEncoder(const std::uint32_t,
             std::queue<hisui::Frame>*,
             const VPLEncoderConfig&,
             const std::uint64_t t_timescale = hisui::Constants::NANO_SECOND);
  ~VPLEncoder();
  static bool isSupported(const std::uint32_t fourcc);

  void outputImage(const std::vector<unsigned char>&) override;
  void flush() override;
  std::uint32_t getFourcc() const override;
  void setResolutionAndBitrate(const std::uint32_t,
                               const std::uint32_t,
                               const std::uint32_t) override;

 private:
  std::uint32_t m_width;
  std::uint32_t m_height;
  std::uint32_t m_bitrate;
  std::uint32_t m_fourcc;
  std::queue<hisui::Frame>* m_buffer;
  const std::uint64_t m_timescale;
  int m_frame = 0;
  boost::rational<std::uint64_t> m_fps;
  std::uint64_t m_sum_of_bits = 0;
  std::vector<std::uint8_t> m_surface_buffer;
  std::vector<::mfxFrameSurface1> m_surfaces;

  std::unique_ptr<::MFXVideoENCODE> m_encoder;
  ::mfxU32 m_codec;
  ::mfxFrameAllocRequest m_alloc_request;
  std::vector<std::uint8_t> m_bitstream_buffer;
  ::mfxBitstream m_bitstream;
  ::mfxFrameInfo m_frame_info;

  void initVPL();
  void releaseVPL();
  void encodeFrame(const std::vector<unsigned char>&);

  static std::unique_ptr<MFXVideoENCODE> createEncoder(
      const ::mfxU32 codec,
      const std::uint32_t width,
      const std::uint32_t height,
      const boost::rational<std::uint64_t> frame_rate,
      const std::uint32_t target_bit_rate,
      const std::uint32_t max_bit_rate,
      const bool init);
};

}  // namespace hisui::video
