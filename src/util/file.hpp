#pragma once

#include <filesystem>
#include <string>
#include <vector>

namespace hisui::util {

struct FindFileResult {
  bool found;
  std::filesystem::path path;
  std::string message;
};

FindFileResult find_file(const std::string&);

std::vector<std::string> glob(const std::string&);

}  // namespace hisui::util
