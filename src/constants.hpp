#pragma once

#include <cstdint>
#include <string>

namespace hisui {

class Constants {
 public:
  static const std::uint32_t PCM_SAMPLE_RATE = 48000;
  static const std::uint32_t VP8_FOURCC = 0x30385056;
  static const std::uint32_t VP9_FOURCC = 0x30395056;
  static const std::uint32_t AV1_FOURCC = 0x31305641;
  static const std::uint32_t H264_FOURCC = 0x34363248;
  static const std::uint32_t I420_FOURCC = 0x30323449;
  static const std::uint32_t VIDEO_H264_OUTPUT_BITRATE = 512000;
  static const std::uint32_t VIDEO_VPX_BIT_RATE_PER_FILE = 200;
  static const std::uint64_t NANO_SECOND = 1000000000;
  static const std::uint32_t FDK_AAC_DEFAULT_BIT_RATE = 64000;
  static const std::uint64_t FDK_AAC_ENCODE_BUFFER_SIZE = 20480;
  static const std::uint64_t LAME_MP3_BUFFER_SIZE = 8192;
  static const std::uint32_t OPUS_DEFAULT_BIT_RATE = 64 * 1024;
  static const std::uint64_t OPUS_ENCODE_FRAME_SIZE = 960;
  static const std::uint64_t OPUS_DECODE_MAX_FRAME_SIZE = 5760;
  static const std::uint64_t OPUS_MAX_PACKET_SIZE = 1276;
  inline static const std::string HISUI_APPLICATION_NAME = "hisui";
};

}  // namespace hisui
