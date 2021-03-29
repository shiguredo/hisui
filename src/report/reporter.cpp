#include "report/reporter.hpp"

#include <spdlog/spdlog.h>

#include <string>

#include "boost/json/serialize.hpp"
#include "boost/json/value_from.hpp"
#include "version/version.hpp"

namespace hisui::report {

std::string Reporter::makeSuccessReport() {
  boost::json::object inputs;
  for (const auto& [path, vdi] : m_video_decoder_map) {
    inputs[path] = {
        {"video_decoder_info", boost::json::value_from(vdi)},
        {"video_resolution_changes",
         boost::json::value_from(m_resolution_changes_map[path])},
    };
  }

  report["inputs"] = inputs;

  collectVersions();

  return boost::json::serialize(report);
}  // namespace hisui::report

void Reporter::collectVersions() {
  report["versions"] = {
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

void Reporter::registerVideoDecoder(const std::string& filename,
                                    const VideoDecoderInfo& vdi) {
  m_video_decoder_map.insert({filename, vdi});
}

void tag_invoke(const boost::json::value_from_tag&,
                boost::json::value& jv,  // NOLINT
                const VideoDecoderInfo& vdi) {
  jv = {
      {"codec", vdi.codec},
      {"duration", vdi.duration},
  };
}

void tag_invoke(const boost::json::value_from_tag&,
                boost::json::value& jv,  // NOLINT
                const ResolutionWithTimestamp& rwt) {
  jv = {
      {"timestamp", rwt.timestamp},
      {"width", rwt.width},
      {"height", rwt.height},
  };
}

}  // namespace hisui::report

