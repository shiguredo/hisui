#pragma once

#include <fdk-aac/FDK_audio.h>
#include <fdk-aac/aacenc_lib.h>

#include <cstdint>

#include "constants.hpp"

namespace hisui::audio {

struct FDKAACInitParameters {
  const std::uint32_t channels = 2;
  const std::uint32_t aot = 2;
  const std::uint32_t sample_rate = 48000;
  const std::uint32_t afterburner = 1;
  const std::uint32_t bit_rate = Constants::FDK_AAC_DEFAULT_BIT_RATE;
  const CHANNEL_MODE mode = ::MODE_2;
  const std::uint32_t transport_type = ::TT_MP4_RAW;
};

void fdk_aac_init(::HANDLE_AACENCODER*,
                  ::AACENC_InfoStruct*,
                  const FDKAACInitParameters&);

}  // namespace hisui::audio
