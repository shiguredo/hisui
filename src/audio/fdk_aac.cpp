#include "audio/fdk_aac.hpp"

#include <fdk-aac/aacenc_lib.h>

#include <cstdint>
#include <stdexcept>

namespace hisui::audio {

void fdk_aac_init(::HANDLE_AACENCODER* handle,
                  ::AACENC_InfoStruct* info,
                  const FDKAACInitParameters& params) {
  if (::aacEncOpen(handle, 0, params.channels) != ::AACENC_OK) {
    throw std::runtime_error("Unable to open encoder");
  }
  if (::aacEncoder_SetParam(*handle, ::AACENC_AOT, params.aot) != ::AACENC_OK) {
    throw std::runtime_error("Unable to set the AOT");
  }
  if (::aacEncoder_SetParam(*handle, ::AACENC_SAMPLERATE, params.sample_rate) !=
      ::AACENC_OK) {
    throw std::runtime_error("Unable to set the sample rate");
  }
  if (::aacEncoder_SetParam(*handle, ::AACENC_CHANNELMODE,
                            static_cast<std::uint32_t>(params.mode)) !=
      ::AACENC_OK) {
    throw std::runtime_error("Unable to set the channel mode");
  }
  if (::aacEncoder_SetParam(*handle, ::AACENC_CHANNELORDER, 1) != ::AACENC_OK) {
    throw std::runtime_error("Unable to set the wav channel order");
  }
  if (::aacEncoder_SetParam(*handle, ::AACENC_BITRATE, params.bit_rate) !=
      ::AACENC_OK) {
    throw std::runtime_error("Unable to set the bitrate");
  }
  if (::aacEncoder_SetParam(*handle, ::AACENC_TRANSMUX,
                            params.transport_type) != ::AACENC_OK) {
    throw std::runtime_error("Unable to set the ADTS transmux");
  }
  if (::aacEncoder_SetParam(*handle, ::AACENC_AFTERBURNER,
                            params.afterburner) != ::AACENC_OK) {
    throw std::runtime_error("Unable to set the afterburner mode");
  }
  if (::aacEncEncode(*handle, nullptr, nullptr, nullptr, nullptr) !=
      ::AACENC_OK) {
    throw std::runtime_error("Unable to initialize the encoder");
  }
  if (::aacEncInfo(*handle, info) != ::AACENC_OK) {
    throw std::runtime_error("Unable to get the encoder info");
  }
}
}  // namespace hisui::audio
