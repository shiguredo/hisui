#include "audio/lyra_handler.hpp"

#include <fmt/core.h>

#include <filesystem>
#include <stdexcept>

namespace hisui::audio {

void LyraHandler::setModelPath(const std::string& model_path) {
  if (!m_handler) {
    m_handler = new LyraHandler(model_path);
  }
}

bool LyraHandler::hasInstance() {
  return m_handler != nullptr;
}

LyraHandler& LyraHandler::getInstance() {
  return *m_handler;
}

LyraHandler::LyraHandler(const std::string& model_path) {
  if (!std::empty(m_model_path)) {
    return;
  }

  if (!std::filesystem::is_directory(model_path)) {
    throw std::invalid_argument(fmt::format("{} is not directory", model_path));
  }

  m_model_path = model_path;
}

void LyraHandler::close() {
  delete m_handler;
  m_handler = nullptr;
}

std::string LyraHandler::getModelPath() const {
  if (!hasInstance()) {
    throw std::runtime_error("lyra model path is not set");
  }
  return m_model_path;
}

}  // namespace hisui::audio
