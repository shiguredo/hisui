#ifndef LYRA_H_INCLUDED
#define LYRA_H_INCLUDED

#include <stddef.h>
#include <stdint.h>

extern "C" {

struct lyra_decoder;
struct lyra_vector_s16;

lyra_decoder* lyra_decoder_create(int sample_rate_hz,
                                  int num_channels,
                                  const char* model_path);
bool lyra_decoder_set_encoded_packet(lyra_decoder* decoder,
                                     const uint8_t* encoded,
                                     size_t length);
lyra_vector_s16* lyra_decoder_decode_samples(lyra_decoder* decoder,
                                             int num_samples);
void lyra_decoder_destroy(lyra_decoder* decoder);

size_t lyra_vector_s16_get_size(lyra_vector_s16* v);
int16_t* lyra_vector_s16_get_data(lyra_vector_s16* v);
void lyra_vector_s16_destroy(lyra_vector_s16* v);
}

#endif
