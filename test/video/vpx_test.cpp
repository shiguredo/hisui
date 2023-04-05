#include <vpx/vpx_image.h>

#include <boost/test/unit_test.hpp>

#include "video/vpx.hpp"
#include "video/yuv.hpp"

BOOST_AUTO_TEST_SUITE(vpx)

BOOST_AUTO_TEST_CASE(update_yuv_image_by_vpx_image_1) {
  auto yuv_image = std::make_shared<hisui::video::YUVImage>(4, 2);
  auto vpx_image = ::vpx_img_alloc(nullptr, VPX_IMG_FMT_I420, 4, 2, 0);
  unsigned char buf0[] = {0, 0, 0, 0, 1, 1, 1, 1};
  unsigned char buf1[] = {2, 2};
  unsigned char buf2[] = {3, 3};
  vpx_image->planes[0] = buf0;
  vpx_image->planes[1] = buf1;
  vpx_image->planes[2] = buf2;

  update_yuv_image_by_vpx_image(yuv_image, vpx_image);

  BOOST_REQUIRE_EQUAL(yuv_image->getWidth(0), 4);
  BOOST_REQUIRE_EQUAL(yuv_image->getHeight(0), 2);
  BOOST_REQUIRE_EQUAL_COLLECTIONS(yuv_image->yuv[0], yuv_image->yuv[0] + 8,
                                  buf0, buf0 + 8);
  BOOST_REQUIRE_EQUAL_COLLECTIONS(yuv_image->yuv[1], yuv_image->yuv[1] + 2,
                                  buf1, buf1 + 2);
  BOOST_REQUIRE_EQUAL_COLLECTIONS(yuv_image->yuv[2], yuv_image->yuv[2] + 2,
                                  buf2, buf2 + 2);
}

BOOST_AUTO_TEST_CASE(update_yuv_image_by_vpx_image_2) {
  auto yuv_image = std::make_shared<hisui::video::YUVImage>(6, 2);
  auto vpx_image = ::vpx_img_alloc(nullptr, VPX_IMG_FMT_I420, 4, 2, 0);
  unsigned char buf0[] = {0, 0, 0, 0, 1, 1, 1, 1};
  unsigned char buf1[] = {2, 2};
  unsigned char buf2[] = {3, 3};
  vpx_image->planes[0] = buf0;
  vpx_image->planes[1] = buf1;
  vpx_image->planes[2] = buf2;

  update_yuv_image_by_vpx_image(yuv_image, vpx_image);

  BOOST_REQUIRE_EQUAL(yuv_image->getWidth(0), 4);
  BOOST_REQUIRE_EQUAL(yuv_image->getHeight(0), 2);
  BOOST_REQUIRE_EQUAL_COLLECTIONS(yuv_image->yuv[0], yuv_image->yuv[0] + 8,
                                  buf0, buf0 + 8);
  BOOST_REQUIRE_EQUAL_COLLECTIONS(yuv_image->yuv[1], yuv_image->yuv[1] + 2,
                                  buf1, buf1 + 2);
  BOOST_REQUIRE_EQUAL_COLLECTIONS(yuv_image->yuv[2], yuv_image->yuv[2] + 2,
                                  buf2, buf2 + 2);
}

BOOST_AUTO_TEST_CASE(update_yuv_image_by_vpx_image_3) {
  auto yuv_image = std::make_shared<hisui::video::YUVImage>(6, 2);
  auto vpx_image = ::vpx_img_alloc(nullptr, VPX_IMG_FMT_I420, 4, 2, 8);
  unsigned char buf0[] = {0, 0, 0, 0, 100, 100, 100, 100,
                          1, 1, 1, 1, 100, 100, 100, 100};
  unsigned char buf1[] = {2, 2, 100};
  unsigned char buf2[] = {3, 3, 100};
  vpx_image->planes[0] = buf0;
  vpx_image->planes[1] = buf1;
  vpx_image->planes[2] = buf2;

  update_yuv_image_by_vpx_image(yuv_image, vpx_image);

  BOOST_REQUIRE_EQUAL(yuv_image->getWidth(0), 4);
  BOOST_REQUIRE_EQUAL(yuv_image->getHeight(0), 2);
  BOOST_REQUIRE_EQUAL_COLLECTIONS(yuv_image->yuv[0], yuv_image->yuv[0] + 4,
                                  buf0, buf0 + 4);
  BOOST_REQUIRE_EQUAL_COLLECTIONS(yuv_image->yuv[0] + 4, yuv_image->yuv[0] + 8,
                                  buf0 + 8, buf0 + 12);
  BOOST_REQUIRE_EQUAL_COLLECTIONS(yuv_image->yuv[1], yuv_image->yuv[1] + 2,
                                  buf1, buf1 + 2);
  BOOST_REQUIRE_EQUAL_COLLECTIONS(yuv_image->yuv[2], yuv_image->yuv[2] + 2,
                                  buf2, buf2 + 2);
}

BOOST_AUTO_TEST_SUITE_END()
