#include "util/file.hpp"

#include <fmt/core.h>
#include <glob.h>

#include <filesystem>
#include <stdexcept>
#include <string>

namespace hisui::util {

FindFileResult find_file(const std::string& filename) {
  auto path = std::filesystem::path(filename);
  if (path.is_absolute()) {
    if (!std::filesystem::exists(path)) {
      return {.found = false,
              .message = fmt::format("does not exist path({})", filename)};
    }
    return {.found = true, .path = path};
  }
  if (std::filesystem::exists(path)) {
    return {.found = true, .path = std::filesystem::absolute(path)};
  }
  path = std::filesystem::absolute(path.filename());
  if (std::filesystem::exists(path)) {
    return {.found = true, .path = path};
  }
  return {.found = false,
          .message = fmt::format("does not exist path({})", filename)};
}

std::vector<std::string> glob(const std::string& pattern) {
  ::glob_t globbuf;
  std::vector<std::string> filenames;

  if (auto ret = ::glob(pattern.c_str(), 0, nullptr, &globbuf)) {
    if (ret == GLOB_NOMATCH) {
      return filenames;
    }
    throw std::runtime_error(
        fmt::format("glob({}) failed: return_value={}", pattern, ret));
  }

  for (std::size_t i = 0; i < globbuf.gl_pathc; ++i) {
    filenames.push_back(globbuf.gl_pathv[i]);
  }

  ::globfree(&globbuf);

  return filenames;
}

}  // namespace hisui::util
