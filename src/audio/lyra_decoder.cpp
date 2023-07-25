#include "audio/lyra_decoder.hpp"

#include <fmt/core.h>
#include <lyra.h>

#include <stdexcept>

#include "constants.hpp"

namespace hisui::audio {

LyraDecoder::LyraDecoder(const int t_channles, const std::string& model_path)
    : m_channels(t_channles) {
  if (m_channels != 1) {
    throw std::invalid_argument(
        fmt::format("invalid number of channels: {}", m_channels));
  }
  m_decoder = lyra_decoder_create(hisui::Constants::PCM_SAMPLE_RATE, m_channels,
                                  model_path.c_str());
  if (m_decoder == nullptr) {
    throw std::runtime_error("could not create lyra decoder");
  }

  m_lyra_buffer = new std::int16_t[10000];  // TODO(haruyama)
}

LyraDecoder::~LyraDecoder() {
  if (m_lyra_buffer) {
    delete[] m_lyra_buffer;
  }
  if (m_decoder) {
    lyra_decoder_destroy(m_decoder);
  }
}

std::pair<const std::int16_t*, const std::size_t> LyraDecoder::decode(
    const unsigned char* src_buffer,
    const std::size_t src_buffer_length) {
  auto r =
      lyra_decoder_set_encoded_packet(m_decoder, src_buffer, src_buffer_length);
  if (!r) {
    throw std::runtime_error("lyra_decoder_set_encoded_packet() failed");
  }
  auto v = lyra_decoder_decode_samples(
      m_decoder, hisui::Constants::PCM_SAMPLE_RATE /
                     50);  // 50 は chromemedia::codec::kFrameRate 由来
  if (v == nullptr) {
    throw std::runtime_error("lyra_decoder_decode_samples() failed");
  }
  auto samples = lyra_vector_s16_get_size(v);
  auto p = lyra_vector_s16_get_data(v);
  std::memcpy(m_lyra_buffer, p, samples * 2);
  lyra_vector_s16_destroy(v);
  return {m_lyra_buffer, samples};
}

}  // namespace hisui::audio
