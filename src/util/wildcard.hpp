#pragma once

#include <string>

namespace hisui::util {

struct WildcardMatchParameters {
  std::string text;
  std::string pattern;
};

bool wildcard_match(const WildcardMatchParameters&);

}  // namespace hisui::util
