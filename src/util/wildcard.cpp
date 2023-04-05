#include "util/wildcard.hpp"

// based on https://github.com/richsalz/wildmat

namespace {

bool match(const char* text, const char* p) {
  for (; *p; text++, p++) {
    if (*text == '\0' && *p != '*') {
      return false;
    }
    switch (*p) {
      case '*':
        while (*++p == '*') {
          continue;
        }
        if (*p == '\0') {
          return true;
        }
        while (*text) {
          if (match(text++, p)) {
            return true;
          }
        }
        return false;
      default:
        if (*text != *p) {
          return false;
        }
    }
  }
  return *text == '\0';
}

}  // namespace

namespace hisui::util {

bool wildcard_match(const WildcardMatchParameters& params) {
  if (params.pattern == "*") {
    return true;
  }
  return match(params.text.c_str(), params.pattern.c_str());
}

}  // namespace hisui::util
