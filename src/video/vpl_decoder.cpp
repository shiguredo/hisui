#include "video/vpl_decoder.hpp"

#include <fmt/core.h>
#include <vpl/mfxdefs.h>
#include <vpl/mfxstructures.h>
#include <vpl/mfxvp8.h>

#include <cstdint>
#include <iostream>
#include <stdexcept>

#include "video/vpl_session.hpp"

namespace hisui::video {

mfxU32 ToMfxCodec(const std::uint32_t fourcc) {
  switch (fourcc) {
    case hisui::Constants::VP8_FOURCC:
      return MFX_CODEC_VP8;
    case hisui::Constants::VP9_FOURCC:
      return MFX_CODEC_VP9;
    case hisui::Constants::H264_FOURCC:
      return MFX_CODEC_AVC;
    case hisui::Constants::AV1_FOURCC:
      return MFX_CODEC_AV1;
    default:
      throw std::runtime_error(fmt::format("unknown fourcc: {:x}", fourcc));
  }
}

std::unique_ptr<MFXVideoDECODE> VplDecoder::CreateDecoder(
    std::shared_ptr<VplSession> session,
    std::uint32_t fourcc,
    std::vector<std::pair<int, int>> sizes) {
  for (auto size : sizes) {
    auto decoder = CreateDecoderInternal(session, ToMfxCodec(fourcc),
                                         size.first, size.second);
    if (decoder != nullptr) {
      return decoder;
    }
  }
  return nullptr;
}

std::unique_ptr<MFXVideoDECODE> VplDecoder::CreateDecoderInternal(
    std::shared_ptr<VplSession> session,
    mfxU32 codec,
    int width,
    int height) {
  std::unique_ptr<MFXVideoDECODE> decoder(
      new MFXVideoDECODE(GetVplSession(session)));

  mfxStatus sts = MFX_ERR_NONE;

  mfxVideoParam param;
  memset(&param, 0, sizeof(param));

  param.mfx.CodecId = codec;
  param.mfx.FrameInfo.FourCC = MFX_FOURCC_NV12;
  param.mfx.FrameInfo.ChromaFormat = MFX_CHROMAFORMAT_YUV420;
  param.mfx.FrameInfo.PicStruct = MFX_PICSTRUCT_PROGRESSIVE;
  param.mfx.FrameInfo.CropX = 0;
  param.mfx.FrameInfo.CropY = 0;
  param.mfx.FrameInfo.CropW = width;
  param.mfx.FrameInfo.CropH = height;
  param.mfx.FrameInfo.Width = (width + 15) / 16 * 16;
  param.mfx.FrameInfo.Height = (height + 15) / 16 * 16;

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
    std::cerr << "Unsupported decoder codec: codec=" << codec_str
              << " sts=" << sts << std::endl;
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
      std::cerr << "decoder->Init() failed: " << sts << std::endl;
      return nullptr;
    }
  }

  return decoder;
}

bool VplDecoder::IsSupported(std::shared_ptr<VplSession> session,
                             std::uint32_t fourcc) {
  if (!session) {
    return false;
  }

  auto decoder = CreateDecoder(session, fourcc, {{4096, 4096}, {2048, 2048}});

  return decoder != nullptr;
}

}  // namespace hisui::video