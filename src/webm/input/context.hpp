#pragma once

#include <cstddef>
#include <cstdint>
#include <cstdio>
#include <string>

namespace mkvparser {

class Block;
class BlockEntry;
class Cluster;
class MkvReader;
class Segment;

}  // namespace mkvparser

namespace hisui::webm::input {

class Context {
 public:
  explicit Context(const std::string&);
  virtual ~Context();

  virtual bool init() = 0;
  std::size_t getBufferSize() const;
  unsigned char* getBuffer();
  std::string getFilePath() const;
  std::int64_t getTimestamp() const;
  std::int64_t getDuration() const;
  bool readFrame();

 protected:
  mkvparser::Segment* m_segment = nullptr;
  const mkvparser::Cluster* m_cluster = nullptr;
  int m_track_index = 0;
  std::string m_file_path;
  std::FILE* m_file = nullptr;

  void reset();
  bool initReaderAndSegment(std::FILE*);
  bool moveNextBlock();
  void rewindCluster();

 private:
  mkvparser::MkvReader* m_reader = nullptr;
  unsigned char* m_buffer = nullptr;
  const mkvparser::BlockEntry* m_block_entry = nullptr;
  const mkvparser::Block* m_block = nullptr;
  int m_block_frame_index = 0;
  bool m_reached_eos = false;
  std::size_t m_buffer_size = 0;
  std::int64_t m_timestamp_ns = 0;
  bool m_is_key_frame = false;
};

}  // namespace hisui::webm::input
