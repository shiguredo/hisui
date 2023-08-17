#pragma once

#include <vpl/mfxdefs.h>
#include <vpl/mfxvideo++.h>

#include <cstdint>
#include <memory>
#include <vector>

#include "constants.hpp"
#include "video/decoder.hpp"
#include "video/vpl_session.hpp"

namespace hisui::webm::input {

class VideoContext;

}

namespace hisui::video {

class YUVImage;

class VPLDecoder : public Decoder {
 public:
  explicit VPLDecoder(std::shared_ptr<hisui::webm::input::VideoContext>);
  ~VPLDecoder();

  const std::shared_ptr<YUVImage> getImage(const std::uint64_t) override;

  static bool isSupported(const std::uint32_t fourcc);

 private:
  std::unique_ptr<MFXVideoDECODE> m_decoder;
  std::uint32_t m_fourcc;
  std::uint64_t m_current_timestamp = 0;
  std::uint64_t m_next_timestamp = 0;
  std::shared_ptr<YUVImage> m_current_yuv_image = nullptr;
  std::shared_ptr<YUVImage> m_next_yuv_image = nullptr;
  bool m_report_enabled = false;
  std::vector<::mfxFrameSurface1> m_surfaces;
  ::mfxFrameAllocRequest m_alloc_request;
  std::vector<uint8_t> m_surface_buffer;
  std::vector<uint8_t> m_bitstream_buffer;
  ::mfxBitstream m_bitstream;

  static std::unique_ptr<::MFXVideoDECODE> createDecoder(
      const ::mfxU32 codec,
      const std::vector<std::pair<std::uint32_t, std::uint32_t>> sizes);

  static std::unique_ptr<::MFXVideoDECODE> createDecoderInternal(
      VPLSession& session,
      const ::mfxU32 codec,
      const std::uint32_t width,
      const std::uint32_t height);

  bool initVpl();
  void releaseVpl();
  void updateImage(const std::uint64_t);
  void updateImageByTimestamp(const std::uint64_t);
  void decode();
};

}  // namespace hisui::video
