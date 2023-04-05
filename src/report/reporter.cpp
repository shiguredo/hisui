#include "report/reporter.hpp"

#include <fmt/core.h>
#include <spdlog/spdlog.h>
#include <sys/time.h>

#include <string>
#include <type_traits>
#include <utility>

#include <boost/json/impl/array.hpp>
#include <boost/json/object.hpp>
#include <boost/json/serialize.hpp>
#include <boost/json/value.hpp>
#include <boost/json/value_from.hpp>

#include "constants.hpp"
#include "version/version.hpp"

namespace {

std::string second_to_string(double second) {
  return fmt::format("{:.2f}s", second);
}

}  // namespace

namespace hisui::report {

bool operator==(ResolutionWithTimestamp const& left,
                ResolutionWithTimestamp const& right) {
  return left.timestamp == right.timestamp && left.width == right.width &&
         left.height == right.height;
}

Reporter::Reporter() {
  m_start_clock = std::clock();
}

std::string Reporter::makeSuccessReport() {
  return makeReport();
}

std::string Reporter::makeFailureReport(const std::string& error) {
  m_report["error"] = error;
  return makeReport();
}

std::string Reporter::makeReport() {
  boost::json::object inputs;
  for (const auto& [path, adi] : m_audio_decoder_map) {
    if (m_video_decoder_map.contains(path)) {
      auto v = m_resolution_changes_map.at(path);
      auto iter = std::unique(std::begin(v), std::end(v));
      v.erase(iter, std::end(v));
      inputs[path] = {
          {"audio_decoder_info", boost::json::value_from(adi)},
          {"video_decoder_info",
           boost::json::value_from(m_video_decoder_map.at(path))},
          {"video_resolution_changes", boost::json::value_from(v)},
      };
    } else {
      inputs[path] = {
          {"audio_decoder_info", boost::json::value_from(adi)},
      };
    }
  }

  for (const auto& [path, vdi] : m_video_decoder_map) {
    if (!m_audio_decoder_map.contains(path)) {
      auto v = m_resolution_changes_map.at(path);
      auto iter = std::unique(std::begin(v), std::end(v));
      v.erase(iter, std::end(v));
      inputs[path] = {
          {"video_decoder_info",
           boost::json::value_from(m_video_decoder_map.at(path))},
          {"video_resolution_changes", boost::json::value_from(v)},
      };
    }
  }

  m_report["inputs"] = inputs;
  m_report["output"] = boost::json::value_from(m_output_info);
  m_report["execution_time"] = second_to_string(
      static_cast<double>(std::clock() - m_start_clock) / CLOCKS_PER_SEC);
  collectVersions();

  return boost::json::serialize(m_report);
}  // namespace hisui::report

void Reporter::collectVersions() {
  m_report["versions"] = {
      {"libvpx", version::get_libvpx_version()},
      {"libwebm", version::get_libwebm_version()},
      {"openh264", version::get_openh264_version()},
#ifdef USE_FDK_AAC
      {"fdk-aac AACENC", version::get_fdkaac_aacenc_version()},
#endif
      {"hisui", version::get_hisui_version()},
      {"cpp-mp4", version::get_cppmp4_version()},
  };
}

void Reporter::open() {
  if (!m_reporter) {
    m_reporter = new Reporter();
  }
}

bool Reporter::hasInstance() {
  return m_reporter != nullptr;
}

Reporter& Reporter::getInstance() {
  return *m_reporter;
}

void Reporter::close() {
  delete m_reporter;
  m_reporter = nullptr;
}

void Reporter::registerResolutionChange(const std::string& filename,
                                        const ResolutionWithTimestamp& rwt) {
  m_resolution_changes_map[filename].push_back(rwt);
}

void Reporter::registerAudioDecoder(const std::string& filename,
                                    const AudioDecoderInfo& adi) {
  m_audio_decoder_map.insert({filename, adi});
}

void Reporter::registerVideoDecoder(const std::string& filename,
                                    const VideoDecoderInfo& vdi) {
  m_video_decoder_map.insert({filename, vdi});
}

void Reporter::registerOutput(const OutputInfo& output_info) {
  m_output_info = output_info;
}

void tag_invoke(const boost::json::value_from_tag&,
                boost::json::value& jv,  // NOLINT
                const AudioDecoderInfo& adi) {
  jv = {
      {"codec", adi.codec},
      {"channels", adi.channels},
      {"duration", second_to_string(static_cast<double>(adi.duration) /
                                    Constants::NANO_SECOND)},
  };
}

void tag_invoke(const boost::json::value_from_tag&,
                boost::json::value& jv,  // NOLINT
                const VideoDecoderInfo& vdi) {
  jv = {
      {"codec", vdi.codec},
      {"duration", second_to_string(static_cast<double>(vdi.duration) /
                                    Constants::NANO_SECOND)},
  };
}

void tag_invoke(const boost::json::value_from_tag&,
                boost::json::value& jv,  // NOLINT
                const ResolutionWithTimestamp& rwt) {
  jv = {
      {"timestamp", second_to_string(static_cast<double>(rwt.timestamp) /
                                     Constants::NANO_SECOND)},
      {"width", rwt.width},
      {"height", rwt.height},
  };
}

void tag_invoke(const boost::json::value_from_tag&,
                boost::json::value& jv,  // NOLINT
                const OutputInfo& oi) {
  jv = {
      {"container", oi.container},
      {"mux_type", oi.mux_type},
      {"video_codec", oi.video_codec},
      {"audio_codec", oi.audio_codec},
      {"duration", second_to_string(oi.duration)},
  };
}

}  // namespace hisui::report
