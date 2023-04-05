#include <boost/test/unit_test.hpp>

#include <boost/json.hpp>

#include "metadata.hpp"

BOOST_AUTO_TEST_SUITE(time_offset)

BOOST_AUTO_TEST_CASE(archive_adjust_time_offsets) {
  hisui::ArchiveItem archive("dummy", "connection_id", 0, 10);
  archive.adjustTimeOffsets(1.5);
  BOOST_REQUIRE_CLOSE(1.5, archive.getStartTimeOffset(), 0.00001);
  BOOST_REQUIRE_CLOSE(11.5, archive.getStopTimeOffset(), 0.00001);
  archive.adjustTimeOffsets(-0.5);
  BOOST_REQUIRE_CLOSE(1.0, archive.getStartTimeOffset(), 0.00001);
  BOOST_REQUIRE_CLOSE(11.0, archive.getStopTimeOffset(), 0.00001);
}

BOOST_AUTO_TEST_CASE(metadata_adjust_time_offsets) {
  hisui::Metadata metadata({
      {"dummy", "connection_id", 0, 10},
      {"dummy", "connection_id", 10, 30},
      {"dummy", "connection_id", 15, 20},
  });
  BOOST_REQUIRE_EQUAL(0, metadata.getMinStartTimeOffset());
  BOOST_REQUIRE_EQUAL(30, metadata.getMaxStopTimeOffset());
  metadata.adjustTimeOffsets(1.5);
  BOOST_REQUIRE_CLOSE(1.5, metadata.getMinStartTimeOffset(), 0.00001);
  BOOST_REQUIRE_CLOSE(31.5, metadata.getMaxStopTimeOffset(), 0.00001);
  {
    auto archives = metadata.getArchiveItems();
    BOOST_REQUIRE_CLOSE(1.5, archives[0].getStartTimeOffset(), 0.00001);
    BOOST_REQUIRE_CLOSE(11.5, archives[0].getStopTimeOffset(), 0.00001);
    BOOST_REQUIRE_CLOSE(11.5, archives[1].getStartTimeOffset(), 0.00001);
    BOOST_REQUIRE_CLOSE(31.5, archives[1].getStopTimeOffset(), 0.00001);
    BOOST_REQUIRE_CLOSE(16.5, archives[2].getStartTimeOffset(), 0.00001);
    BOOST_REQUIRE_CLOSE(21.5, archives[2].getStopTimeOffset(), 0.00001);
  }
  metadata.adjustTimeOffsets(-1.0);
  BOOST_REQUIRE_CLOSE(0.5, metadata.getMinStartTimeOffset(), 0.00001);
  BOOST_REQUIRE_CLOSE(30.5, metadata.getMaxStopTimeOffset(), 0.00001);
  {
    auto archives = metadata.getArchiveItems();
    BOOST_REQUIRE_CLOSE(0.5, archives[0].getStartTimeOffset(), 0.00001);
    BOOST_REQUIRE_CLOSE(10.5, archives[0].getStopTimeOffset(), 0.00001);
    BOOST_REQUIRE_CLOSE(10.5, archives[1].getStartTimeOffset(), 0.00001);
    BOOST_REQUIRE_CLOSE(30.5, archives[1].getStopTimeOffset(), 0.00001);
    BOOST_REQUIRE_CLOSE(15.5, archives[2].getStartTimeOffset(), 0.00001);
    BOOST_REQUIRE_CLOSE(20.5, archives[2].getStopTimeOffset(), 0.00001);
  }
}

BOOST_AUTO_TEST_SUITE_END()
