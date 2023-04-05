#include "webm/input/context.hpp"

#include <fmt/core.h>
#include <mkvparser/mkvparser.h>
#include <mkvparser/mkvreader.h>

#include <stdexcept>

namespace hisui::webm::input {

void Context::reset() {
  if (m_reader != nullptr) {
    delete m_reader;
  }
  if (m_segment != nullptr) {
    delete m_segment;
  }
  if (m_buffer != nullptr) {
    delete[] m_buffer;
  }
  m_reader = nullptr;
  m_segment = nullptr;
  m_buffer = nullptr;
  m_buffer_size = 0;
  m_cluster = nullptr;
  m_block_entry = nullptr;
  m_block = nullptr;
  m_block_frame_index = 0;
  m_reached_eos = false;
  m_timestamp_ns = 0;
  m_track_index = 0;
  m_is_key_frame = false;
  if (m_file) {
    std::fclose(m_file);
    m_file = nullptr;
  }
}

void Context::initReaderAndSegment(std::FILE* file) {
  m_reader = new mkvparser::MkvReader(file);
  m_reached_eos = false;

  mkvparser::EBMLHeader header;
  long long pos = 0; /* NOLINT */
  const auto parse_ret = header.Parse(m_reader, pos);
  if (parse_ret < 0) {
    reset();
    throw std::runtime_error(
        fmt::format("WebM header.Parse() failed: error_code={}", parse_ret));
  }

  const auto create_instance_ret =
      mkvparser::Segment::CreateInstance(m_reader, pos, m_segment);
  if (create_instance_ret != 0) {
    reset();
    throw std::runtime_error(fmt::format(
        "WebM mkvparser::Segment::CreateInstance() failed: error_code={}",
        create_instance_ret));
  }
  const auto segument_load_ret = m_segment->Load();
  if (segument_load_ret < 0) {
    reset();
    throw std::runtime_error(fmt::format(
        "WebM m_segment->Load() failed: error_code={}", segument_load_ret));
  }
}

bool Context::moveNextBlock() {
  if (m_cluster == nullptr) {
    throw std::runtime_error("m_cluster is null. should be initialized");
  }

  bool block_entry_eos = false;
  do {
    std::int64_t status = 0;
    bool get_new_block = false;
    if (m_block_entry == nullptr && !block_entry_eos) {
      status = m_cluster->GetFirst(m_block_entry);
      get_new_block = true;
    } else if (block_entry_eos || m_block_entry->EOS()) {
      m_cluster = m_segment->GetNext(m_cluster);
      if (m_cluster == nullptr || m_cluster->EOS()) {
        m_buffer_size = 0;
        m_reached_eos = true;
        return false;
      }
      status = m_cluster->GetFirst(m_block_entry);
      block_entry_eos = false;
      get_new_block = true;
    } else if (m_block == nullptr ||
               m_block_frame_index == m_block->GetFrameCount() ||
               m_block->GetTrackNumber() != m_track_index) {
      status = m_cluster->GetNext(m_block_entry, m_block_entry);
      if (m_block_entry == nullptr || m_block_entry->EOS()) {
        block_entry_eos = true;
        continue;
      }
      get_new_block = true;
    }
    if (status || m_block_entry == nullptr) {
      throw std::runtime_error("cannot get BlockEntry");
    }
    if (get_new_block) {
      m_block = m_block_entry->GetBlock();
      if (m_block == nullptr) {
        throw std::runtime_error("cannot get Block");
      }
      m_block_frame_index = 0;
    }
  } while (block_entry_eos || m_block->GetTrackNumber() != m_track_index);
  return true;
}

void Context::rewindCluster() {
  m_cluster = m_segment->GetFirst();
  m_block = nullptr;
  m_block_entry = nullptr;
  m_block_frame_index = 0;
  m_reached_eos = false;
}

Context::Context(const std::string& t_file_path) : m_file_path(t_file_path) {}

Context::~Context() {
  reset();
}

bool Context::readFrame() {
  // This check is needed for frame parallel decoding, in which case this
  // function could be called even after it has reached end of input stream.
  if (m_reached_eos) {
    return false;
  }

  if (!moveNextBlock()) {
    return false;
  }

  const mkvparser::Block::Frame& frame = m_block->GetFrame(m_block_frame_index);
  ++m_block_frame_index;
  std::size_t frame_len = static_cast<std::size_t>(frame.len);
  if (frame_len > m_buffer_size) {
    if (m_buffer != nullptr) {
      delete[] m_buffer;
    }
    m_buffer = new unsigned char[frame_len];
  }
  m_buffer_size = frame_len;
  m_timestamp_ns = m_block->GetTime(m_cluster);
  m_is_key_frame = m_block->IsKey();

  const auto status = frame.Read(m_reader, m_buffer);
  if (status != 0) {
    throw std::runtime_error(
        fmt::format("Frame::Read() failed: status={}", status));
  }
  return true;
}

std::size_t Context::getBufferSize() const {
  return m_buffer_size;
}

unsigned char* Context::getBuffer() {
  return m_buffer;
}

std::int64_t Context::getTimestamp() const {
  return m_timestamp_ns;
}

std::int64_t Context::getDuration() const {
  return m_segment->GetDuration();
}

std::string Context::getFilePath() const {
  return m_file_path;
}

};  // namespace hisui::webm::input
