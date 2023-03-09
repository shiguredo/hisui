#include "audio/buffer_fdk_aac_encoder.hpp"

#include <fmt/core.h>
#include <spdlog/spdlog.h>

#include <algorithm>
#include <cstddef>
#include <cstdint>
#include <iterator>
#include <stdexcept>

#include "audio/fdk_aac.hpp"
#include "constants.hpp"
#include "frame.hpp"

namespace hisui::audio {

BufferFDKAACEncoder::BufferFDKAACEncoder(
    std::queue<hisui::Frame>* t_buffer,
    const BufferFDKAACEncoderParameters& params)
    : m_buffer(t_buffer) {
  ::AACENC_InfoStruct info;
  std::uint32_t channels = 2;

  fdk_aac_init(&m_handle, &info,
               {.channels = channels, .bit_rate = params.bit_rate});

  m_max_sample_size =
      2 *
      info.frameLength;  // 2 = (bits_per_sample) / 8 where bits_per_sample = 16
  m_aac_buffer = new std::uint8_t[Constants::FDK_AAC_ENCODE_BUFFER_SIZE];
}

BufferFDKAACEncoder::~BufferFDKAACEncoder() {
  if (m_aac_buffer) {
    delete[] m_aac_buffer;
  }
  ::aacEncClose(&m_handle);
}

void BufferFDKAACEncoder::addSample(const std::int16_t left,
                                    const std::int16_t right) {
  m_pcm_buffer.push_back(left);
  m_pcm_buffer.push_back(right);

  if (std::size(m_pcm_buffer) >= m_max_sample_size) {
    encodeAndWrite();
    m_timestamp += m_max_sample_size / 2;
  }
}

void BufferFDKAACEncoder::flush() {
  if (std::size(m_pcm_buffer) > 0) {
    encodeAndWrite();
  }
}

void BufferFDKAACEncoder::encodeAndWrite() {
  ::AACENC_BufDesc in_buf, out_buf;
  ::AACENC_InArgs in_args;
  ::AACENC_OutArgs out_args;
  int in_identifier = ::IN_AUDIO_DATA;
  int in_size, in_elem_size;
  int out_identifier = ::OUT_BITSTREAM_DATA;
  int out_size, out_elem_size;
  void *in_ptr, *out_ptr;
  ::AACENC_ERROR err;

  const int num_in_samples = static_cast<int>(std::size(m_pcm_buffer));

  in_ptr = m_pcm_buffer.data();
  in_size = num_in_samples * 2;
  in_elem_size = 2;

  in_args.numInSamples = num_in_samples;
  in_buf.numBufs = 1;
  in_buf.bufs = &in_ptr;
  in_buf.bufferIdentifiers = &in_identifier;
  in_buf.bufSizes = &in_size;
  in_buf.bufElSizes = &in_elem_size;

  out_ptr = m_aac_buffer;
  out_size = Constants::FDK_AAC_ENCODE_BUFFER_SIZE;
  out_elem_size = 1;
  out_buf.numBufs = 1;
  out_buf.bufs = &out_ptr;
  out_buf.bufferIdentifiers = &out_identifier;
  out_buf.bufSizes = &out_size;
  out_buf.bufElSizes = &out_elem_size;

  if ((err = ::aacEncEncode(m_handle, &in_buf, &out_buf, &in_args,
                            &out_args)) != ::AACENC_OK) {
    if (err == ::AACENC_ENCODE_EOF) {
      return;
    }
    throw std::runtime_error("Encoding failed");
  }

  if (out_args.numOutBytes == 0) {
    return;
  }

  const std::size_t data_size = static_cast<std::size_t>(out_args.numOutBytes);
  if (data_size <= 7) {
    throw std::runtime_error(
        fmt::format("out_args.numOutBytes is too small: {}", data_size));
  }
  auto headless_data_size = data_size - 7;

  std::uint8_t* data = new std::uint8_t[headless_data_size];
  std::copy_n(m_aac_buffer + 7, headless_data_size, data);
  m_buffer->push(hisui::Frame{.timestamp = m_timestamp,
                              .data = data,
                              .data_size = headless_data_size,
                              .is_key = true});

  m_pcm_buffer.clear();
}

}  // namespace hisui::audio
