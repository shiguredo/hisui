#include "video/vpl_encoder.hpp"

#include <spdlog/spdlog.h>
#include <vpl/mfxvp8.h>

#include <memory>

#include <video/vpl.hpp>

namespace hisui::video {

std::unique_ptr<MFXVideoENCODE> VplEncoder::CreateEncoder(
    const ::mfxU32 codec,
    const std::uint32_t width,
    const std::uint32_t height,
    const boost::rational<std::uint64_t> frame_rate,
    const std::uint32_t target_bit_rate,
    const std::uint32_t max_bit_rate,
    const bool init) {
  if (!hisui::video::VplSession::hasInstance()) {
    throw std::runtime_error("VPL session is not opened");
  }
  mfxStatus sts = MFX_ERR_NONE;

  mfxVideoParam param;
  memset(&param, 0, sizeof(param));

  param.mfx.CodecId = codec;
  if (codec == MFX_CODEC_VP8) {
    //param.mfx.CodecProfile = MFX_PROFILE_VP8_0;
  } else if (codec == MFX_CODEC_VP9) {
    //param.mfx.CodecProfile = MFX_PROFILE_VP9_0;
  } else if (codec == MFX_CODEC_AVC) {
    //param.mfx.CodecProfile = MFX_PROFILE_AVC_HIGH;
    //param.mfx.CodecLevel = MFX_LEVEL_AVC_51;
    //param.mfx.CodecProfile = MFX_PROFILE_AVC_MAIN;
    //param.mfx.CodecLevel = MFX_LEVEL_AVC_1;
  } else if (codec == MFX_CODEC_AV1) {
    //param.mfx.CodecProfile = MFX_PROFILE_AV1_MAIN;
  }
  param.mfx.TargetUsage = MFX_TARGETUSAGE_BALANCED;
  //param.mfx.BRCParamMultiplier = 1;
  //param.mfx.InitialDelayInKB = target_kbps;
  param.mfx.TargetKbps = static_cast<std::uint16_t>(target_bit_rate);
  param.mfx.MaxKbps = static_cast<std::uint16_t>(max_bit_rate);
  param.mfx.RateControlMethod = MFX_RATECONTROL_VBR;
  //param.mfx.NumSlice = 1;
  //param.mfx.NumRefFrame = 1;
  param.mfx.FrameInfo.FrameRateExtN =
      static_cast<std::uint32_t>(frame_rate.numerator());
  param.mfx.FrameInfo.FrameRateExtD =
      static_cast<std::uint32_t>(frame_rate.denominator());
  param.mfx.FrameInfo.FourCC = MFX_FOURCC_NV12;
  param.mfx.FrameInfo.ChromaFormat = MFX_CHROMAFORMAT_YUV420;
  param.mfx.FrameInfo.PicStruct = MFX_PICSTRUCT_PROGRESSIVE;
  param.mfx.FrameInfo.CropX = 0;
  param.mfx.FrameInfo.CropY = 0;
  param.mfx.FrameInfo.CropW = static_cast<std::uint16_t>(width);
  param.mfx.FrameInfo.CropH = static_cast<std::uint16_t>(height);
  // Width must be a multiple of 16
  // Height must be a multiple of 16 in case of frame picture and a multiple of 32 in case of field picture
  param.mfx.FrameInfo.Width =
      (static_cast<std::uint16_t>(width) + 15) / 16 * 16;
  param.mfx.FrameInfo.Height =
      (static_cast<std::uint16_t>(height) + 15) / 16 * 16;

  //param.mfx.GopOptFlag = MFX_GOP_STRICT | MFX_GOP_CLOSED;
  //param.mfx.IdrInterval = codec_settings->H264().keyFrameInterval;
  //param.mfx.IdrInterval = 0;
  param.mfx.GopRefDist = 1;
  //param.mfx.EncodedOrder = 0;
  param.AsyncDepth = 1;
  param.IOPattern =
      MFX_IOPATTERN_IN_SYSTEM_MEMORY | MFX_IOPATTERN_OUT_SYSTEM_MEMORY;

  mfxExtBuffer* ext_buffers[10];
  mfxExtCodingOption ext_coding_option;
  mfxExtCodingOption2 ext_coding_option2;
  int ext_buffers_size = 0;
  if (codec == MFX_CODEC_AVC) {
    memset(&ext_coding_option, 0, sizeof(ext_coding_option));
    ext_coding_option.Header.BufferId = MFX_EXTBUFF_CODING_OPTION;
    ext_coding_option.Header.BufferSz = sizeof(ext_coding_option);
    ext_coding_option.AUDelimiter = MFX_CODINGOPTION_OFF;
    ext_coding_option.MaxDecFrameBuffering = 1;
    //ext_coding_option.NalHrdConformance = MFX_CODINGOPTION_OFF;
    //ext_coding_option.VuiVclHrdParameters = MFX_CODINGOPTION_ON;
    //ext_coding_option.SingleSeiNalUnit = MFX_CODINGOPTION_ON;
    //ext_coding_option.RefPicMarkRep = MFX_CODINGOPTION_OFF;
    //ext_coding_option.PicTimingSEI = MFX_CODINGOPTION_OFF;
    //ext_coding_option.RecoveryPointSEI = MFX_CODINGOPTION_OFF;
    //ext_coding_option.FramePicture = MFX_CODINGOPTION_OFF;
    //ext_coding_option.FieldOutput = MFX_CODINGOPTION_ON;

    memset(&ext_coding_option2, 0, sizeof(ext_coding_option2));
    ext_coding_option2.Header.BufferId = MFX_EXTBUFF_CODING_OPTION2;
    ext_coding_option2.Header.BufferSz = sizeof(ext_coding_option2);
    ext_coding_option2.RepeatPPS = MFX_CODINGOPTION_ON;
    //ext_coding_option2.MaxSliceSize = 1;
    //ext_coding_option2.AdaptiveI = MFX_CODINGOPTION_ON;

    ext_buffers[0] = reinterpret_cast<mfxExtBuffer*>(&ext_coding_option);
    ext_buffers[1] = reinterpret_cast<mfxExtBuffer*>(&ext_coding_option2);
    ext_buffers_size = 2;
  }

  if (ext_buffers_size != 0) {
    param.ExtParam = ext_buffers;
    param.NumExtParam = static_cast<std::uint16_t>(ext_buffers_size);
  }

  std::unique_ptr<MFXVideoENCODE> encoder(
      new MFXVideoENCODE(hisui::video::VplSession::getInstance().getSession()));

  // MFX_ERR_NONE	The function completed successfully.
  // MFX_ERR_UNSUPPORTED	The function failed to identify a specific implementation for the required features.
  // MFX_WRN_PARTIAL_ACCELERATION	The underlying hardware does not fully support the specified video parameters; The encoding may be partially accelerated. Only SDK HW implementations may return this status code.
  // MFX_WRN_INCOMPATIBLE_VIDEO_PARAM	The function detected some video parameters were incompatible with others; incompatibility resolved.
  mfxVideoParam bk_param;
  memcpy(&bk_param, &param, sizeof(bk_param));
  sts = encoder->Query(&param, &param);
  if (sts < 0) {
    memcpy(&param, &bk_param, sizeof(bk_param));

    // 失敗したら LowPower ON にした状態でもう一度確認する
    param.mfx.LowPower = MFX_CODINGOPTION_ON;
    if (codec == MFX_CODEC_AVC) {
      param.mfx.RateControlMethod = MFX_RATECONTROL_CQP;
      param.mfx.QPI = 25;
      param.mfx.QPP = 33;
      param.mfx.QPB = 40;
      //param.IOPattern = MFX_IOPATTERN_IN_SYSTEM_MEMORY;
    }
    memcpy(&bk_param, &param, sizeof(bk_param));
    sts = encoder->Query(&param, &param);
    if (sts < 0) {
      const char* codec_str = codec == MFX_CODEC_VP8   ? "MFX_CODEC_VP8"
                              : codec == MFX_CODEC_VP9 ? "MFX_CODEC_VP9"
                              : codec == MFX_CODEC_AV1 ? "MFX_CODEC_AV1"
                              : codec == MFX_CODEC_AVC ? "MFX_CODEC_AVC"
                                                       : "MFX_CODEC_UNKNOWN";
      spdlog::warn("Unsupported encoder codec: codec={}, sts={}", codec_str,
                   static_cast<std::int32_t>(sts));
      return nullptr;
    }
  }

  //#define F(NAME)                                              \
  //  if (bk_param.NAME != param.NAME)                           \
  //  std::cout << "param " << #NAME << " old=" << bk_param.NAME \
  //            << " new=" << param.NAME << std::endl
  //
  //  F(mfx.LowPower);
  //  F(mfx.BRCParamMultiplier);
  //  F(mfx.FrameInfo.FrameRateExtN);
  //  F(mfx.FrameInfo.FrameRateExtD);
  //  F(mfx.FrameInfo.FourCC);
  //  F(mfx.FrameInfo.ChromaFormat);
  //  F(mfx.FrameInfo.PicStruct);
  //  F(mfx.FrameInfo.CropX);
  //  F(mfx.FrameInfo.CropY);
  //  F(mfx.FrameInfo.CropW);
  //  F(mfx.FrameInfo.CropH);
  //  F(mfx.FrameInfo.Width);
  //  F(mfx.FrameInfo.Height);
  //  F(mfx.CodecId);
  //  F(mfx.CodecProfile);
  //  F(mfx.CodecLevel);
  //  F(mfx.GopPicSize);
  //  F(mfx.GopRefDist);
  //  F(mfx.GopOptFlag);
  //  F(mfx.IdrInterval);
  //  F(mfx.TargetUsage);
  //  F(mfx.RateControlMethod);
  //  F(mfx.InitialDelayInKB);
  //  F(mfx.TargetKbps);
  //  F(mfx.MaxKbps);
  //  F(mfx.BufferSizeInKB);
  //  F(mfx.NumSlice);
  //  F(mfx.NumRefFrame);
  //  F(mfx.EncodedOrder);
  //  F(mfx.DecodedOrder);
  //  F(mfx.ExtendedPicStruct);
  //  F(mfx.TimeStampCalc);
  //  F(mfx.SliceGroupsPresent);
  //  F(mfx.MaxDecFrameBuffering);
  //  F(mfx.EnableReallocRequest);
  //  F(AsyncDepth);
  //  F(IOPattern);
  //#undef F

  //if (sts != MFX_ERR_NONE) {
  //  const char* codec_str = codec == MFX_CODEC_VP8   ? "MFX_CODEC_VP8"
  //                          : codec == MFX_CODEC_VP9 ? "MFX_CODEC_VP9"
  //                          : codec == MFX_CODEC_AV1 ? "MFX_CODEC_AV1"
  //                          : codec == MFX_CODEC_AVC ? "MFX_CODEC_AVC"
  //                                                   : "MFX_CODEC_UNKNOWN";
  //  std::cerr << "Supported specified codec but has warning: codec="
  //            << codec_str << " sts=" << sts << std::endl;
  //}

  if (init) {
    sts = encoder->Init(&param);
    if (sts != MFX_ERR_NONE) {
      spdlog::warn("Failed to Init: sts={}", static_cast<std::int32_t>(sts));
      return nullptr;
    }
  }

  return encoder;
}

bool VplEncoder::IsSupported(const std::uint32_t fourcc) {
  auto encoder =
      CreateEncoder(ToMfxCodec(fourcc), 1920, 1080, 30, 10, 20, false);
  return encoder != nullptr;
}

}  // namespace hisui::video
