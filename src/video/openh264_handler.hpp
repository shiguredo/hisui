#pragma once

#include <codec/api/wels/codec_app_def.h>

#include <string>

class ISVCDecoder;
class ISVCEncoder;

namespace hisui::video {

class OpenH264Handler {
 public:
  OpenH264Handler(const OpenH264Handler&) = delete;
  OpenH264Handler& operator=(const OpenH264Handler&) = delete;
  OpenH264Handler(OpenH264Handler&&) = delete;
  OpenH264Handler& operator=(OpenH264Handler&&) = delete;

  using CreateDecoderFunc = long (*)(::ISVCDecoder**); /* NOLINT */
  using DestroyDecoderFunc = void (*)(::ISVCDecoder*);
  using CreateEncoderFunc = int (*)(::ISVCEncoder**);
  using DestroyEncoderFunc = void (*)(::ISVCEncoder*);
  using GetCodecVersoinFunc = ::OpenH264Version (*)();
  CreateDecoderFunc createDecoder = nullptr;
  DestroyDecoderFunc destroyDecoder = nullptr;
  CreateEncoderFunc createEncoder = nullptr;
  DestroyEncoderFunc destroyEncoder = nullptr;
  GetCodecVersoinFunc getCodecVersion = nullptr;

  static void open(const std::string&);
  static bool hasInstance();
  static OpenH264Handler& getInstance();
  static void close();

 private:
  void* m_openh264_handle = nullptr;
  inline static OpenH264Handler* m_handler = nullptr;

  explicit OpenH264Handler(const std::string&);
  ~OpenH264Handler();
};

}  // namespace hisui::video
