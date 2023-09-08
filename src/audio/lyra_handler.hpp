#pragma once

#include <codec/api/wels/codec_app_def.h>

#include <string>

namespace hisui::audio {

class LyraHandler {
 public:
  LyraHandler(const LyraHandler&) = delete;
  LyraHandler& operator=(const LyraHandler&) = delete;
  LyraHandler(LyraHandler&&) = delete;
  LyraHandler& operator=(LyraHandler&&) = delete;

  std::string getModelPath() const;

  static void setModelPath(const std::string&);
  static bool hasInstance();
  static LyraHandler& getInstance();
  static void close();

 private:
  std::string m_model_path = "";
  inline static LyraHandler* m_handler = nullptr;

  explicit LyraHandler(const std::string&);
};

}  // namespace hisui::audio
