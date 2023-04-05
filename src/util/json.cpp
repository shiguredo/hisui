#include "util/json.hpp"

#include <fmt/core.h>

#include <cstdint>
#include <stdexcept>
#include <string>

#include <boost/json/array.hpp>
#include <boost/json/impl/array.hpp>
#include <boost/json/object.hpp>
#include <boost/json/parse.hpp>
#include <boost/json/src.hpp>  // https://github.com/boostorg/json#header-only
#include <boost/json/string.hpp>
#include <boost/json/system_error.hpp>
#include <boost/json/value.hpp>

namespace hisui::util {

boost::json::string get_string_from_json_object(boost::json::object o,
                                                const std::string& key) {
  if (o[key].is_string()) {
    return o[key].as_string();
  }
  throw std::runtime_error(fmt::format("{} is not string", key));
}

boost::json::string get_string_from_json_object_with_default(
    boost::json::object o,
    const std::string& key,
    const std::string& s) {
  if (!o.contains(key) || o[key].is_null()) {
    boost::json::string js(s);
    return js;
  }
  if (o[key].is_string()) {
    return o[key].as_string();
  }
  throw std::runtime_error(fmt::format("{} is not string", key));
}

double get_double_from_json_object(boost::json::object o,
                                   const std::string& key) {
  if (o[key].is_number()) {
    boost::json::error_code ec;
    auto value = o[key].to_number<double>(ec);
    if (ec) {
      throw std::runtime_error(
          fmt::format("{} to_number<double>() failed: {}", key, ec.message()));
    }
    return value;
  }
  throw std::runtime_error(fmt::format("{} is not number", key));
}

double get_double_from_json_object_with_default(boost::json::object o,
                                                const std::string& key,
                                                const double d) {
  if (!o.contains(key) || o[key].is_null()) {
    return d;
  }
  if (o[key].is_number()) {
    boost::json::error_code ec;
    auto value = o[key].to_number<double>(ec);
    if (ec) {
      throw std::runtime_error(
          fmt::format("{} to_number<double>() failed: {}", key, ec.message()));
    }
    return value;
  }
  throw std::runtime_error(fmt::format("{} is not number", key));
}

bool get_bool_from_json_object(boost::json::object o, const std::string& key) {
  if (o[key].is_bool()) {
    return o[key].as_bool();
  }
  throw std::runtime_error(fmt::format("{} is not bool", key));
}

bool get_bool_from_json_object_with_default(boost::json::object o,
                                            const std::string& key,
                                            const bool b) {
  if (!o.contains(key) || o[key].is_null()) {
    return b;
  }
  if (o[key].is_bool()) {
    return o[key].as_bool();
  }
  throw std::runtime_error(fmt::format("{} is not bool", key));
}

boost::json::array get_array_from_json_object_with_default(
    boost::json::object o,
    const std::string& key,
    const boost::json::array& a) {
  if (!o.contains(key) || o[key].is_null()) {
    return a;
  }
  if (o[key].is_array()) {
    return o[key].as_array();
  }
  throw std::runtime_error(fmt::format("{} is not array", key));
}

}  // namespace hisui::util
