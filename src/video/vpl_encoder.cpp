#include "video/vpl_encoder.hpp"

#include <libyuv/convert_from.h>
#include <spdlog/spdlog.h>
#include <vpl/mfxvp8.h>

#include <memory>

#include <video/vpl.hpp>

#include "config.hpp"
#include "frame.hpp"

namespace hisui::video {

VPLEncoderConfig::VPLEncoderConfig(const std::uint32_t t_width,
                                   const std::uint32_t t_height,
                                   const hisui::Config& config)
    : width(t_width),
      height(t_height),
      fps(config.out_video_frame_rate),
      target_bit_rate(config.out_video_bit_rate * 1000),
      max_bit_rate(config.out_video_bit_rate * 1000) {}

std::unique_ptr<MFXVideoENCODE> VPLEncoder::createEncoder(
    const ::mfxU32 codec,
    const std::uint32_t width,
    const std::uint32_t height,
    const boost::rational<std::uint64_t> frame_rate,
    const std::uint32_t target_bit_rate,
    const std::uint32_t max_bit_rate,
    const bool init) {
  if (!hisui::video::VPLSession::hasInstance()) {
    throw std::runtime_error("VPL session is not opened");
  }
  ::mfxStatus sts = MFX_ERR_NONE;

  ::mfxVideoParam param;
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
      new MFXVideoENCODE(hisui::video::VPLSession::getInstance().getSession()));

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

bool VPLEncoder::isSupported(const std::uint32_t fourcc) {
  auto encoder =
      createEncoder(ToMfxCodec(fourcc), 1920, 1080, 30, 10, 20, false);
  return encoder != nullptr;
}

VPLEncoder::VPLEncoder(const std::uint32_t t_fourcc,
                       std::queue<hisui::Frame>* t_buffer,
                       const VPLEncoderConfig& t_config,
                       const std::uint64_t t_timescale)
    : m_fourcc(t_fourcc), m_buffer(t_buffer), m_timescale(t_timescale) {
  m_width = t_config.width;
  m_height = t_config.height;
  m_fps = t_config.fps;
  m_bitrate = t_config.target_bit_rate;
  m_encoder = createEncoder(
      ToMfxCodec(m_fourcc), t_config.width, t_config.height, t_config.fps,
      t_config.target_bit_rate, t_config.max_bit_rate, true);
  if (!m_encoder) {
    throw std::runtime_error("createEncoder() failed:");
  }
  initVPL();
}

VPLEncoder::~VPLEncoder() {
  if (m_frame > 0) {
    spdlog::debug("VPLEncoder: number of frames: {}", m_frame);
    spdlog::debug("VPLEncoder: final average bitrate (kbps): {}",
                  m_sum_of_bits * m_fps.numerator() / m_fps.denominator() /
                      static_cast<std::uint64_t>(m_frame) / 1024);
  }
  releaseVPL();
}

void VPLEncoder::initVPL() {
  ::mfxStatus sts = MFX_ERR_NONE;

  ::mfxVideoParam param;
  memset(&param, 0, sizeof(param));

  // Retrieve video parameters selected by encoder.
  // - BufferSizeInKB parameter is required to set bit stream buffer size
  sts = m_encoder->GetVideoParam(&param);
  if (sts != MFX_ERR_NONE) {
    throw std::runtime_error(fmt::format("GetVideoParam() failed: sts={}",
                                         static_cast<std::int32_t>(sts)));
  }
  spdlog::info("BufferSizeInKB={}", param.mfx.BufferSizeInKB);

  // Query number of required surfaces for encoder
  memset(&m_alloc_request, 0, sizeof(m_alloc_request));
  sts = m_encoder->QueryIOSurf(&param, &m_alloc_request);
  if (sts != MFX_ERR_NONE) {
    throw std::runtime_error(fmt::format("QueryIOSurf() failed: sts={}",
                                         static_cast<std::int32_t>(sts)));
  }
  spdlog::info("Encoder NumFrameSuggested={}",
               m_alloc_request.NumFrameSuggested);

  m_frame_info = param.mfx.FrameInfo;

  // 出力ビットストリームの初期化
  m_bitstream_buffer.resize(param.mfx.BufferSizeInKB * 1000);

  memset(&m_bitstream, 0, sizeof(m_bitstream));
  m_bitstream.MaxLength = static_cast<std::uint32_t>(m_bitstream_buffer.size());
  m_bitstream.Data = m_bitstream_buffer.data();

  // 必要な枚数分の入力サーフェスを作る
  {
    int width = (m_alloc_request.Info.Width + 31) / 32 * 32;
    int height = (m_alloc_request.Info.Height + 31) / 32 * 32;
    // 1枚あたりのバイト数
    // NV12 なので 1 ピクセルあたり 12 ビット
    int size = width * height * 12 / 8;
    m_surface_buffer.resize(
        static_cast<std::size_t>(m_alloc_request.NumFrameSuggested * size));

    m_surfaces.clear();
    m_surfaces.reserve(m_alloc_request.NumFrameSuggested);
    for (int i = 0; i < m_alloc_request.NumFrameSuggested; i++) {
      ::mfxFrameSurface1 surface;
      memset(&surface, 0, sizeof(surface));
      surface.Info = m_frame_info;
      surface.Data.Y = m_surface_buffer.data() + i * size;
      surface.Data.U = m_surface_buffer.data() + i * size + width * height;
      surface.Data.V = m_surface_buffer.data() + i * size + width * height + 1;
      surface.Data.Pitch = static_cast<std::uint16_t>(width);
      m_surfaces.push_back(surface);
    }
  }
}

void VPLEncoder::releaseVPL() {
  if (m_encoder) {
    m_encoder->Close();
  }
}

void VPLEncoder::outputImage(const std::vector<unsigned char>& yuv) {
  encodeFrame(yuv);
  ++m_frame;
}

void VPLEncoder::encodeFrame(const std::vector<unsigned char>& yuv) {
  // 使ってない入力サーフェスを取り出す
  auto surface =
      std::find_if(m_surfaces.begin(), m_surfaces.end(),
                   [](const ::mfxFrameSurface1& s) { return !s.Data.Locked; });
  if (surface == m_surfaces.end()) {
    throw std::runtime_error("unlocked surface is not found");
  }

  auto yuv_data = yuv.data();

  // I420 から NV12 に変換
  libyuv::I420ToNV12(
      yuv_data, static_cast<int>(m_width), yuv_data + m_width * m_height,
      m_width >> 1, yuv_data + m_width * m_height + ((m_width * m_height) >> 2),
      m_width >> 1, surface->Data.Y, surface->Data.Pitch, surface->Data.U,
      surface->Data.Pitch, static_cast<int>(m_width),
      static_cast<int>(m_height));

  ::mfxStatus sts;

  ::mfxEncodeCtrl ctrl;
  memset(&ctrl, 0, sizeof(ctrl));
  ctrl.FrameType = MFX_FRAMETYPE_UNKNOWN;

  // NV12 をハードウェアエンコード
  ::mfxSyncPoint syncp;
  sts = m_encoder->EncodeFrameAsync(&ctrl, &*surface, &m_bitstream, &syncp);
  // alloc_request_.NumFrameSuggested が 1 の場合は MFX_ERR_MORE_DATA は発生しない
  if (sts == MFX_ERR_MORE_DATA) {
    // もっと入力が必要なので出直す
    return;
  }
  if (sts != MFX_ERR_NONE) {
    throw std::runtime_error(fmt::format("EncodeFrameAsync() failed: sts={}",
                                         static_cast<std::int32_t>(sts)));
  }

  sts = ::MFXVideoCORE_SyncOperation(
      hisui::video::VPLSession::getInstance().getSession(), syncp, 600000);
  if (sts != MFX_ERR_NONE) {
    throw std::runtime_error(
        fmt::format("MFXVideoCORE_SyncOperation() failed: sts={}",
                    static_cast<std::int32_t>(sts)));
  }

  {
    std::uint8_t* p = m_bitstream.Data + m_bitstream.DataOffset;
    std::uint32_t data_size = m_bitstream.DataLength;
    m_bitstream.DataLength = 0;

    std::uint8_t* data = new std::uint8_t[data_size];
    std::copy_n(p, data_size, data);
    m_sum_of_bits += data_size * 8;
    const std::uint64_t pts_ns = static_cast<std::uint64_t>(m_frame) *
                                 m_timescale * m_fps.denominator() /
                                 m_fps.numerator();
    m_buffer->push(
        hisui::Frame{.timestamp = pts_ns,
                     .data = data,
                     .data_size = data_size,
                     .is_key = m_bitstream.FrameType == MFX_FRAMETYPE_IDR ||
                               m_bitstream.FrameType == MFX_FRAMETYPE_I});
  }
}

void VPLEncoder::flush() {}

std::uint32_t VPLEncoder::getFourcc() const {
  return m_fourcc;
}

void VPLEncoder::setResolutionAndBitrate(const std::uint32_t,
                                         const std::uint32_t,
                                         const std::uint32_t) {
  throw std::runtime_error(
      "VPLEncoder::setResolutionAndBitrate is not implemented");
}

}  // namespace hisui::video
