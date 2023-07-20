#include "audio/lyra_decoder.hpp"

#include <fmt/core.h>

#include <stdexcept>

#include "constants.hpp"

namespace hisui::audio {

LyraDecoder::LyraDecoder(const int t_channles, const std::string& model_path)
    : m_channels(t_channles) {
  if (m_channels != 1) {
    throw std::invalid_argument(
        fmt::format("invalid number of channels: {}", m_channels));
  }
}

LyraDecoder::~LyraDecoder() {}

std::pair<const std::int16_t*, const std::size_t> LyraDecoder::decode(
    const unsigned char*,
    const std::size_t) {
  throw std::logic_error("unimplemented");
}

}  // namespace hisui::audio
