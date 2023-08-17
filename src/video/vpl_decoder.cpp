#include "video/vpl_decoder.hpp"

#include <fmt/core.h>
#include <libyuv/convert.h>
#include <spdlog/spdlog.h>
#include <vpl/mfxdefs.h>
#include <vpl/mfxstructures.h>
#include <vpl/mfxvp8.h>

#include <cstdint>
#include <iostream>
#include <stdexcept>

#include "report/reporter.hpp"
#include "video/vpl.hpp"
#include "video/vpl_session.hpp"
#include "video/yuv.hpp"
#include "webm/input/video_context.hpp"

namespace hisui::video {

bool VPLDecoder::initVpl() {
  m_decoder = createDecoder(m_fourcc, {{4096, 4096}, {2048, 2048}});
  if (!m_decoder) {
    throw std::runtime_error(
        fmt::format("createDecoder() failed: fourcc={}", m_fourcc));
  }

  ::mfxStatus sts = MFX_ERR_NONE;

  ::mfxVideoParam param;
  memset(&param, 0, sizeof(param));
  sts = m_decoder->GetVideoParam(&param);
  if (sts != MFX_ERR_NONE) {
    return false;
  }

  memset(&m_alloc_request, 0, sizeof(m_alloc_request));
  sts = m_decoder->QueryIOSurf(&param, &m_alloc_request);
  if (sts != MFX_ERR_NONE) {
    throw std::runtime_error(fmt::format("QueryIOSurf() failed: sts={}",
                                         static_cast<std::int32_t>(sts)));
  }

  spdlog::debug("Decoder NumFrameSuggested={}",
                m_alloc_request.NumFrameSuggested);

  // 入力ビットストリーム
  m_bitstream_buffer.resize(1024 * 1024);
  memset(&m_bitstream, 0, sizeof(m_bitstream));
  m_bitstream.MaxLength = static_cast<std::uint32_t>(m_bitstream_buffer.size());
  m_bitstream.Data = m_bitstream_buffer.data();

  // 入力ビットストリーム
  // 必要な枚数分の出力サーフェスを作る
  {
    auto width = (m_alloc_request.Info.Width + 31) / 32 * 32;
    auto height = (m_alloc_request.Info.Height + 31) / 32 * 32;
    // 1枚あたりのバイト数
    // NV12 なので 1 ピクセルあたり 12 ビット
    auto size = width * height * 12 / 8;
    m_surface_buffer.resize(
        static_cast<std::uint64_t>(m_alloc_request.NumFrameSuggested * size));

    m_surfaces.clear();
    m_surfaces.reserve(m_alloc_request.NumFrameSuggested);
    for (int i = 0; i < m_alloc_request.NumFrameSuggested; i++) {
      ::mfxFrameSurface1 surface;
      memset(&surface, 0, sizeof(surface));
      surface.Info = param.mfx.FrameInfo;
      surface.Data.Y = m_surface_buffer.data() + i * size;
      surface.Data.U = m_surface_buffer.data() + i * size + width * height;
      surface.Data.V = m_surface_buffer.data() + i * size + width * height + 1;
      surface.Data.Pitch = static_cast<std::uint16_t>(width);
      m_surfaces.push_back(surface);
    }
  }

  return true;
}

void VPLDecoder::releaseVpl() {
  if (m_decoder != nullptr) {
    m_decoder->Close();
  }
}

VPLDecoder::VPLDecoder(std::shared_ptr<hisui::webm::input::VideoContext> t_webm)
    : Decoder(t_webm), m_fourcc(t_webm->getFourcc()) {
  initVpl();

  m_current_yuv_image =
      std::shared_ptr<YUVImage>(create_black_yuv_image(m_width, m_height));
  m_next_yuv_image =
      std::shared_ptr<YUVImage>(create_black_yuv_image(m_width, m_height));

  if (hisui::report::Reporter::hasInstance()) {
    m_report_enabled = true;

    hisui::report::Reporter::getInstance().registerVideoDecoder(
        m_webm->getFilePath(),
        {.codec = "H.264", .duration = m_webm->getDuration()});

    hisui::report::Reporter::getInstance().registerResolutionChange(
        m_webm->getFilePath(),
        {.timestamp = 0, .width = m_width, .height = m_height});
  }
}

VPLDecoder::~VPLDecoder() {
  releaseVpl();
}

const std::shared_ptr<YUVImage> VPLDecoder::getImage(
    const std::uint64_t timestamp) {
  if (!m_webm || m_is_time_over) {
    return m_black_yuv_image;
  }
  // 時間超過した
  if (m_duration <= timestamp) {
    m_is_time_over = true;
    return m_black_yuv_image;
  }
  updateImage(timestamp);
  return m_current_yuv_image;
}

void VPLDecoder::updateImage(const std::uint64_t timestamp) {
  // 次のブロックに逹っしていない
  if (timestamp < m_next_timestamp) {
    return;
  }
  // 次以降のブロックに逹っした
  updateImageByTimestamp(timestamp);
}

void VPLDecoder::updateImageByTimestamp(const std::uint64_t timestamp) {
  if (m_finished_webm) {
    return;
  }

  do {
    if (m_report_enabled) {
      if (m_current_yuv_image->getWidth(0) != m_next_yuv_image->getWidth(0) ||
          m_current_yuv_image->getHeight(0) != m_next_yuv_image->getHeight(0)) {
        hisui::report::Reporter::getInstance().registerResolutionChange(
            m_webm->getFilePath(), {.timestamp = m_next_timestamp,
                                    .width = m_next_yuv_image->getWidth(0),
                                    .height = m_next_yuv_image->getHeight(0)});
      }
    }
    m_current_yuv_image = m_next_yuv_image;
    m_current_timestamp = m_next_timestamp;
    if (m_webm->readFrame()) {
      decode();
      m_next_timestamp = static_cast<std::uint64_t>(m_webm->getTimestamp());
    } else {
      // m_duration までは m_current_image を出すので webm を読み終えても m_current_image を維持する
      m_finished_webm = true;
      m_next_timestamp = std::numeric_limits<std::uint64_t>::max();
      return;
    }
  } while (timestamp >= m_next_timestamp);
}

void VPLDecoder::decode() {
  auto buffer_size = m_webm->getBufferSize();

  if (m_bitstream.MaxLength < m_bitstream.DataLength + buffer_size) {
    m_bitstream_buffer.resize(m_bitstream.DataLength + buffer_size);
    m_bitstream.MaxLength = static_cast<std::uint32_t>(
        m_bitstream.DataLength + m_bitstream_buffer.size());
    m_bitstream.Data = m_bitstream_buffer.data();
  }

  memmove(m_bitstream.Data, m_bitstream.Data + m_bitstream.DataOffset,
          m_bitstream.DataLength);
  m_bitstream.DataOffset = 0;
  memcpy(m_bitstream.Data + m_bitstream.DataLength, m_webm->getBuffer(),
         buffer_size);
  m_bitstream.DataLength += buffer_size;

  auto surface =
      std::find_if(std::begin(m_surfaces), std::end(m_surfaces),
                   [](const mfxFrameSurface1& s) { return !s.Data.Locked; });
  if (surface == std::end(m_surfaces)) {
    throw std::runtime_error("unlocked surface is not found");
  }
  ::mfxStatus sts;
  ::mfxSyncPoint syncp;
  ::mfxFrameSurface1* out_surface = nullptr;
  while (true) {
    sts = m_decoder->DecodeFrameAsync(&m_bitstream, &*surface, &out_surface,
                                      &syncp);
    if (sts == MFX_WRN_DEVICE_BUSY) {
      std::this_thread::sleep_for(std::chrono::milliseconds(1));
      continue;
    }
    // 受信した映像のサイズが変わってたら width, height を更新する
    if (sts == MFX_WRN_VIDEO_PARAM_CHANGED) {
      mfxVideoParam param;
      memset(&param, 0, sizeof(param));
      sts = m_decoder->GetVideoParam(&param);
      if (sts != MFX_ERR_NONE) {
        throw std::runtime_error(fmt::format("GetVideoParam() failed: sts={}",
                                             static_cast<std::int32_t>(sts)));
      }

      if (m_width != param.mfx.FrameInfo.CropW ||
          m_height != param.mfx.FrameInfo.CropH) {
        m_width = param.mfx.FrameInfo.CropW;
        m_height = param.mfx.FrameInfo.CropH;
      }
      continue;
    }
    break;
  }
  if (sts == MFX_ERR_MORE_DATA) {
    // もっと入力が必要なので出直す
    return;
  }

  if (!syncp) {
    spdlog::info("Failed to DecodeFrameAsync: syncp is null, sts={}",
                 static_cast<std::int32_t>(sts));
    return;
  }
  if (sts != MFX_ERR_NONE) {
    throw std::runtime_error(fmt::format("DecodeFrameAsync() failed: sts={}",
                                         static_cast<std::int32_t>(sts)));
  }

  // H264 は sts == MFX_WRN_VIDEO_PARAM_CHANGED でハンドリングできるのでここではチェックしない
  // VP9 は受信フレームのサイズが変わっても MFX_WRN_VIDEO_PARAM_CHANGED を返さないようなので、
  // ここで毎フレーム情報を取得してサイズを更新する。
  if (m_fourcc != Constants::H264_FOURCC) {
    ::mfxVideoParam param;
    memset(&param, 0, sizeof(param));
    sts = m_decoder->GetVideoParam(&param);
    if (sts != MFX_ERR_NONE) {
      throw std::runtime_error(fmt::format("GetVideoParam() failed: sts={}",
                                           static_cast<std::int32_t>(sts)));
    }

    if (m_width != param.mfx.FrameInfo.CropW ||
        m_height != param.mfx.FrameInfo.CropH) {
      m_width = param.mfx.FrameInfo.CropW;
      m_height = param.mfx.FrameInfo.CropH;
    }
  }

  sts = ::MFXVideoCORE_SyncOperation(
      hisui::video::VPLSession::getInstance().getSession(), syncp, 600000);
  if (sts != MFX_ERR_NONE) {
    throw std::runtime_error(
        fmt::format("MFXVideoCORE_SyncOperation() failed: sts={}",
                    static_cast<std::int32_t>(sts)));
  }

  m_next_yuv_image = std::make_shared<YUVImage>(m_width, m_height);
  // NV12 から I420 に変換
  libyuv::NV12ToI420(out_surface->Data.Y, out_surface->Data.Pitch,
                     out_surface->Data.UV, out_surface->Data.Pitch,
                     m_next_yuv_image->yuv[0], static_cast<int>(m_width),
                     m_next_yuv_image->yuv[1], (m_width + 1) >> 1,
                     m_next_yuv_image->yuv[2], (m_width + 1) >> 1,
                     static_cast<int>(m_width), static_cast<int>(m_height));

  return;
}

std::unique_ptr<::MFXVideoDECODE> VPLDecoder::createDecoder(
    const std::uint32_t fourcc,
    const std::vector<std::pair<std::uint32_t, std::uint32_t>> sizes) {
  if (!hisui::video::VPLSession::hasInstance()) {
    throw std::runtime_error("VPL session is not opened");
  }
  for (auto size : sizes) {
    auto decoder =
        createDecoderInternal(hisui::video::VPLSession::getInstance(),
                              ToMfxCodec(fourcc), size.first, size.second);
    if (decoder) {
      return decoder;
    }
  }
  return nullptr;
}

std::unique_ptr<::MFXVideoDECODE> VPLDecoder::createDecoderInternal(
    VPLSession& session,
    ::mfxU32 codec,
    std::uint32_t width,
    std::uint32_t height) {
  std::unique_ptr<MFXVideoDECODE> decoder(
      new MFXVideoDECODE(session.getSession()));

  ::mfxStatus sts = MFX_ERR_NONE;

  ::mfxVideoParam param;
  memset(&param, 0, sizeof(param));

  param.mfx.CodecId = codec;
  param.mfx.FrameInfo.FourCC = MFX_FOURCC_NV12;
  param.mfx.FrameInfo.ChromaFormat = MFX_CHROMAFORMAT_YUV420;
  param.mfx.FrameInfo.PicStruct = MFX_PICSTRUCT_PROGRESSIVE;
  param.mfx.FrameInfo.CropX = 0;
  param.mfx.FrameInfo.CropY = 0;
  param.mfx.FrameInfo.CropW = static_cast<std::uint16_t>(width);
  param.mfx.FrameInfo.CropH = static_cast<std::uint16_t>(height);
  param.mfx.FrameInfo.Width =
      (static_cast<std::uint16_t>(width) + 15) / 16 * 16;
  param.mfx.FrameInfo.Height =
      (static_cast<std::uint16_t>(height) + 15) / 16 * 16;

  param.mfx.GopRefDist = 1;
  param.AsyncDepth = 1;
  param.IOPattern = MFX_IOPATTERN_OUT_SYSTEM_MEMORY;

  //qmfxExtCodingOption ext_coding_option;
  //qmemset(&ext_coding_option, 0, sizeof(ext_coding_option));
  //qext_coding_option.Header.BufferId = MFX_EXTBUFF_CODING_OPTION;
  //qext_coding_option.Header.BufferSz = sizeof(ext_coding_option);
  //qext_coding_option.MaxDecFrameBuffering = 1;

  //qmfxExtBuffer* ext_buffers[1];
  //qext_buffers[0] = (mfxExtBuffer*)&ext_coding_option;
  //qparam.ExtParam = ext_buffers;
  //qparam.NumExtParam = sizeof(ext_buffers) / sizeof(ext_buffers[0]);

  sts = decoder->Query(&param, &param);
  if (sts < 0) {
    const char* codec_str = codec == MFX_CODEC_VP8   ? "MFX_CODEC_VP8"
                            : codec == MFX_CODEC_VP9 ? "MFX_CODEC_VP9"
                            : codec == MFX_CODEC_AV1 ? "MFX_CODEC_AV1"
                            : codec == MFX_CODEC_AVC ? "MFX_CODEC_AVC"
                                                     : "MFX_CODEC_UNKNOWN";
    spdlog::warn("Unsupported decoder codec: codec={}, sts={}", codec_str,
                 static_cast<std::int32_t>(sts));
    return nullptr;
  }

  //if (sts != MFX_ERR_NONE) {
  //  RTC_LOG(LS_WARNING) << "Supported specified codec but has warning: sts="
  //                      << sts;
  //}

  // Query した上で Init しても MFX_ERR_UNSUPPORTED になることがあるので
  // 本来 Init が不要な時も常に呼ぶようにして確認する
  /*if (init)*/ {
    // Initialize the oneVPL encoder
    sts = decoder->Init(&param);
    if (sts != MFX_ERR_NONE) {
      const char* codec_str = codec == MFX_CODEC_VP8   ? "MFX_CODEC_VP8"
                              : codec == MFX_CODEC_VP9 ? "MFX_CODEC_VP9"
                              : codec == MFX_CODEC_AV1 ? "MFX_CODEC_AV1"
                              : codec == MFX_CODEC_AVC ? "MFX_CODEC_AVC"
                                                       : "MFX_CODEC_UNKNOWN";
      spdlog::warn("decoder->Init() failed: codec={}, std={}", codec_str,
                   static_cast<std::int32_t>(sts));
      return nullptr;
    }
  }

  return decoder;
}

bool VPLDecoder::isSupported(const std::uint32_t fourcc) {
  auto decoder = createDecoder(fourcc, {{4096, 4096}, {2048, 2048}});

  return decoder != nullptr;
}

}  // namespace hisui::video
