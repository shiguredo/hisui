#pragma once

#include <cstdint>
#include <string>

#include <boost/json/array.hpp>
#include <boost/json/object.hpp>
#include <boost/json/string.hpp>
#include <boost/json/value.hpp>

namespace hisui::util {

boost::json::string get_string_from_json_object(boost::json::object o,
                                                const std::string& key);
boost::json::string get_string_from_json_object_with_default(
    boost::json::object o,
    const std::string& key,
    const std::string& d);

double get_double_from_json_object(boost::json::object o,
                                   const std::string& key);

double get_double_from_json_object_with_default(boost::json::object o,
                                                const std::string& key,
                                                const double);

bool get_bool_from_json_object(boost::json::object o, const std::string& key);
bool get_bool_from_json_object_with_default(boost::json::object o,
                                            const std::string& key,
                                            const bool);

boost::json::array get_array_from_json_object_with_default(
    boost::json::object o,
    const std::string& key,
    const boost::json::array&);
}  // namespace hisui::util
