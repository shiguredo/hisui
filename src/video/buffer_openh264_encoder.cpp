#include "video/buffer_openh264_encoder.hpp"

#include <codec/api/svc/codec_api.h>
#include <codec/api/svc/codec_def.h>

#include <bits/exception.h>
#include <fmt/core.h>
#include <spdlog/fmt/fmt.h>
#include <spdlog/spdlog.h>
#include <algorithm>
#include <stdexcept>

#include <boost/rational.hpp>

#include "constants.hpp"
#include "frame.hpp"
#include "video/openh264.hpp"
#include "video/openh264_handler.hpp"

namespace hisui::video {

class OpenH264EncoderConfig;

BufferOpenH264Encoder::BufferOpenH264Encoder(
    std::queue<hisui::Frame>* t_buffer,
    const OpenH264EncoderConfig& config,
    const std::uint64_t t_timescale)
    : m_buffer(t_buffer), m_timescale(t_timescale) {
  m_width = config.width;
  m_height = config.height;
  m_fps = config.fps;
  m_bitrate = config.bitrate;
  const auto create_encoder_ret =
      OpenH264Handler::getInstance().createEncoder(&m_encoder);
  if (create_encoder_ret != 0 || m_encoder == nullptr) {
    throw std::runtime_error(fmt::format(
        "OpenH264 createEncoder() failed: error_code={}", create_encoder_ret));
  }

  SEncParamExt param;
  if (const auto ret = m_encoder->GetDefaultParams(&param)) {
    throw std::runtime_error(
        fmt::format("OpenH264 GetDefaultParams() failed: error_code={}", ret));
  }
  param.iUsageType = CAMERA_VIDEO_REAL_TIME;  // TODO(haruyama): config にする?
  param.fMaxFrameRate = static_cast<float>(m_fps.numerator()) /
                        static_cast<float>(m_fps.denominator());
  param.iPicWidth = static_cast<int>(m_width);
  param.iPicHeight = static_cast<int>(m_height);
  param.iTargetBitrate = 1000 * static_cast<int>(m_bitrate);
  if (const auto ret = m_encoder->InitializeExt(&param)) {
    throw std::runtime_error(fmt::format(
        "OpenH264 Encoder Initialize() failed: error_code={}", ret));
  }

  auto videoFormat = videoFormatI420;
  if (const auto ret =
          m_encoder->SetOption(ENCODER_OPTION_DATAFORMAT, &videoFormat)) {
    throw std::runtime_error(
        fmt::format("OpenH264 SetOption(ENCODER_OPTION_DATAFORMAT) failed: "
                    "error_code={}",
                    ret));
  }

  m_pic.iPicWidth = static_cast<int>(m_width);
  m_pic.iPicHeight = static_cast<int>(m_height);
  m_pic.iColorFormat = videoFormatI420;
  m_pic.iStride[0] = m_pic.iPicWidth;
  m_pic.iStride[1] = m_pic.iStride[2] = m_pic.iPicWidth >> 1;
}

void BufferOpenH264Encoder::outputImage(const std::vector<unsigned char>& yuv) {
  auto data_size = m_width * m_height * 3 >> 1;
  std::uint8_t* data = new std::uint8_t[data_size];
  std::copy_n(std::begin(yuv), data_size, data);

  m_pic.pData[0] = data;
  m_pic.pData[1] = data + m_width * m_height;
  m_pic.pData[2] = data + m_width * m_height + ((m_width * m_height) >> 2);
  encodeFrame();
  delete[] data;
  ++m_frame;
}

void BufferOpenH264Encoder::flush() {}

BufferOpenH264Encoder::~BufferOpenH264Encoder() {
  if (m_frame > 0) {
    spdlog::debug("OpenH264Encoder: number of frames: {}", m_frame);
    spdlog::debug("OpenH264Encoder: final average bitrate (kbps): {}",
                  m_sum_of_bits * m_fps.numerator() / m_fps.denominator() /
                      static_cast<std::uint64_t>(m_frame) / 1024);
  }
  if (m_encoder) {
    m_encoder->Uninitialize();
    OpenH264Handler::getInstance().destroyEncoder(m_encoder);
  }
}

bool BufferOpenH264Encoder::encodeFrame() {
  const std::uint64_t pts_ns = static_cast<std::uint64_t>(m_frame) *
                               m_timescale * m_fps.denominator() /
                               m_fps.numerator();

  ::SFrameBSInfo info = {};

  if (const auto ret = m_encoder->EncodeFrame(&m_pic, &info)) {
    throw std::runtime_error(
        fmt::format("OpenH264 EncodeFrame() failed: error_code={}", ret));
  }
  if (info.eFrameType == videoFrameTypeSkip) {
    return false;
  }

  std::vector<std::uint8_t> layer_data;
  for (auto layer = 0; layer < info.iLayerNum; ++layer) {
    std::size_t data_size = 0;
    const auto& layer_bs_info = info.sLayerInfo[layer];

    auto nal_index = layer_bs_info.iNalCount - 1;
    do {
      data_size +=
          static_cast<std::size_t>(layer_bs_info.pNalLengthInByte[nal_index]);
      --nal_index;
    } while (nal_index >= 0);

    std::copy_n(&layer_bs_info.pBsBuf[layer], data_size,
                back_inserter(layer_data));
  }

  auto data_size = layer_data.size();
  std::uint8_t* data = new std::uint8_t[data_size];
  std::copy_n(std::begin(layer_data), data_size, data);
  m_buffer->push(hisui::Frame{.timestamp = pts_ns,
                              .data = data,
                              .data_size = data_size,
                              .is_key = info.eFrameType == videoFrameTypeIDR});

  m_sum_of_bits += data_size * 8;

  if (m_frame > 0 && m_frame % 100 == 0) {
    spdlog::trace("OpenH264Encoder: frame index: {}", m_frame);
    spdlog::trace("OpenH264Encoder: average bitrate (kbps): {}",
                  m_sum_of_bits * m_fps.numerator() / m_fps.denominator() /
                      static_cast<std::uint64_t>(m_frame) / 1024);
  }

  return true;
}  // namespace hisui::video

std::uint32_t BufferOpenH264Encoder::getFourcc() const {
  return hisui::Constants::H264_FOURCC;
}

void BufferOpenH264Encoder::setResolutionAndBitrate(const std::uint32_t,
                                                    const std::uint32_t,
                                                    const std::uint32_t) {
  throw std::runtime_error(
      "BufferOpenH264Encoder::setResolutionAndBitrate is not implemented");
}

}  // namespace hisui::video
